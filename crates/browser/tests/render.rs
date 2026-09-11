//! The wait ladder, against a real browser and a real server.

mod support;

use rchtmltopdf_browser::render::{Progress, Stage};
use rchtmltopdf_core::settings::{LoadSettings, WebSettings};
use serde_json::json;
use std::time::{Duration, Instant};
use support::{TestServer, launch, one_at_a_time};

#[tokio::test]
async fn a_plain_page_settles() {
    let _turn = one_at_a_time().await;
    let Some(browser) = launch().await else {
        return;
    };
    let server = TestServer::start().await;
    let page = browser.new_page().await.unwrap();
    page.prepare(&WebSettings::default()).await.unwrap();

    let progress = Progress::new();
    page.load(&server.url("/plain"), &LoadSettings::default(), &progress)
        .await
        .unwrap();

    assert_eq!(progress.current(), Stage::Settled);
    browser.close().await.unwrap();
}

/// The point of waiting for the network rather than only for the load event.
/// The stylesheet arrives well after load, and printing before it would use the
/// wrong styles.
#[tokio::test]
async fn a_slow_stylesheet_holds_the_page_open() {
    let _turn = one_at_a_time().await;
    let Some(browser) = launch().await else {
        return;
    };
    let server = TestServer::start().await;
    let page = browser.new_page().await.unwrap();
    page.prepare(&WebSettings::default()).await.unwrap();

    let started = Instant::now();
    page.load(
        &server.url("/slow-css"),
        &LoadSettings::default(),
        &Progress::new(),
    )
    .await
    .unwrap();
    let waited = started.elapsed();

    assert!(
        waited >= Duration::from_millis(700),
        "settled after {waited:?}, before the stylesheet could have arrived"
    );
    browser.close().await.unwrap();
}

/// A resource that 404s must end the wait, not extend it. Counting a request
/// that never reports completion is how a converter hangs for ever.
#[tokio::test]
async fn a_missing_image_does_not_hold_the_page_open() {
    let _turn = one_at_a_time().await;
    let Some(browser) = launch().await else {
        return;
    };
    let server = TestServer::start().await;
    let page = browser.new_page().await.unwrap();
    page.prepare(&WebSettings::default()).await.unwrap();

    let settled = tokio::time::timeout(
        Duration::from_secs(15),
        page.load(
            &server.url("/missing-image"),
            &LoadSettings::default(),
            &Progress::new(),
        ),
    )
    .await;

    assert!(settled.is_ok(), "a 404 subresource hung the wait");
    settled.unwrap().unwrap();
    browser.close().await.unwrap();
}

/// `--window-status` replaces the delay rather than adding to it, and it has to
/// actually wait for a status that arrives late.
#[tokio::test]
async fn window_status_is_waited_for() {
    let _turn = one_at_a_time().await;
    let Some(browser) = launch().await else {
        return;
    };
    let server = TestServer::start().await;
    let page = browser.new_page().await.unwrap();
    page.prepare(&WebSettings::default()).await.unwrap();

    let load = LoadSettings {
        window_status: Some("ready".into()),
        // Long enough that passing this test by accident is not possible: if the
        // delay were used instead of the status, the wait would be much longer.
        javascript_delay: Duration::from_secs(30),
        ..LoadSettings::default()
    };

    let started = Instant::now();
    tokio::time::timeout(
        Duration::from_secs(20),
        page.load(&server.url("/late-status"), &load, &Progress::new()),
    )
    .await
    .expect("should not have fallen back to the delay")
    .unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the delay was used instead of the status"
    );
    browser.close().await.unwrap();
}

/// Scripting off must still complete the ladder, and must actually be off.
#[tokio::test]
async fn scripting_can_be_disabled_and_the_ladder_still_completes() {
    let _turn = one_at_a_time().await;
    let Some(browser) = launch().await else {
        return;
    };
    let server = TestServer::start().await;
    let page = browser.new_page().await.unwrap();

    let web = WebSettings {
        javascript: false,
        ..WebSettings::default()
    };
    page.prepare(&web).await.unwrap();

    let progress = Progress::new();
    page.load(
        &server.url("/scripted"),
        &LoadSettings::default(),
        &progress,
    )
    .await
    .unwrap();
    assert_eq!(progress.current(), Stage::Settled);

    // The script would have rewritten this had it been allowed to run.
    let text = page
        .session()
        .send(
            "Runtime.evaluate",
            json!({
                "expression": "document.getElementById('t').textContent",
                "returnByValue": true,
            }),
        )
        .await
        .unwrap();
    assert_eq!(text["result"]["value"], "before");

    browser.close().await.unwrap();
}
