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
    page.prepare(&support::prepare(WebSettings::default()))
        .await
        .unwrap();

    let progress = Progress::new();
    page.load(&server.url("/plain"), &LoadSettings::default(), &progress)
        .await
        .unwrap();

    assert_eq!(progress.current(), Stage::Settled);
    browser.close().await.unwrap();
}

/// Progress exists so a conversion that gets stuck can say which rung it is on.
/// Asserting only the final rung proves nothing: the line above the assertion
/// sets it. This watches a slow load from outside and checks an intermediate
/// rung is actually reached and published.
#[tokio::test]
async fn the_rung_being_climbed_is_visible_from_outside() {
    let _turn = one_at_a_time().await;
    let Some(browser) = launch().await else {
        return;
    };
    let server = TestServer::start().await;
    let page = browser.new_page().await.unwrap();
    page.prepare(&support::prepare(WebSettings::default()))
        .await
        .unwrap();

    let progress = Progress::new();
    let watcher = {
        let progress = progress.clone();
        tokio::spawn(async move {
            let mut seen = Vec::new();
            // The stylesheet takes 700ms, so there is a wide window in which to
            // catch the rungs before the last one.
            for _ in 0..120 {
                let stage = progress.current();
                if !seen.contains(&stage) {
                    seen.push(stage);
                }
                if stage == Stage::Settled {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            seen
        })
    };

    page.load(
        &server.url("/slow-css"),
        &LoadSettings::default(),
        &progress,
    )
    .await
    .unwrap();

    let seen = watcher.await.unwrap();
    assert!(
        seen.contains(&Stage::AwaitingNetworkIdle),
        "never saw the network rung; only saw {seen:?}"
    );
    assert!(seen.len() > 1, "only ever saw one rung: {seen:?}");
    browser.close().await.unwrap();
}

/// A request whose connection is dropped is reported as failed, not finished.
/// The 404 route returns a body, so it completes normally and never reaches that
/// branch; this one hangs up instead.
#[tokio::test]
async fn a_request_that_fails_outright_does_not_hold_the_page_open() {
    let _turn = one_at_a_time().await;
    let Some(browser) = launch().await else {
        return;
    };
    let server = TestServer::start().await;
    let page = browser.new_page().await.unwrap();
    page.prepare(&support::prepare(WebSettings::default()))
        .await
        .unwrap();

    let settled = tokio::time::timeout(
        Duration::from_secs(15),
        page.load(
            &server.url("/broken-image"),
            &LoadSettings::default(),
            &Progress::new(),
        ),
    )
    .await;

    assert!(settled.is_ok(), "a failed request hung the wait");
    settled.unwrap().unwrap();
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
    page.prepare(&support::prepare(WebSettings::default()))
        .await
        .unwrap();

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
    page.prepare(&support::prepare(WebSettings::default()))
        .await
        .unwrap();

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
    page.prepare(&support::prepare(WebSettings::default()))
        .await
        .unwrap();

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

    // The status is set 400ms in, so anything faster means it was not waited
    // for at all. The upper bound is already implied by the timeout above; this
    // lower one is the assertion that matters.
    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "settled in {:?}, before the status could have been set",
        started.elapsed()
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
    page.prepare(&support::prepare(web.clone())).await.unwrap();

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

/// A dead address must fail, not quietly produce the browser's error page.
///
/// Before this was checked, the ladder returned success in under a second for a
/// URL that could not be reached, and printing would have produced a PDF of
/// Chromium's own "site can't be reached" screen.
///
/// The failure is **reported rather than raised** (D44): the load returns a
/// report naming the document's own request, and what happens next is
/// `--load-error-handling`'s to decide. What must not happen is a report that
/// says nothing, which is what would have the error page printed.
#[tokio::test]
async fn an_unreachable_url_is_reported_as_the_documents_own_failure() {
    let _turn = one_at_a_time().await;
    let Some(browser) = launch().await else {
        return;
    };
    let page = browser.new_page().await.unwrap();
    page.prepare(&support::prepare(WebSettings::default()))
        .await
        .unwrap();

    // Port 1 is reserved and nothing listens there.
    let outcome = page
        .load(
            "http://127.0.0.1:1/nothing",
            &LoadSettings::default(),
            &Progress::new(),
        )
        .await;

    let report = outcome.expect("a navigation failure is a report, not an error");
    let failed = report
        .document
        .expect("the document's own request should be named as failed");
    assert!(failed.url.contains("127.0.0.1:1"), "{}", failed.url);
    assert!(
        !failed.error.name().is_empty(),
        "the error should carry the name applications grep for"
    );

    browser.close().await.unwrap();
}

/// A page that emits events on a timer must still settle.
///
/// The quiet period used to be reset by any event at all, so anything ticking
/// faster than twice a second kept the page from ever being considered done.
/// Analytics and scroll handlers calling history.replaceState do exactly that.
#[tokio::test]
async fn a_page_that_ticks_still_settles() {
    let _turn = one_at_a_time().await;
    let Some(browser) = launch().await else {
        return;
    };
    let server = TestServer::start().await;
    let page = browser.new_page().await.unwrap();
    page.prepare(&support::prepare(WebSettings::default()))
        .await
        .unwrap();

    let settled = tokio::time::timeout(
        Duration::from_secs(15),
        page.load(
            &server.url("/ticking"),
            &LoadSettings::default(),
            &Progress::new(),
        ),
    )
    .await;

    assert!(
        settled.is_ok(),
        "a page emitting events on a timer never settled"
    );
    settled.unwrap().unwrap();
    browser.close().await.unwrap();
}
