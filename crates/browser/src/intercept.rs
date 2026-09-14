//! Answering every request the document makes, one at a time.
//!
//! Four of wkhtmltopdf's options need a say in an individual request, and
//! Chromium offers no switch for any of them:
//!
//! - the local file rule (see [`crate::file_access`]), which has no switch at
//!   all;
//! - `--encoding`, which needs the document served with a `Content-Type` it does
//!   not have;
//! - `--custom-header` **without** propagation, which is the document's own
//!   request and nothing else;
//! - `--username` and `--password`, which answer a 401 rather than being sent
//!   ahead of one.
//!
//! So `Fetch.enable` pauses each request before it goes out, and this decides
//! what to do with it.
//!
//! # Three things about `Fetch` that bite
//!
//! **Every paused request must be answered, including the navigation.** Enabling
//! the domain without something reading `Fetch.requestPaused` does not slow the
//! page down, it stops it dead: the document itself never loads and the
//! conversion sits there until the deadline. So enabling and answering are one
//! call here, and the guard that comes back keeps the answering alive.
//!
//! **Subscribe before enabling.** The first request can pause before
//! `Fetch.enable` has returned, and an event with nobody subscribed is gone.
//!
//! **`Fetch.continueRequest` replaces the headers rather than adding to them.**
//! Sending only the header the user asked for drops the `Accept`, the
//! `User-Agent` and everything else the browser was going to send. The paused
//! event carries the request's own headers, so they are merged.

use crate::cdp::Session;
use crate::error::Result;
use crate::file_access::{FileAccess, Verdict};
use base64::Engine;
use rchtmltopdf_core::NetworkError;
use rchtmltopdf_core::settings::Pair;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Charset {
    /// The document's URL: the one request this applies to.
    pub url: String,
    /// Where to read the bytes from.
    pub path: PathBuf,
    /// What to tell the browser they are.
    pub name: String,
}

/// What to answer a 401 with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// Everything the handler needs to decide, bound to one document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    pub files: FileAccess,
    /// The document's own URL, which is what "the document's own request" means.
    pub document: String,
    pub serve_as: Option<Charset>,
    /// Headers for the document's request and for nothing else.
    ///
    /// Empty when `--custom-header-propagation` is on, because then the headers
    /// go on every request through `Network.setExtraHTTPHeaders` and this has
    /// nothing to add. That asymmetry is wkhtmltopdf's, and it surprises people.
    pub document_headers: Vec<Pair>,
    pub credentials: Option<Credentials>,
}

impl Rules {
    /// Whether anything here needs a request paused.
    pub fn needed(&self) -> bool {
        self.files.polices_anything()
            || self.serve_as.is_some()
            || !self.document_headers.is_empty()
            || self.credentials.is_some()
    }
}

/// Interception, for as long as this is held.
#[derive(Debug)]
pub struct Interception {
    task: JoinHandle<()>,
    refused: Arc<Mutex<Vec<Refusal>>>,
    rejected: Arc<AtomicBool>,
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

impl Refusal {
    /// The error the request was failed with.
    ///
    /// A refusal is answered with `AccessDenied`, so this is what the browser
    /// reports back for it, and it is the name the exit line carries: a refused
    /// file is exit 1 whatever the handlers say (D49). wkhtmltopdf wrote
    /// `ProtocolUnknownError` here, because it swapped the file for
    /// `about:blank` and failed that instead. The name is an artefact of the
    /// swap rather than a rule, and D48 already declined to copy one of those.
    pub fn error(&self) -> NetworkError {
        NetworkError::ContentAccessDenied
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

    /// Whether the credentials were offered and asked for again.
    ///
    /// A second challenge for the same request means the first answer was wrong.
    /// Chromium would go on asking; cancelling stops that, and it also means the
    /// server's own 401 body becomes the response, which renders as a page.
    /// Reporting it is what keeps a wrong password from printing as a document.
    pub fn credentials_rejected(&self) -> bool {
        self.rejected.load(Ordering::SeqCst)
    }
}

impl Drop for Interception {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Start policing a session's requests.
///
/// Returns `None` when nothing here needs a say, in which case interception is
/// not installed and every request keeps the round trip it would have paid to be
/// waved through.
pub async fn install(session: &Session, rules: Rules) -> Result<Option<Interception>> {
    if !rules.needed() {
        return Ok(None);
    }

    // Read once, up front, rather than inside the handler: the answer is the
    // same every time. An unreadable document is left to load itself; it will
    // fail on its own and say so better than this could.
    let served = rules.serve_as.as_ref().and_then(|charset| {
        let body = std::fs::read(&charset.path).ok()?;
        Some(base64::engine::general_purpose::STANDARD.encode(body))
    });

    // Before `Fetch.enable`, not after: the first request can pause while that
    // call is still in flight.
    let mut events = session.subscribe_many(&["Fetch.requestPaused", "Fetch.authRequired"]);
    session
        .send(
            "Fetch.enable",
            // Without this the auth event never arrives and Chromium answers the
            // challenge itself, which means not answering it.
            json!({ "handleAuthRequests": rules.credentials.is_some() }),
        )
        .await?;

    let refused = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&refused);
    let rejected = Arc::new(AtomicBool::new(false));
    let note_rejection = Arc::clone(&rejected);
    let answering = session.clone();

    let task = tokio::spawn(async move {
        // A wrong password is retried by the browser for ever unless somebody
        // stops. Credentials are offered once per request and the second ask is
        // cancelled, which turns a bad password into a reported failure rather
        // than a conversion that never ends.
        let mut offered: HashMap<String, ()> = HashMap::new();

        while let Some(event) = events.next().await {
            let Some(id) = event
                .params
                .get("requestId")
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };

            if event.method == "Fetch.authRequired" {
                answer_challenge(
                    &answering,
                    &id,
                    &rules.credentials,
                    &mut offered,
                    &note_rejection,
                )
                .await;
                continue;
            }

            let request = event.params.get("request");
            let url = request
                .and_then(|request| request.get("url"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let is_document = url == rules.document;

            // The document itself, answered with the charset that was asked for
            // rather than fetched and guessed at.
            if let (true, Some(charset), Some(body)) = (is_document, &rules.serve_as, &served) {
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

            // Errors are dropped on purpose. A request whose page navigated away
            // can no longer be answered, and that is not a reason to stop
            // answering the rest — which is what returning would do.
            match rules.files.verdict(url) {
                Verdict::Allow => {
                    let mut params = json!({ "requestId": id });
                    if is_document && !rules.document_headers.is_empty() {
                        params["headers"] = merged_headers(request, &rules.document_headers);
                    }
                    let _ = answering.send("Fetch.continueRequest", params).await;
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

    Ok(Some(Interception {
        task,
        refused,
        rejected,
    }))
}

async fn answer_challenge(
    session: &Session,
    id: &str,
    credentials: &Option<Credentials>,
    offered: &mut HashMap<String, ()>,
    rejected: &AtomicBool,
) {
    let response = match credentials {
        Some(credentials) if offered.insert(id.to_string(), ()).is_none() => json!({
            "response": "ProvideCredentials",
            "username": credentials.username,
            "password": credentials.password,
        }),
        // Asked twice means the first answer was wrong, and there is no second
        // password to try. Cancelling stops Chromium asking for ever; it also
        // makes the server's own 401 body the response, which would render as a
        // document, so the rejection is recorded and the conversion fails on it.
        _ => {
            if credentials.is_some() {
                rejected.store(true, Ordering::SeqCst);
            }
            json!({ "response": "CancelAuth" })
        }
    };

    let _ = session
        .send(
            "Fetch.continueWithAuth",
            json!({ "requestId": id, "authChallengeResponse": response }),
        )
        .await;
}

/// The request's own headers with the user's added, as the protocol wants them.
///
/// **Merged, not replaced.** `Fetch.continueRequest` takes the whole header set,
/// so sending only `X-Tenant: 42` would drop the `Accept`, the `User-Agent` and
/// the cookies the browser was about to send.
fn merged_headers(request: Option<&Value>, extra: &[Pair]) -> Value {
    let mut headers: Vec<Value> = request
        .and_then(|request| request.get("headers"))
        .and_then(Value::as_object)
        .map(|existing| {
            existing
                .iter()
                .filter(|(name, _)| !extra.iter().any(|pair| pair.name.eq_ignore_ascii_case(name)))
                .map(|(name, value)| {
                    json!({ "name": name, "value": value.as_str().unwrap_or_default() })
                })
                .collect()
        })
        .unwrap_or_default();

    headers.extend(
        extra
            .iter()
            .map(|pair| json!({ "name": pair.name, "value": pair.value })),
    );
    Value::Array(headers)
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

    fn pair(name: &str, value: &str) -> Pair {
        Pair {
            name: name.into(),
            value: value.into(),
        }
    }

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

    /// The trap: `continueRequest` takes the whole header set, so anything not
    /// sent back is dropped.
    #[test]
    fn the_browsers_own_headers_survive_a_header_being_added() {
        let request = json!({
            "headers": { "Accept": "text/html", "User-Agent": "Chrome" }
        });
        let merged = merged_headers(Some(&request), &[pair("X-Tenant", "42")]);
        let names: Vec<&str> = merged
            .as_array()
            .unwrap()
            .iter()
            .map(|header| header["name"].as_str().unwrap())
            .collect();

        assert!(names.contains(&"Accept"), "{names:?}");
        assert!(names.contains(&"User-Agent"), "{names:?}");
        assert!(names.contains(&"X-Tenant"), "{names:?}");
    }

    /// A header the user names replaces the browser's rather than joining it,
    /// and header names are case-insensitive so the comparison has to be too.
    #[test]
    fn a_named_header_replaces_the_browsers_own() {
        let request = json!({ "headers": { "accept": "text/html" } });
        let merged = merged_headers(Some(&request), &[pair("Accept", "application/pdf")]);
        let headers = merged.as_array().unwrap();

        assert_eq!(headers.len(), 1, "{headers:?}");
        assert_eq!(headers[0]["value"], json!("application/pdf"));
    }

    /// Rules that could refuse nothing: a local document that has been given the
    /// run of the disk.
    fn open() -> Rules {
        Rules {
            files: crate::file_access::Policy {
                enabled: true,
                allowed: Vec::new(),
            }
            .about("file:///nowhere.html"),
            ..Rules::default()
        }
    }

    #[test]
    fn nothing_is_intercepted_when_nothing_needs_a_say() {
        assert!(!open().needed());

        assert!(
            Rules {
                document_headers: vec![pair("X-A", "1")],
                ..open()
            }
            .needed()
        );
        assert!(
            Rules {
                credentials: Some(Credentials::default()),
                ..open()
            }
            .needed()
        );
        assert!(
            Rules {
                serve_as: Some(Charset {
                    url: "file:///nowhere.html".into(),
                    path: PathBuf::from("/nowhere.html"),
                    name: "UTF-8".into(),
                }),
                ..open()
            }
            .needed()
        );

        // And the default, which has no document at all, polices everything:
        // nothing local is readable by a page from nowhere.
        assert!(Rules::default().needed());
    }

    /// A wrong password is retried for ever unless somebody stops. The second
    /// ask for the same request is cancelled, which makes it a reported failure
    /// rather than a conversion that never ends.
    #[test]
    fn credentials_are_offered_once_per_request() {
        let mut offered = HashMap::new();
        assert!(offered.insert("R1".to_string(), ()).is_none(), "first ask");
        assert!(offered.insert("R1".to_string(), ()).is_some(), "second ask");
        assert!(
            offered.insert("R2".to_string(), ()).is_none(),
            "another request"
        );
    }
}
