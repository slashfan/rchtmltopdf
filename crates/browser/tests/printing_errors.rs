//! The printing paths that a real browser cannot be asked to produce.
//!
//! Chromium will not return invalid base64, or stop a stream without ending it,
//! or hand back something that is not a PDF. Those branches exist because a
//! protocol that can do any of them should not take the process down, and
//! without a stand-in they are unreachable, which means untested.
//!
//! Deterministic and fast: no browser anywhere near these.

use base64::Engine;
use rchtmltopdf_browser::cdp::framing::{Framed, frame};
use rchtmltopdf_browser::{Client, Error, Page};
use rchtmltopdf_core::settings::{ObjectSettings, PageSetup};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

type Handler = Box<dyn FnMut(&Value) -> Vec<Value> + Send>;

/// A page wired to a stand-in browser that answers however `handler` says.
fn page_over(mut handler: Handler) -> (Client, Page) {
    let (ours, theirs) = tokio::io::duplex(1024 * 1024);
    let (our_read, our_write) = tokio::io::split(ours);
    let (their_read, their_write) = tokio::io::split(theirs);

    tokio::spawn(async move {
        let mut incoming = Framed::new(their_read);
        let mut outgoing = their_write;
        while let Ok(Some(message)) = incoming.next_message().await {
            let Ok(request) = serde_json::from_slice::<Value>(&message) else {
                continue;
            };
            for reply in handler(&request) {
                let bytes = frame(&serde_json::to_vec(&reply).unwrap());
                if outgoing.write_all(&bytes).await.is_err() {
                    return;
                }
            }
        }
    });

    let client = Client::new(our_read, our_write);
    let page = Page::over_session(
        client.session(rchtmltopdf_browser::SessionId::new("S1")),
        "T1",
    );
    (client, page)
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Answers a print request with the given stream chunks, in order.
fn streaming(chunks: Vec<(String, bool)>) -> Handler {
    let mut remaining = chunks.into_iter();
    Box::new(move |request| {
        let id = request["id"].clone();
        match request["method"].as_str() {
            Some("Emulation.setEmulatedMedia") => vec![json!({ "id": id, "result": {} })],
            Some("Page.printToPDF") => {
                vec![json!({ "id": id, "result": { "stream": "handle-1" } })]
            }
            Some("IO.read") => {
                let (data, eof) = remaining.next().unwrap_or((String::new(), true));
                vec![json!({
                    "id": id,
                    "result": { "data": data, "base64Encoded": true, "eof": eof }
                })]
            }
            Some("IO.close") => vec![json!({ "id": id, "result": {} })],
            _ => vec![json!({ "id": id, "result": {} })],
        }
    })
}

async fn print(handler: Handler) -> Result<Vec<u8>, Error> {
    let (_client, page) = page_over(handler);
    page.print_to_pdf(
        &PageSetup::default(),
        &ObjectSettings::page(rchtmltopdf_core::Input::Stdin),
    )
    .await
}

/// The loop has only ever run once in any test, because every real fixture fits
/// in one chunk. This is the only thing that exercises the accumulation.
#[tokio::test]
async fn a_pdf_arriving_in_several_chunks_is_reassembled() {
    let document = b"%PDF-1.4\nthis document arrived in three pieces\n%%EOF";
    let chunks = vec![
        (encode(&document[..10]), false),
        (encode(&document[10..30]), false),
        (encode(&document[30..]), true),
    ];

    let pdf = print(streaming(chunks)).await.unwrap();
    assert_eq!(pdf, document);
}

#[tokio::test]
async fn a_single_chunk_still_works() {
    let document = b"%PDF-1.4\nsmall\n%%EOF";
    let pdf = print(streaming(vec![(encode(document), true)]))
        .await
        .unwrap();
    assert_eq!(pdf, document);
}

#[tokio::test]
async fn invalid_base64_is_reported_rather_than_panicking() {
    let chunks = vec![("not base64 at all !!!".to_string(), true)];
    match print(streaming(chunks)).await {
        Err(Error::Malformed { detail }) => assert!(detail.contains("base64"), "{detail}"),
        other => panic!("expected a malformed-stream error, got {other:?}"),
    }
}

/// A stream that returns nothing and never says it has ended would otherwise
/// spin for ever. The guard is the only thing standing between that and a
/// conversion that never returns.
#[tokio::test]
async fn a_stream_that_stops_without_ending_does_not_spin() {
    let chunks = vec![(encode(b"%PDF-1.4"), false), (String::new(), false)];

    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), print(streaming(chunks)))
        .await
        .expect("should have given up rather than spinning");

    match outcome {
        Err(Error::Malformed { detail }) => assert!(detail.contains("without ending"), "{detail}"),
        other => panic!("expected a malformed-stream error, got {other:?}"),
    }
}

#[tokio::test]
async fn something_that_is_not_a_pdf_is_refused() {
    let chunks = vec![(encode(b"<html>this is not a pdf</html>"), true)];
    match print(streaming(chunks)).await {
        Err(Error::Malformed { detail }) => assert!(detail.contains("not a PDF"), "{detail}"),
        other => panic!("expected a malformed-stream error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_print_that_returns_no_stream_handle_is_reported() {
    let handler: Handler = Box::new(|request| vec![json!({ "id": request["id"], "result": {} })]);
    match print(handler).await {
        Err(Error::Malformed { detail }) => assert!(detail.contains("stream handle"), "{detail}"),
        other => panic!("expected a malformed-reply error, got {other:?}"),
    }
}

/// The handle is a resource inside the browser. It has to be given back even
/// when reading it failed, or a pooled browser accumulates them.
#[tokio::test]
async fn the_stream_handle_is_closed_even_when_reading_fails() {
    let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&closed);

    let handler: Handler = Box::new(move |request| {
        let id = request["id"].clone();
        match request["method"].as_str() {
            Some("Page.printToPDF") => vec![json!({ "id": id, "result": { "stream": "h" } })],
            Some("IO.read") => vec![json!({
                "id": id,
                "result": { "data": "!!! not base64 !!!", "base64Encoded": true, "eof": false }
            })],
            Some("IO.close") => {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                vec![json!({ "id": id, "result": {} })]
            }
            _ => vec![json!({ "id": id, "result": {} })],
        }
    });

    assert!(print(handler).await.is_err());
    assert!(
        closed.load(std::sync::atomic::Ordering::SeqCst),
        "the handle was left open after a failed read"
    );
}
