//! The protocol client, exercised against a stand-in browser.
//!
//! The stand-in speaks the real wire format over in-memory pipes, so framing,
//! correlation, sessions and shutdown are all covered without launching
//! anything. Tests that need a real Chromium come later and live elsewhere.

use rchtmltopdf_browser::cdp::framing::{Framed, frame};
use rchtmltopdf_browser::{Client, Error};
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

/// Replies the stand-in sends back for one received command.
type Handler = Box<dyn FnMut(&Value) -> Vec<Value> + Send>;

/// Wire a client to a stand-in browser driven by `handler`.
fn connect(mut handler: Handler) -> Client {
    let (ours, theirs) = tokio::io::duplex(1024 * 1024);
    let (our_read, our_write) = tokio::io::split(ours);
    let (their_read, their_write): (ReadHalf<DuplexStream>, WriteHalf<DuplexStream>) =
        tokio::io::split(theirs);

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

    Client::new(our_read, our_write)
}

/// The common case: echo back a successful result for every command.
fn echoing() -> Handler {
    Box::new(|request| {
        vec![json!({
            "id": request["id"],
            "result": { "method": request["method"] }
        })]
    })
}

// --- commands and replies ----------------------------------------------------

#[tokio::test]
async fn a_command_gets_its_reply() {
    let client = connect(Box::new(|request| {
        vec![json!({ "id": request["id"], "result": { "frameId": "F1" } })]
    }));

    let result = client
        .send("Page.navigate", json!({ "url": "about:blank" }))
        .await;
    assert_eq!(result.unwrap()["frameId"], "F1");
}

#[tokio::test]
async fn a_command_with_no_params_omits_them() {
    let client = connect(Box::new(|request| {
        // The browser rejects a null params field, so it must be absent, not null.
        assert!(request.get("params").is_none(), "params should be omitted");
        vec![json!({ "id": request["id"], "result": {} })]
    }));
    client.send("Page.enable", Value::Null).await.unwrap();
}

/// Replies may come back in any order. Matching is by id, never by arrival.
#[tokio::test]
async fn replies_are_matched_by_id_not_by_order() {
    let client = connect(Box::new(|request| {
        let id = request["id"].as_u64().unwrap();
        // Answer the second command first.
        if id == 1 {
            vec![]
        } else {
            vec![
                json!({ "id": id, "result": { "which": "second" } }),
                json!({ "id": 1, "result": { "which": "first" } }),
            ]
        }
    }));

    let first = client.send("First.command", Value::Null);
    let second = client.send("Second.command", Value::Null);
    let (first, second) = tokio::join!(first, second);

    assert_eq!(first.unwrap()["which"], "first");
    assert_eq!(second.unwrap()["which"], "second");
}

/// Genuinely concurrent, on separate tasks, so ids really do interleave.
#[tokio::test]
async fn many_commands_can_be_in_flight_at_once() {
    let client = std::sync::Arc::new(connect(echoing()));

    let mut tasks = Vec::new();
    for _ in 0..50 {
        let client = std::sync::Arc::clone(&client);
        tasks.push(tokio::spawn(async move {
            client
                .send("Runtime.evaluate", json!({ "expression": "1" }))
                .await
        }));
    }

    for task in tasks {
        let result = task.await.unwrap().unwrap();
        assert_eq!(result["method"], "Runtime.evaluate");
    }
}

// --- errors ------------------------------------------------------------------

#[tokio::test]
async fn an_error_reply_becomes_a_typed_error_never_a_panic() {
    let client = connect(Box::new(|request| {
        vec![json!({
            "id": request["id"],
            "error": { "code": -32000, "message": "Cannot navigate to invalid URL" }
        })]
    }));

    match client.send("Page.navigate", json!({ "url": "nope" })).await {
        Err(Error::Protocol(error)) => {
            assert_eq!(error.code, -32000);
            assert_eq!(error.message, "Cannot navigate to invalid URL");
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

/// A message we cannot decode costs us that message, not the connection.
#[tokio::test]
async fn a_malformed_message_does_not_kill_the_session() {
    let client = connect(Box::new(|request| {
        vec![
            json!("this is not a protocol message"),
            json!({ "id": request["id"], "result": { "ok": true } }),
        ]
    }));

    assert_eq!(
        client.send("Page.enable", Value::Null).await.unwrap()["ok"],
        true
    );
    assert!(!client.is_closed());
}

// --- events ------------------------------------------------------------------

#[tokio::test]
async fn events_reach_a_subscriber_and_are_filtered_by_method() {
    let client = connect(Box::new(|request| {
        vec![
            json!({ "id": request["id"], "result": {} }),
            json!({ "method": "Network.requestWillBeSent", "params": { "requestId": "R1" } }),
            json!({ "method": "Page.loadEventFired", "params": { "timestamp": 1.5 } }),
        ]
    }));

    let mut loads = client.subscribe("Page.loadEventFired");
    client.send("Page.enable", Value::Null).await.unwrap();

    let event = loads.next().await.unwrap();
    assert_eq!(event.method, "Page.loadEventFired");
    assert_eq!(event.params["timestamp"], 1.5);
    assert_eq!(event.domain(), "Page");
}

#[tokio::test]
async fn subscribe_all_sees_every_event() {
    let client = connect(Box::new(|request| {
        vec![
            json!({ "id": request["id"], "result": {} }),
            json!({ "method": "A.one", "params": {} }),
            json!({ "method": "B.two", "params": {} }),
        ]
    }));

    let mut all = client.subscribe_all();
    client.send("Page.enable", Value::Null).await.unwrap();

    assert_eq!(all.next().await.unwrap().method, "A.one");
    assert_eq!(all.next().await.unwrap().method, "B.two");
}

// --- sessions ----------------------------------------------------------------

#[tokio::test]
async fn attaching_is_flattened_and_routes_later_commands() {
    let client = connect(Box::new(|request| match request["method"].as_str() {
        Some("Target.attachToTarget") => {
            // The acceptance criterion: flattened, not the legacy envelope.
            assert_eq!(request["params"]["flatten"], true);
            assert_eq!(request["params"]["targetId"], "T1");
            vec![json!({ "id": request["id"], "result": { "sessionId": "S1" } })]
        }
        _ => {
            // Everything after must carry the session id as a top-level field.
            assert_eq!(request["sessionId"], "S1");
            vec![json!({
                "id": request["id"],
                "sessionId": "S1",
                "result": { "seen": request["method"].clone() }
            })]
        }
    }));

    let session = client.attach_to_target("T1").await.unwrap();
    assert_eq!(session.id().as_str(), "S1");

    let result = session.send("Page.enable", Value::Null).await.unwrap();
    assert_eq!(result["seen"], "Page.enable");
}

#[tokio::test]
async fn a_session_subscription_ignores_other_sessions() {
    let client = connect(Box::new(|request| {
        vec![
            json!({ "id": request["id"], "result": { "sessionId": "S1" } }),
            json!({ "method": "Page.loadEventFired", "params": { "from": "other" }, "sessionId": "S2" }),
            json!({ "method": "Page.loadEventFired", "params": { "from": "browser" } }),
            json!({ "method": "Page.loadEventFired", "params": { "from": "ours" }, "sessionId": "S1" }),
        ]
    }));

    let session = client.attach_to_target("T1").await.unwrap();
    let mut loads = session.subscribe("Page.loadEventFired");

    // Provoke the events, then read: the first one we see must be ours.
    session.send("Page.enable", Value::Null).await.ok();
    let event = loads.next().await.unwrap();
    assert_eq!(event.params["from"], "ours");
}

#[tokio::test]
async fn attaching_without_a_session_id_is_reported_not_panicked() {
    let client = connect(Box::new(|request| {
        vec![json!({ "id": request["id"], "result": {} })]
    }));

    match client.attach_to_target("T1").await {
        Err(Error::Malformed { detail }) => assert!(detail.contains("sessionId")),
        other => panic!("expected a malformed-message error, got {other:?}"),
    }
}

// --- shutdown ----------------------------------------------------------------

/// Dropping the client must close the write pipe, because that is what tells a
/// real browser to exit.
#[tokio::test]
async fn dropping_the_client_closes_the_pipe() {
    let (ours, theirs) = tokio::io::duplex(4096);
    let (our_read, our_write) = tokio::io::split(ours);
    let (their_read, _their_write) = tokio::io::split(theirs);

    let saw_eof = tokio::spawn(async move {
        let mut incoming = Framed::new(their_read);
        // None means end of stream, which is the browser seeing its input close.
        matches!(incoming.next_message().await, Ok(None))
    });

    let client = Client::new(our_read, our_write);
    drop(client);

    assert!(
        saw_eof.await.unwrap(),
        "the browser side should have seen EOF"
    );
}

/// A browser that goes away mid-command must not leave the caller hanging
/// forever. This is the difference between a crashed Chromium surfacing as an
/// error and a conversion that never returns.
#[tokio::test]
async fn a_pending_command_fails_when_the_browser_goes_away() {
    let (ours, theirs) = tokio::io::duplex(4096);
    let (our_read, our_write) = tokio::io::split(ours);
    let (their_read, their_write) = tokio::io::split(theirs);

    tokio::spawn(async move {
        let mut incoming = Framed::new(their_read);
        // Take one command, then vanish without ever answering it.
        let _ = incoming.next_message().await;
        drop(their_write);
        drop(incoming);
    });

    let client = Client::new(our_read, our_write);
    match client
        .send("Page.navigate", json!({ "url": "about:blank" }))
        .await
    {
        Err(Error::ConnectionClosed) => {}
        other => panic!("expected ConnectionClosed, got {other:?}"),
    }
}

#[tokio::test]
async fn sending_after_the_connection_closed_fails_fast() {
    let (ours, theirs) = tokio::io::duplex(4096);
    let (our_read, our_write) = tokio::io::split(ours);
    drop(theirs); // the browser is gone before we say anything

    let client = Client::new(our_read, our_write);

    // The reader sees EOF and closes the connection.
    for _ in 0..100 {
        if client.is_closed() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(
        client.is_closed(),
        "the client should have noticed the browser leaving"
    );

    match client.send("Page.enable", Value::Null).await {
        Err(Error::ConnectionClosed) => {}
        other => panic!("expected ConnectionClosed, got {other:?}"),
    }
}

/// Asking for several methods gets those and nothing else.
///
/// The alternative was subscribing to everything and filtering, which is how a
/// caller measuring quiet periods ends up woken by unrelated chatter.
#[tokio::test]
async fn a_subscription_can_name_several_methods() {
    let client = connect(Box::new(|request| {
        vec![
            json!({ "id": request["id"], "result": {} }),
            json!({ "method": "Runtime.consoleAPICalled", "params": { "seq": 1 } }),
            json!({ "method": "Network.loadingFinished", "params": { "seq": 2 } }),
            json!({ "method": "Page.frameNavigated", "params": { "seq": 3 } }),
            json!({ "method": "Network.requestWillBeSent", "params": { "seq": 4 } }),
        ]
    }));

    let mut traffic =
        client.subscribe_many(&["Network.requestWillBeSent", "Network.loadingFinished"]);
    client.send("Network.enable", Value::Null).await.unwrap();

    // The console and navigation events in between must not arrive at all.
    assert_eq!(traffic.next().await.unwrap().params["seq"], 2);
    assert_eq!(traffic.next().await.unwrap().params["seq"], 4);
}

// --- the ways a connection can end -------------------------------------------

/// A stream that can no longer be framed has desynchronised, which is a
/// different diagnosis from the browser going away. The caller has to be able to
/// tell them apart, or the size limit's message can never reach anyone.
#[tokio::test]
async fn a_framing_error_reaches_the_caller_as_itself() {
    let (ours, theirs) = tokio::io::duplex(1024 * 1024);
    let (our_read, our_write) = tokio::io::split(ours);
    let (_their_read, mut their_write) = tokio::io::split(theirs);

    tokio::spawn(async move {
        // Endless bytes with no terminator: the framer gives up at its limit.
        let flood = vec![b'x'; 8192];
        loop {
            if their_write.write_all(&flood).await.is_err() {
                return;
            }
        }
    });

    let client = Client::new(our_read, our_write);

    // Whatever we ask, the answer is that the stream is unusable.
    match client.send("Page.enable", Value::Null).await {
        Err(Error::MessageTooLarge { limit }) => assert!(limit > 0),
        other => panic!("expected MessageTooLarge, got {other:?}"),
    }
}

/// Arriving after the connection has gone must give the same diagnosis as being
/// present when it went, not a generic one.
#[tokio::test]
async fn the_reason_survives_for_callers_that_arrive_later() {
    let (ours, theirs) = tokio::io::duplex(4096);
    let (our_read, our_write) = tokio::io::split(ours);
    let (_their_read, mut their_write) = tokio::io::split(theirs);

    tokio::spawn(async move {
        let flood = vec![b'x'; 8192];
        loop {
            if their_write.write_all(&flood).await.is_err() {
                return;
            }
        }
    });

    let client = Client::new(our_read, our_write);
    let first = client.send("Page.enable", Value::Null).await;
    assert!(matches!(first, Err(Error::MessageTooLarge { .. })));

    // Long after the fact, a second caller gets told the same thing.
    let second = client.send("Page.enable", Value::Null).await;
    assert!(
        matches!(second, Err(Error::MessageTooLarge { .. })),
        "got {second:?}"
    );
}

/// The race the pending map was restructured to make impossible: a request
/// registering itself just as the connection is torn down. Nothing here proves
/// the interleaving directly, but every one of these must come back rather than
/// hang, and before the change some of them could wait for ever.
#[tokio::test]
async fn requests_racing_a_teardown_all_come_back() {
    let (ours, theirs) = tokio::io::duplex(4096);
    let (our_read, our_write) = tokio::io::split(ours);

    let client = std::sync::Arc::new(Client::new(our_read, our_write));

    let mut tasks = Vec::new();
    for _ in 0..25 {
        let client = std::sync::Arc::clone(&client);
        tasks.push(tokio::spawn(async move {
            client.send("Page.enable", Value::Null).await
        }));
    }

    // Hang up while they are in flight.
    drop(theirs);

    for task in tasks {
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
        let outcome = outcome.expect("a request hung instead of failing").unwrap();
        assert!(outcome.is_err(), "should not have succeeded: {outcome:?}");
    }
}
