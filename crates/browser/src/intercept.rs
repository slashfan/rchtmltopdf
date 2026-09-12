//! Answering every request the document makes, one at a time.
//!
//! Chromium has no switch for wkhtmltopdf's local file rule (see
//! [`crate::file_access`]), so the rule is applied per request: `Fetch.enable`
//! pauses each one before it goes out, and this decides whether to let it go.
//!
//! # Two things about `Fetch` that bite
//!
//! **Every paused request must be answered, including the navigation.** Enabling
//! the domain without something reading `Fetch.requestPaused` does not slow the
//! page down, it stops it dead: the document itself never loads and the
//! conversion sits there until the deadline. So enabling and answering are one
//! call here, and the guard that comes back keeps the answering alive. Dropping
//! it stops interception, which is why it is returned rather than spawned and
//! forgotten.
//!
//! **Subscribe before enabling.** The first request can pause before
//! `Fetch.enable` has returned, and an event with nobody subscribed is gone.

use crate::cdp::Session;
use crate::error::Result;
use crate::file_access::{FileAccess, Verdict};
use base64::Engine;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

/// A document to answer ourselves, so it is read as the charset asked for.
///
/// `--encoding` tells the renderer what a document that does not declare a
/// charset is written in. There is no protocol command for that and no launch
/// switch either — `--default-encoding` was tried and does nothing — so the only
/// way in is to answer the document's own request with a `Content-Type` that
/// says so. The URL is unchanged, so every relative link in the document still
/// resolves against where the file actually is.
#[derive(Debug, Clone)]
pub struct Charset {
    /// The document's URL: the one request this applies to.
    pub url: String,
    /// Where to read the bytes from.
    pub path: PathBuf,
    /// What to tell the browser they are.
    pub name: String,
}

/// Interception, for as long as this is held.
#[derive(Debug)]
pub struct Interception {
    task: JoinHandle<()>,
    refused: Arc<Mutex<Vec<Refusal>>>,
}

/// One request that was not allowed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub url: String,
    pub reason: &'static str,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} was not loaded: {}", self.url, self.reason)
    }
}

impl Interception {
    /// What was refused, in the order it was asked for, once each.
    ///
    /// A stylesheet asked for twice is one thing to tell the user about, and a
    /// page that retries a blocked image in a loop must not turn into a thousand
    /// lines of stderr.
    pub fn refused(&self) -> Vec<Refusal> {
        self.refused.lock().expect("not poisoned").clone()
    }
}

impl Drop for Interception {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Start policing a session's requests.
///
/// Returns `None` when the policy could never refuse anything, in which case
/// interception is not installed and every request keeps the round trip it would
/// have paid to be waved through.
pub async fn install(
    session: &Session,
    access: FileAccess,
    serve_as: Option<Charset>,
) -> Result<Option<Interception>> {
    // Read once, up front, rather than inside the handler: the answer is the
    // same every time and a failure here is better than a half-answered request.
    // An unreadable document is left to load itself; it will fail on its own and
    // say so better than this could.
    let document = serve_as.and_then(|charset| {
        let body = std::fs::read(&charset.path).ok()?;
        Some((
            charset,
            base64::engine::general_purpose::STANDARD.encode(body),
        ))
    });

    if !access.polices_anything() && document.is_none() {
        return Ok(None);
    }

    // Before `Fetch.enable`, not after: the first request can pause while that
    // call is still in flight.
    let mut paused = session.subscribe("Fetch.requestPaused");
    session.send("Fetch.enable", json!({})).await?;

    let refused = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&refused);
    let answering = session.clone();

    let task = tokio::spawn(async move {
        while let Some(event) = paused.next().await {
            let Some(id) = event.params.get("requestId").and_then(Value::as_str) else {
                continue;
            };
            let url = event
                .params
                .get("request")
                .and_then(|request| request.get("url"))
                .and_then(Value::as_str)
                .unwrap_or_default();

            // The document itself, answered with the charset that was asked for
            // rather than fetched and guessed at.
            if let Some((charset, body)) = &document {
                if url == charset.url {
                    let _ = answering
                        .send(
                            "Fetch.fulfillRequest",
                            json!({
                                "requestId": id,
                                "responseCode": 200,
                                "responseHeaders": [{
                                    "name": "Content-Type",
                                    "value": format!("text/html; charset={}", charset.name),
                                }],
                                "body": body,
                            }),
                        )
                        .await;
                    continue;
                }
            }

            // Errors are dropped on purpose. A request whose page navigated away
            // can no longer be answered, and that is not a reason to stop
            // answering the rest — which is what returning would do.
            match access.verdict(url) {
                Verdict::Allow => {
                    let _ = answering
                        .send("Fetch.continueRequest", json!({ "requestId": id }))
                        .await;
                }
                Verdict::Block(reason) => {
                    remember(&record, url, reason);
                    let _ = answering
                        .send(
                            "Fetch.failRequest",
                            json!({ "requestId": id, "errorReason": "AccessDenied" }),
                        )
                        .await;
                }
            }
        }
    });

    Ok(Some(Interception { task, refused }))
}

fn remember(record: &Mutex<Vec<Refusal>>, url: &str, reason: &'static str) {
    let mut held = record.lock().expect("not poisoned");
    if held.iter().any(|refusal| refusal.url == url) {
        return;
    }
    held.push(Refusal {
        url: url.to_string(),
        reason,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_refusal_is_only_worth_saying_once() {
        let record = Mutex::new(Vec::new());
        remember(&record, "file:///a.css", "no");
        remember(&record, "file:///a.css", "no");
        remember(&record, "file:///b.css", "no");

        let held = record.lock().unwrap();
        assert_eq!(held.len(), 2);
        assert_eq!(held[0].url, "file:///a.css");
    }

    #[test]
    fn a_refusal_reads_as_a_sentence() {
        let refusal = Refusal {
            url: "file:///etc/hostname".into(),
            reason: "local file access is off by default",
        };
        assert_eq!(
            refusal.to_string(),
            "file:///etc/hostname was not loaded: local file access is off by default"
        );
    }
}
