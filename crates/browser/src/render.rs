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

use crate::band::Measured;
use crate::cdp::{Event, Session};
use crate::error::Error;
use crate::error::Result;
use crate::launch::Page;
use crate::plan::{Command, Injection, LoadPlan, Settle};
use rchtmltopdf_core::NetworkError;
use rchtmltopdf_core::settings::LoadSettings;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
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
    // Not about whether the network is busy, but about whether what came back
    // was what was asked for: a 404 finishes loading like anything else.
    "Network.responseReceived",
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
    pub async fn load(
        &self,
        url: &str,
        load: &LoadSettings,
        progress: &Progress,
    ) -> Result<LoadReport> {
        let session = self.session();
        let ladder = LoadPlan::new(load);

        // Subscribe before navigating. The load event for a small document can
        // arrive before the navigate call has even returned.
        let mut loaded = session.subscribe("Page.loadEventFired");
        let mut traffic = session.subscribe_many(TRAFFIC_EVENTS);

        progress.enter(Stage::Navigating);
        let outcome = session.send("Page.navigate", json!({ "url": url })).await?;

        // A navigation that fails still answers, and the failure is a field in
        // the reply rather than a protocol error. **Reported, not raised**
        // (D44): the caller's `--load-error-handling` decides whether the
        // conversion ends here, carries on without this document, or leaves a
        // blank page where it would have been, and a failure raised from inside
        // this function never reached that choice. Nothing is waited for after
        // this: there was no navigation, so no load event is coming.
        if let Some(reason) = outcome.get("errorText").and_then(Value::as_str) {
            return Ok(LoadReport {
                navigation_failed: true,
                document: Some(Failed {
                    url: url.to_string(),
                    // Named the way wkhtmltopdf named it, not the way Chromium
                    // does. An application branching on `HostNotFoundError` is
                    // reading the stderr of a program it did not write (D14).
                    error: NetworkError::from_chromium(reason),
                    // The navigation never got a response to carry a status.
                    http_status: 0,
                }),
                media: Vec::new(),
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
        let report = wait_for_quiet_network(&mut traffic).await;

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
        Ok(report)
    }
}

impl Page {
    /// Load nothing, so that there is a page to print (D44).
    ///
    /// Where a document did not arrive at all and `--load-error-handling
    /// ignore` says to carry on regardless: wkhtmltopdf left a blank page
    /// where that document would have been and went on counting, so the page
    /// behind it keeps its number. `about:blank` printed with the print
    /// command the document would have been printed with is that page, at the
    /// same paper size and margins, and it is the only way to avoid printing
    /// whatever the browser left on screen instead.
    pub async fn blank(&self) -> Result<()> {
        let session = self.session();
        let mut loaded = session.subscribe("Page.loadEventFired");
        session
            .send("Page.navigate", json!({ "url": "about:blank" }))
            .await?;
        if loaded.next().await.is_none() {
            return Err(crate::error::Error::ConnectionClosed);
        }
        Ok(())
    }

    /// Measure the band document this page has loaded (D39).
    ///
    /// The body's height is what wkhtmltopdf reserved for a band document,
    /// and how far the body reaches is how tall the frame on the sheet has
    /// to be for none of it to be cut off. Both in CSS pixels from the page,
    /// converted here, so the sheet and the print call agree on the number.
    pub async fn measure_band(&self) -> Result<Measured> {
        const EXPRESSION: &str = "(() => { const body = document.body; \
             if (!body) { return [0, 0]; } \
             const box = body.getBoundingClientRect(); \
             return [box.height, box.bottom]; })()";
        let answer = self
            .session()
            .send(
                "Runtime.evaluate",
                json!({ "expression": EXPRESSION, "returnByValue": true }),
            )
            .await?;
        if let Some(message) = thrown(&answer) {
            return Err(Error::Script {
                source: "measuring the band document".to_string(),
                message,
            });
        }
        let value = |index: usize| {
            answer["result"]["value"][index]
                .as_f64()
                .unwrap_or(0.0)
                .max(0.0)
        };
        let to_mm = |pixels: f64| pixels / crate::plan::CSS_PIXELS_PER_INCH * 25.4;
        Ok(Measured {
            height_mm: to_mm(value(0)),
            extent_mm: to_mm(value(1)),
        })
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

/// What went wrong on the way, beyond the page being ready.
///
/// **`Page.navigate` does not report every failure.** It carries an `errorText`
/// for a host that does not resolve, and nothing at all for a proxy that refuses
/// the connection or for credentials the server would not accept — the call
/// succeeds and Chromium renders its own error page, which would then be
/// printed. And a 404 is not a failure to Chromium at all: the bytes came back
/// and an error page is a page. It was a failure to Qt, which is where
/// `ContentNotFoundError` comes from.
///
/// So the network events are watched, and what is found is **reported rather
/// than decided on**. Whether a failed subresource is worth an exit code is
/// `--load-media-error-handling`'s business and the command line layer's, not
/// this module's (D14).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// The document's own request, if it failed. A 404 is one of these: the
    /// bytes came back and there is a page to print.
    pub document: Option<Failed>,
    /// The navigation itself failed, so nothing arrived and there is no page
    /// to print — not even an error page (D44). `document` names the failure.
    pub navigation_failed: bool,
    /// Everything else that failed, once per URL and in the order it was asked
    /// for.
    pub media: Vec<Failed>,
}

/// One request that did not produce what was wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    pub url: String,
    pub error: NetworkError,
    /// The status the server answered with, or **0 when no response arrived**
    /// — a host that did not resolve, a connection nobody accepted, a file that
    /// is not there. wkhtmltopdf prints both this and [`NetworkError::code`] on
    /// the line applications grep for, and the pair is how they tell a server
    /// that refused from a server that was never reached.
    pub http_status: u16,
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.url, self.error)
    }
}

/// What the traffic watcher is keeping track of while the page loads.
#[derive(Debug, Default)]
struct Watch {
    /// The document's own request id: the first one Chromium calls a document.
    document: Option<String>,
    /// Every request id seen, so a failure can name the URL it was for.
    urls: HashMap<String, String>,
    report: LoadReport,
}

impl Watch {
    fn record(&mut self, id: &str, error: NetworkError, http_status: u16) {
        let url = self.urls.get(id).cloned().unwrap_or_default();

        if self.document.as_deref() == Some(id) {
            // The first failure is the one worth reporting: a redirect chain
            // that ends badly should name what went wrong, not what came after.
            self.report.document.get_or_insert(Failed {
                url,
                error,
                http_status,
            });
            return;
        }

        // Chromium asks for this on every navigation and wkhtmltopdf never did,
        // so a site without one would fail a conversion for a file the document
        // never mentioned.
        if url.ends_with("/favicon.ico") {
            return;
        }
        if self.report.media.iter().any(|failed| failed.url == url) {
            return;
        }
        self.report.media.push(Failed {
            url,
            error,
            http_status,
        });
    }
}

/// Wait until nothing has been in flight for a while.
///
/// Counts by request id rather than by a running total, so a reply that arrives
/// for a request we never counted cannot drive the total negative and declare
/// the page idle early.
async fn wait_for_quiet_network(events: &mut crate::cdp::Events) -> LoadReport {
    let mut in_flight: HashSet<String> = HashSet::new();
    let mut watch = Watch::default();

    loop {
        let next = if in_flight.is_empty() {
            // Nothing outstanding: give the page a moment to start something
            // else before calling it quiet.
            match tokio::time::timeout(QUIET_PERIOD, events.next()).await {
                Err(_) => return watch.report,
                Ok(event) => event,
            }
        } else {
            events.next().await
        };

        let Some(event) = next else {
            // The connection went away. Whatever happens next will report it
            // better than this loop can.
            return watch.report;
        };
        apply_traffic(&event, &mut in_flight, &mut watch);
    }
}

fn apply_traffic(event: &Event, in_flight: &mut HashSet<String>, watch: &mut Watch) {
    let Some(id) = event.params.get("requestId").and_then(Value::as_str) else {
        return;
    };

    match event.method.as_str() {
        "Network.requestWillBeSent" => {
            // The first document-type request is the one being converted. A
            // redirect reuses the id, so following one keeps this pointing at
            // the navigation rather than at whatever it landed on.
            if watch.document.is_none()
                && event.params.get("type").and_then(Value::as_str) == Some("Document")
            {
                watch.document = Some(id.to_string());
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
            watch.urls.insert(id.to_string(), url.to_string());
            in_flight.insert(id.to_string());
        }
        "Network.responseReceived" => {
            let status = event
                .params
                .get("response")
                .and_then(|response| response.get("status"))
                .and_then(Value::as_u64)
                .unwrap_or(200);
            if let Some(error) = NetworkError::from_status(status as u16) {
                watch.record(id, error, status as u16);
            }
        }
        "Network.loadingFinished" => {
            in_flight.remove(id);
        }
        "Network.loadingFailed" => {
            in_flight.remove(id);
            let text = event
                .params
                .get("errorText")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // Nothing answered, so there is no status to report beside it.
            watch.record(id, NetworkError::from_chromium(text), 0);
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
            &mut Watch::default(),
        );
        assert_eq!(in_flight.len(), 1);

        apply_traffic(
            &event("Network.loadingFinished", json!({ "requestId": "R1" })),
            &mut in_flight,
            &mut Watch::default(),
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
            &mut Watch::default(),
        );
        apply_traffic(
            &event("Network.loadingFailed", json!({ "requestId": "R1" })),
            &mut in_flight,
            &mut Watch::default(),
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
                &mut Watch::default(),
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
            &mut Watch::default(),
        );
        apply_traffic(
            &event(
                "Network.requestWillBeSent",
                json!({ "requestId": "R1", "request": { "url": "https://example.com/a" } }),
            ),
            &mut in_flight,
            &mut Watch::default(),
        );
        assert_eq!(in_flight.len(), 1, "the real request is still outstanding");
    }

    #[test]
    fn unrelated_events_are_ignored() {
        let mut in_flight = HashSet::new();
        apply_traffic(
            &event("Page.loadEventFired", json!({})),
            &mut in_flight,
            &mut Watch::default(),
        );
        apply_traffic(
            &event("Runtime.consoleAPICalled", json!({ "requestId": "R1" })),
            &mut in_flight,
            &mut Watch::default(),
        );
        assert!(in_flight.is_empty());
    }

    /// The document's own request is the one whose failure matters. A
    /// subresource that fails still produces a PDF (#28 makes that a choice).
    #[test]
    fn only_the_documents_own_failure_is_a_navigation_failure() {
        let mut in_flight = HashSet::new();
        let mut watch = Watch::default();

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
            &mut watch,
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
            &mut watch,
        );

        // An image that 404s is not a failed conversion.
        apply_traffic(
            &event(
                "Network.loadingFailed",
                json!({ "requestId": "IMG", "errorText": "net::ERR_FAILED" }),
            ),
            &mut in_flight,
            &mut watch,
        );
        assert!(watch.report.document.is_none());

        // The document is.
        apply_traffic(
            &event(
                "Network.loadingFailed",
                json!({ "requestId": "DOC", "errorText": "net::ERR_PROXY_CONNECTION_FAILED" }),
            ),
            &mut in_flight,
            &mut watch,
        );
        assert_eq!(
            watch.report.document.as_ref().map(|failed| failed.error),
            Some(NetworkError::ConnectionRefused)
        );
        // And the image is reported separately, because a subresource is
        // somebody else's decision (D14).
        assert_eq!(watch.report.media.len(), 1);
        assert_eq!(watch.report.media[0].error, NetworkError::UnknownContent);
    }

    /// A redirect reuses the request id, so the first document-type request
    /// stays the one being watched rather than whatever it landed on.
    #[test]
    fn a_later_document_request_does_not_steal_the_first() {
        let mut in_flight = HashSet::new();
        let mut watch = Watch::default();
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
                &mut watch,
            );
        }
        assert_eq!(watch.document.as_deref(), Some("FIRST"));
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
