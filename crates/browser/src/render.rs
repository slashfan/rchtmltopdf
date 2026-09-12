//! Deciding when a page is finished.
//!
//! There is no single signal for "ready to print", so D07 stacks four, in this
//! order, and the order is the whole design:
//!
//! 1. the load event
//! 2. the network going quiet
//! 3. web fonts being ready
//! 4. a fixed delay, or a nominated `window.status`
//!
//! Each rung exists because the ones around it are not enough on their own. The
//! load event fires before late scripts have fetched anything. Waiting only for
//! the network never finishes on a page that long-polls or sends analytics
//! beacons, which is why there is a deadline over the top rather than a fifth
//! rung. And font readiness has to come *after* the network is quiet: it
//! resolves immediately when no font is used yet, and never at all when a web
//! font is still being fetched by a stylesheet that itself has not arrived.
//!
//! The current rung is published as it goes, so a conversion that gets stuck can
//! say where rather than leaving it to be guessed.
//!
//! # Two things happen around the ladder, and where matters
//!
//! **The user stylesheet goes in after the load event and before rung 2.** It has
//! to be after, because there is no `document.head` to put it in until the
//! document exists. It has to be before rung 2 so a stylesheet named by URL is
//! still counted as in flight, and before rung 3 because a stylesheet that
//! declares a font face makes the wait for fonts meaningless if it arrives
//! afterwards: the wait resolves against the fonts the page already had, and the
//! one being introduced is still being fetched when the page is printed.
//!
//! **`--run-script` runs after the whole ladder.** It is for a page that has
//! finished arriving; a script that ran before the network went quiet would see
//! a different document on every run.

use crate::cdp::{Event, Session};
use crate::error::Error;
use crate::error::Result;
use crate::launch::Page;
use crate::plan::{Command, Injection, LoadPlan, Settle};
use rchtmltopdf_core::settings::LoadSettings;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the network has to stay quiet to count as idle.
const QUIET_PERIOD: Duration = Duration::from_millis(500);

/// How often to look at `window.status`.
const STATUS_POLL: Duration = Duration::from_millis(50);

/// The only events that say anything about whether the network is busy.
///
/// Subscribing to exactly these matters twice over. A subscription that took
/// everything would have its quiet period reset by unrelated chatter, so a page
/// emitting any event on a timer would never settle. And it would queue every
/// event on the session from before navigation until the delay ends, unread.
const TRAFFIC_EVENTS: &[&str] = &[
    "Network.requestWillBeSent",
    "Network.loadingFinished",
    "Network.loadingFailed",
];

/// Where a page has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    #[default]
    Navigating,
    AwaitingLoad,
    Injecting,
    AwaitingNetworkIdle,
    AwaitingFonts,
    AwaitingWindowStatus,
    Delaying,
    RunningScripts,
    Settled,
}

impl Stage {
    /// Phrased to drop into "timed out while …".
    pub fn describe(self) -> &'static str {
        match self {
            Stage::Navigating => "opening the document",
            Stage::AwaitingLoad => "waiting for the page to load",
            Stage::Injecting => "putting the user stylesheet into the document",
            Stage::AwaitingNetworkIdle => "waiting for the network to go idle",
            Stage::AwaitingFonts => "waiting for web fonts",
            Stage::AwaitingWindowStatus => "waiting for window.status",
            Stage::Delaying => "waiting out the JavaScript delay",
            Stage::RunningScripts => "running --run-script",
            Stage::Settled => "finishing",
        }
    }
}

/// The rung a page is on, readable from elsewhere while it is still climbing.
///
/// Exists so the conversion deadline can say what it interrupted. A trace
/// collected at the end would be no help for a run that never ends, which is
/// exactly the case worth diagnosing.
#[derive(Debug, Clone, Default)]
pub struct Progress(Arc<Mutex<Stage>>);

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Stage {
        *self.0.lock().unwrap()
    }

    fn enter(&self, stage: Stage) {
        *self.0.lock().unwrap() = stage;
    }

    /// Move the rung along from outside, so a deadline can be tested against a
    /// known rung without driving a real page there.
    #[doc(hidden)]
    pub fn enter_for_test(&self, stage: Stage) {
        self.enter(stage);
    }
}

impl Page {
    /// Get the page ready to be loaded into.
    ///
    /// Separate from navigating because these have to be in place before the
    /// document arrives, not after: media queries decide which resources are
    /// fetched at all.
    ///
    /// Takes the commands the plan decided rather than the settings they were
    /// decided from. There is nothing left here to decide, which is the point:
    /// a choice that never reached the plan cannot be made on the way out (D27).
    pub async fn prepare(&self, commands: &[Command]) -> Result<()> {
        for command in commands {
            self.session()
                .send(command.method, command.params.clone())
                .await?;
        }
        Ok(())
    }

    /// Open a document and wait until it has stopped changing.
    ///
    /// Not bounded from the inside. A page that long-polls never settles by
    /// design, so the bound belongs to the conversion deadline above this
    /// (D16), which can then say which rung it interrupted.
    pub async fn load(&self, url: &str, load: &LoadSettings, progress: &Progress) -> Result<()> {
        let session = self.session();
        let ladder = LoadPlan::new(load);

        // Subscribe before navigating. The load event for a small document can
        // arrive before the navigate call has even returned.
        let mut loaded = session.subscribe("Page.loadEventFired");
        let mut traffic = session.subscribe_many(TRAFFIC_EVENTS);

        progress.enter(Stage::Navigating);
        let outcome = session.send("Page.navigate", json!({ "url": url })).await?;

        // A navigation that fails still answers, and the failure is a field in
        // the reply rather than a protocol error. Ignoring it means a dead
        // address loads the browser's own error page and reports success, which
        // would then be printed. D14 says that is exit 1 and no PDF.
        if let Some(reason) = outcome.get("errorText").and_then(Value::as_str) {
            return Err(crate::error::Error::Navigation {
                url: url.to_string(),
                reason: reason.to_string(),
            });
        }

        progress.enter(Stage::AwaitingLoad);
        if loaded.next().await.is_none() {
            return Err(crate::error::Error::ConnectionClosed);
        }

        // Before the network is judged idle, so a stylesheet named by URL is
        // still counted as in flight, and before the wait for fonts, so one that
        // declares a font face is waited for rather than raced.
        if let Some(injection) = &ladder.inject {
            progress.enter(Stage::Injecting);
            inject(session, injection).await?;
        }

        progress.enter(Stage::AwaitingNetworkIdle);
        let document = wait_for_quiet_network(&mut traffic).await;
        if let Some(reason) = document.failure {
            return Err(crate::error::Error::Navigation {
                url: url.to_string(),
                reason,
            });
        }

        progress.enter(Stage::AwaitingFonts);
        wait_for_fonts(session).await;

        // The last rung is the only one there is a choice about, and the choice
        // was made in the plan rather than here.
        match &ladder.settle {
            Settle::WindowStatus(wanted) => {
                progress.enter(Stage::AwaitingWindowStatus);
                wait_for_window_status(session, wanted).await;
            }
            Settle::Delay(delay) => {
                progress.enter(Stage::Delaying);
                tokio::time::sleep(*delay).await;
            }
        }

        // After the ladder, not on it: `--run-script` is for a page that has
        // finished arriving, and a script that ran before the network went quiet
        // would see a different document each time.
        if !ladder.scripts.is_empty() {
            progress.enter(Stage::RunningScripts);
            for source in &ladder.scripts {
                run_script(session, source).await?;
            }
        }

        progress.enter(Stage::Settled);
        Ok(())
    }
}

/// Put the user stylesheet into the document.
///
/// A file is read here and inlined rather than left to the browser to fetch:
/// the user named it on the command line, so the policy governing what the
/// *document* may read off the disk has nothing to say about it (D10).
///
/// It goes at the end of the head, which is where wkhtmltopdf's own help says it
/// goes. That is not quite what Qt did — a Qt user stylesheet was a separate
/// cascade origin and beat author rules outright, and an injected `<style>` is
/// an author rule that beats only the ones before it. A document whose own rules
/// carry `!important` still wins, and no protocol command offers the other
/// thing.
async fn inject(session: &Session, injection: &Injection) -> Result<()> {
    let expression = match injection {
        Injection::File(path) => {
            let css = std::fs::read_to_string(path).map_err(|error| Error::StyleSheet {
                path: path.clone(),
                reason: error.to_string(),
            })?;
            format!(
                "(() => {{ const sheet = document.createElement('style'); \
                 sheet.textContent = {}; document.head.appendChild(sheet); }})()",
                js_string(&css)
            )
        }
        Injection::Link(url) => format!(
            "(() => {{ const link = document.createElement('link'); \
             link.rel = 'stylesheet'; link.href = {}; document.head.appendChild(link); }})()",
            js_string(url)
        ),
    };

    if let Some(message) = evaluate(session, &expression).await? {
        return Err(Error::StyleSheet {
            path: match injection {
                Injection::File(path) => path.clone(),
                Injection::Link(url) => std::path::PathBuf::from(url),
            },
            reason: message,
        });
    }
    Ok(())
}

/// Run one `--run-script`, and fail the conversion if it throws.
///
/// Failing is the default error handling and, for now, the only one:
/// `--load-error-handling` is not wired up yet (#28). A script that throws and
/// is ignored leaves a document that renders and is missing whatever the script
/// was there to do, which is the outcome D14 exists to avoid.
async fn run_script(session: &Session, source: &str) -> Result<()> {
    match evaluate(session, source).await? {
        Some(message) => Err(Error::Script {
            source: source.to_string(),
            message,
        }),
        None => Ok(()),
    }
}

/// Evaluate an expression, awaiting a promise, and report what it threw.
///
/// A thrown exception is **not** a protocol error: the command succeeds and the
/// throw is a field in the reply. Ignoring that field is how a script that fails
/// every time looks like one that works.
async fn evaluate(session: &Session, expression: &str) -> Result<Option<String>> {
    let answer = session
        .send(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "awaitPromise": true,
                "returnByValue": true,
            }),
        )
        .await?;
    Ok(thrown(&answer))
}

/// What an evaluation threw, if it threw.
fn thrown(answer: &Value) -> Option<String> {
    let details = answer.get("exceptionDetails")?;
    let described = details
        .get("exception")
        .and_then(|exception| exception.get("description"))
        .and_then(Value::as_str);
    let text = details.get("text").and_then(Value::as_str);
    Some(described.or(text).unwrap_or("the script threw").to_string())
}

/// A Rust string as a JavaScript string literal.
///
/// JSON's string syntax is JavaScript's, so this is exactly the escaping needed
/// and is how a stylesheet containing a quote, a backslash or a newline reaches
/// the page as itself rather than as a syntax error.
fn js_string(raw: &str) -> String {
    serde_json::to_string(raw).unwrap_or_else(|_| "\"\"".to_string())
}

/// The main document's request, and whether it failed.
///
/// **`Page.navigate` does not report every failure.** It carries an `errorText`
/// for a host that does not resolve, and nothing at all for a proxy that refuses
/// the connection or for credentials the server would not accept — the call
/// succeeds and Chromium renders its own error page, which is then printed. D14
/// says that is exit 1 and no PDF, so the network events are watched instead.
///
/// Only the document. A subresource that fails still produces a PDF, which is
/// what `--load-media-error-handling` defaults to (#28 makes it a choice).
#[derive(Debug, Default)]
struct MainDocument {
    id: Option<String>,
    failure: Option<String>,
}

/// Wait until nothing has been in flight for a while.
///
/// Counts by request id rather than by a running total, so a reply that arrives
/// for a request we never counted cannot drive the total negative and declare
/// the page idle early.
async fn wait_for_quiet_network(events: &mut crate::cdp::Events) -> MainDocument {
    let mut in_flight: HashSet<String> = HashSet::new();
    let mut document = MainDocument::default();

    loop {
        let next = if in_flight.is_empty() {
            // Nothing outstanding: give the page a moment to start something
            // else before calling it quiet.
            match tokio::time::timeout(QUIET_PERIOD, events.next()).await {
                Err(_) => return document,
                Ok(event) => event,
            }
        } else {
            events.next().await
        };

        let Some(event) = next else {
            // The connection went away. Whatever happens next will report it
            // better than this loop can.
            return document;
        };
        apply_traffic(&event, &mut in_flight, &mut document);
    }
}

fn apply_traffic(event: &Event, in_flight: &mut HashSet<String>, document: &mut MainDocument) {
    let Some(id) = event.params.get("requestId").and_then(Value::as_str) else {
        return;
    };

    match event.method.as_str() {
        "Network.requestWillBeSent" => {
            // The first document-type request is the one being converted. A
            // redirect reuses the id, so following one keeps this pointing at
            // the navigation rather than at whatever it landed on.
            if document.id.is_none()
                && event.params.get("type").and_then(Value::as_str) == Some("Document")
            {
                document.id = Some(id.to_string());
            }

            // Inline data never touches the network, and counting it means
            // waiting for something that will never be reported as finished.
            let url = event
                .params
                .get("request")
                .and_then(|request| request.get("url"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if url.starts_with("data:") || url.starts_with("blob:") {
                return;
            }
            in_flight.insert(id.to_string());
        }
        "Network.loadingFinished" => {
            in_flight.remove(id);
        }
        "Network.loadingFailed" => {
            in_flight.remove(id);
            if document.id.as_deref() == Some(id) {
                document.failure = Some(
                    event
                        .params
                        .get("errorText")
                        .and_then(Value::as_str)
                        .unwrap_or("the request failed")
                        .to_string(),
                );
            }
        }
        _ => {}
    }
}

/// Wait for web fonts, if the page uses any.
///
/// Failures are deliberately swallowed. A page with scripting disabled, or one
/// that navigated again underneath us, cannot answer, and neither is a reason to
/// refuse to print. The cost of being wrong here is a fallback font, not a
/// failed conversion.
async fn wait_for_fonts(session: &Session) {
    let _ = session
        .send(
            "Runtime.evaluate",
            json!({
                "expression": "document.fonts ? document.fonts.ready.then(() => true) : true",
                "awaitPromise": true,
                "returnByValue": true,
            }),
        )
        .await;
}

/// Poll until `window.status` matches.
///
/// Polling rather than watching, because there is no event for it. Unbounded on
/// purpose: a status that never arrives is the deadline's business, and it can
/// say that this is what it was waiting for.
async fn wait_for_window_status(session: &Session, wanted: &str) {
    loop {
        let answer = session
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": "String(window.status)",
                    "returnByValue": true,
                }),
            )
            .await;

        match answer {
            Ok(value) => {
                if value["result"]["value"].as_str() == Some(wanted) {
                    return;
                }
            }
            // The page is gone; nothing to wait for.
            Err(_) => return,
        }

        tokio::time::sleep(STATUS_POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(method: &str, params: Value) -> Event {
        Event {
            method: method.into(),
            params,
            session_id: None,
        }
    }

    #[test]
    fn a_request_counts_until_it_finishes() {
        let mut in_flight = HashSet::new();
        apply_traffic(
            &event(
                "Network.requestWillBeSent",
                json!({ "requestId": "R1", "request": { "url": "https://example.com/a.css" } }),
            ),
            &mut in_flight,
            &mut MainDocument::default(),
        );
        assert_eq!(in_flight.len(), 1);

        apply_traffic(
            &event("Network.loadingFinished", json!({ "requestId": "R1" })),
            &mut in_flight,
            &mut MainDocument::default(),
        );
        assert!(in_flight.is_empty());
    }

    #[test]
    fn a_failed_request_also_stops_counting() {
        let mut in_flight = HashSet::new();
        apply_traffic(
            &event(
                "Network.requestWillBeSent",
                json!({ "requestId": "R1", "request": { "url": "https://example.com/gone.png" } }),
            ),
            &mut in_flight,
            &mut MainDocument::default(),
        );
        apply_traffic(
            &event("Network.loadingFailed", json!({ "requestId": "R1" })),
            &mut in_flight,
            &mut MainDocument::default(),
        );
        assert!(
            in_flight.is_empty(),
            "a 404 must not hold the page open for ever"
        );
    }

    /// Inline data is reported like a request but never reported as finished.
    /// Counting it means never going idle, which is the gotcha D07 names.
    #[test]
    fn inline_data_is_not_counted() {
        let mut in_flight = HashSet::new();
        for url in ["data:text/css,body{}", "blob:https://example.com/abc"] {
            apply_traffic(
                &event(
                    "Network.requestWillBeSent",
                    json!({ "requestId": "R1", "request": { "url": url } }),
                ),
                &mut in_flight,
                &mut MainDocument::default(),
            );
        }
        assert!(in_flight.is_empty());
    }

    /// Counting ids rather than keeping a running total means a stray completion
    /// cannot drive the count below zero and declare the page idle early.
    #[test]
    fn a_completion_for_something_never_started_is_harmless() {
        let mut in_flight = HashSet::new();
        apply_traffic(
            &event("Network.loadingFinished", json!({ "requestId": "ghost" })),
            &mut in_flight,
            &mut MainDocument::default(),
        );
        apply_traffic(
            &event(
                "Network.requestWillBeSent",
                json!({ "requestId": "R1", "request": { "url": "https://example.com/a" } }),
            ),
            &mut in_flight,
            &mut MainDocument::default(),
        );
        assert_eq!(in_flight.len(), 1, "the real request is still outstanding");
    }

    #[test]
    fn unrelated_events_are_ignored() {
        let mut in_flight = HashSet::new();
        apply_traffic(
            &event("Page.loadEventFired", json!({})),
            &mut in_flight,
            &mut MainDocument::default(),
        );
        apply_traffic(
            &event("Runtime.consoleAPICalled", json!({ "requestId": "R1" })),
            &mut in_flight,
            &mut MainDocument::default(),
        );
        assert!(in_flight.is_empty());
    }

    /// The document's own request is the one whose failure matters. A
    /// subresource that fails still produces a PDF (#28 makes that a choice).
    #[test]
    fn only_the_documents_own_failure_is_a_navigation_failure() {
        let mut in_flight = HashSet::new();
        let mut document = MainDocument::default();

        apply_traffic(
            &event(
                "Network.requestWillBeSent",
                json!({
                    "requestId": "DOC",
                    "type": "Document",
                    "request": { "url": "https://example.com/a" }
                }),
            ),
            &mut in_flight,
            &mut document,
        );
        apply_traffic(
            &event(
                "Network.requestWillBeSent",
                json!({
                    "requestId": "IMG",
                    "type": "Image",
                    "request": { "url": "https://example.com/a.png" }
                }),
            ),
            &mut in_flight,
            &mut document,
        );

        // An image that 404s is not a failed conversion.
        apply_traffic(
            &event(
                "Network.loadingFailed",
                json!({ "requestId": "IMG", "errorText": "net::ERR_FAILED" }),
            ),
            &mut in_flight,
            &mut document,
        );
        assert!(document.failure.is_none());

        // The document is.
        apply_traffic(
            &event(
                "Network.loadingFailed",
                json!({ "requestId": "DOC", "errorText": "net::ERR_PROXY_CONNECTION_FAILED" }),
            ),
            &mut in_flight,
            &mut document,
        );
        assert_eq!(
            document.failure.as_deref(),
            Some("net::ERR_PROXY_CONNECTION_FAILED")
        );
    }

    /// A redirect reuses the request id, so the first document-type request
    /// stays the one being watched rather than whatever it landed on.
    #[test]
    fn a_later_document_request_does_not_steal_the_first() {
        let mut in_flight = HashSet::new();
        let mut document = MainDocument::default();
        for id in ["FIRST", "IFRAME"] {
            apply_traffic(
                &event(
                    "Network.requestWillBeSent",
                    json!({
                        "requestId": id,
                        "type": "Document",
                        "request": { "url": "https://example.com/a" }
                    }),
                ),
                &mut in_flight,
                &mut document,
            );
        }
        assert_eq!(document.id.as_deref(), Some("FIRST"));
    }

    #[test]
    fn every_stage_can_be_described() {
        for stage in [
            Stage::Navigating,
            Stage::AwaitingLoad,
            Stage::Injecting,
            Stage::AwaitingNetworkIdle,
            Stage::AwaitingFonts,
            Stage::AwaitingWindowStatus,
            Stage::Delaying,
            Stage::RunningScripts,
            Stage::Settled,
        ] {
            assert!(!stage.describe().is_empty());
        }
        // Reads as part of a sentence the user will actually see.
        assert_eq!(
            format!("timed out while {}", Stage::AwaitingNetworkIdle.describe()),
            "timed out while waiting for the network to go idle"
        );
    }

    #[test]
    fn progress_starts_at_the_beginning_and_moves() {
        let progress = Progress::new();
        assert_eq!(progress.current(), Stage::Navigating);
        progress.enter(Stage::AwaitingFonts);
        assert_eq!(progress.current(), Stage::AwaitingFonts);
    }
}
