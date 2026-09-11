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

use crate::cdp::{Event, Session};
use crate::error::Result;
use crate::launch::Page;
use rchtmltopdf_core::settings::{LoadSettings, WebSettings};
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
    AwaitingNetworkIdle,
    AwaitingFonts,
    AwaitingWindowStatus,
    Delaying,
    Settled,
}

impl Stage {
    /// Phrased to drop into "timed out while …".
    pub fn describe(self) -> &'static str {
        match self {
            Stage::Navigating => "opening the document",
            Stage::AwaitingLoad => "waiting for the page to load",
            Stage::AwaitingNetworkIdle => "waiting for the network to go idle",
            Stage::AwaitingFonts => "waiting for web fonts",
            Stage::AwaitingWindowStatus => "waiting for window.status",
            Stage::Delaying => "waiting out the JavaScript delay",
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
    /// document arrives, not after.
    pub async fn prepare(&self, web: &WebSettings) -> Result<()> {
        let session = self.session();
        session.send("Page.enable", Value::Null).await?;
        session.send("Network.enable", Value::Null).await?;

        // Before the document arrives, not after: media queries decide which
        // resources are fetched at all.
        self.emulate_media(web).await?;

        if !web.javascript {
            session
                .send(
                    "Emulation.setScriptExecutionDisabled",
                    json!({ "value": true }),
                )
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

        progress.enter(Stage::AwaitingNetworkIdle);
        wait_for_quiet_network(&mut traffic).await;

        progress.enter(Stage::AwaitingFonts);
        wait_for_fonts(session).await;

        match &load.window_status {
            Some(wanted) => {
                progress.enter(Stage::AwaitingWindowStatus);
                wait_for_window_status(session, wanted).await;
            }
            None => {
                progress.enter(Stage::Delaying);
                tokio::time::sleep(load.javascript_delay).await;
            }
        }

        progress.enter(Stage::Settled);
        Ok(())
    }
}

/// Wait until nothing has been in flight for a while.
///
/// Counts by request id rather than by a running total, so a reply that arrives
/// for a request we never counted cannot drive the total negative and declare
/// the page idle early.
async fn wait_for_quiet_network(events: &mut crate::cdp::Events) {
    let mut in_flight: HashSet<String> = HashSet::new();

    loop {
        let next = if in_flight.is_empty() {
            // Nothing outstanding: give the page a moment to start something
            // else before calling it quiet.
            match tokio::time::timeout(QUIET_PERIOD, events.next()).await {
                Err(_) => return,
                Ok(event) => event,
            }
        } else {
            events.next().await
        };

        let Some(event) = next else {
            // The connection went away. Whatever happens next will report it
            // better than this loop can.
            return;
        };
        apply_traffic(&event, &mut in_flight);
    }
}

fn apply_traffic(event: &Event, in_flight: &mut HashSet<String>) {
    let Some(id) = event.params.get("requestId").and_then(Value::as_str) else {
        return;
    };

    match event.method.as_str() {
        "Network.requestWillBeSent" => {
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
        "Network.loadingFinished" | "Network.loadingFailed" => {
            in_flight.remove(id);
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
        );
        assert_eq!(in_flight.len(), 1);

        apply_traffic(
            &event("Network.loadingFinished", json!({ "requestId": "R1" })),
            &mut in_flight,
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
        );
        apply_traffic(
            &event("Network.loadingFailed", json!({ "requestId": "R1" })),
            &mut in_flight,
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
        );
        apply_traffic(
            &event(
                "Network.requestWillBeSent",
                json!({ "requestId": "R1", "request": { "url": "https://example.com/a" } }),
            ),
            &mut in_flight,
        );
        assert_eq!(in_flight.len(), 1, "the real request is still outstanding");
    }

    #[test]
    fn unrelated_events_are_ignored() {
        let mut in_flight = HashSet::new();
        apply_traffic(&event("Page.loadEventFired", json!({})), &mut in_flight);
        apply_traffic(
            &event("Runtime.consoleAPICalled", json!({ "requestId": "R1" })),
            &mut in_flight,
        );
        assert!(in_flight.is_empty());
    }

    #[test]
    fn every_stage_can_be_described() {
        for stage in [
            Stage::Navigating,
            Stage::AwaitingLoad,
            Stage::AwaitingNetworkIdle,
            Stage::AwaitingFonts,
            Stage::AwaitingWindowStatus,
            Stage::Delaying,
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
