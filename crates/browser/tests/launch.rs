//! Launching a real browser.
//!
//! These need a browser on the machine. Without one they print a line and pass,
//! so `cargo test` works on a laptop that has none. Setting
//! `RCHTMLTOPDF_REQUIRE_CHROMIUM` turns that skip into a failure, which is what
//! CI does, so a pinned browser quietly going missing is caught rather than
//! silently skipped forever.

use rchtmltopdf_browser::locate::{Executable, SystemEnvironment, locate};
use rchtmltopdf_browser::{Browser, LaunchOptions};
use serde_json::{Value, json};

/// How to launch here.
///
/// The sandbox is not available everywhere. A GitHub runner refuses it outright,
/// and so does a default Docker container, which is the single most common thing
/// people hit when moving a deployment into one. Where that is true the
/// environment says so and these tests follow, rather than pretending the
/// sandbox works and failing.
///
/// It stays on by default, because on a developer machine it does work and that
/// is the path worth exercising. It is never turned on automatically in the
/// product (D10): an unsandboxed browser rendering untrusted HTML is the thing
/// being protected against.
fn options() -> LaunchOptions {
    LaunchOptions {
        no_sandbox: std::env::var_os("RCHTMLTOPDF_TEST_NO_SANDBOX").is_some(),
        // A shared CI runner starting a browser cold is far slower than a
        // developer machine. The default is a backstop for real use, not a
        // statement about how fast a loaded runner ought to be.
        handshake_timeout: Some(std::time::Duration::from_secs(60)),
        ..LaunchOptions::default()
    }
}

/// Starting a browser is expensive. Running several of these at once on a small
/// runner makes each of them slow enough to look broken, so they take turns.
/// Concurrency *within* a test is unaffected, which is what the two-browser test
/// is actually about.
async fn one_at_a_time() -> tokio::sync::SemaphorePermit<'static> {
    static TURN: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    TURN.acquire().await.expect("the semaphore is never closed")
}

fn browser_or_skip() -> Option<Executable> {
    match locate(None, &SystemEnvironment) {
        Ok(executable) => Some(executable),
        Err(error) => {
            if std::env::var_os("RCHTMLTOPDF_REQUIRE_CHROMIUM").is_some() {
                panic!("RCHTMLTOPDF_REQUIRE_CHROMIUM is set but no browser was found:\n{error}");
            }
            eprintln!("skipping: no browser on this machine");
            None
        }
    }
}

/// The whole point of the descriptor plumbing: a browser that actually answers.
/// If the pipe were misplaced this would hang rather than fail, which is why the
/// launch has its own handshake timeout.
#[tokio::test]
async fn a_launched_browser_answers() {
    let _turn = one_at_a_time().await;
    let Some(executable) = browser_or_skip() else {
        return;
    };
    let browser = Browser::launch(&executable, &options())
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let version = browser
        .client()
        .send("Browser.getVersion", Value::Null)
        .await
        .unwrap();
    let product = version["product"].as_str().unwrap();
    println!("launched {product} from {}", executable.path.display());
    assert!(
        product.contains("Chrom") || product.contains("HeadlessChrome"),
        "unexpected product: {product}"
    );

    browser.close().await.unwrap();
}

#[tokio::test]
async fn a_page_can_be_opened_and_driven() {
    let _turn = one_at_a_time().await;
    let Some(executable) = browser_or_skip() else {
        return;
    };
    let browser = Browser::launch(&executable, &options()).await.unwrap();

    let page = browser.new_page().await.unwrap();
    assert!(!page.target_id().is_empty());

    // Round-trip through the session, which proves session routing works over a
    // real connection and not only against the stand-in.
    let evaluated = page
        .session()
        .send(
            "Runtime.evaluate",
            json!({ "expression": "6 * 7", "returnByValue": true }),
        )
        .await
        .unwrap();
    assert_eq!(evaluated["result"]["value"], 42);

    browser.close().await.unwrap();
}

/// Two browsers at once must not collide. This is what the pipe transport buys
/// over a debugging port, where concurrent conversions race for a number.
#[tokio::test]
async fn two_browsers_can_run_at_the_same_time() {
    let _turn = one_at_a_time().await;
    let Some(executable) = browser_or_skip() else {
        return;
    };
    let first = Browser::launch(&executable, &options()).await.unwrap();
    let second = Browser::launch(&executable, &options()).await.unwrap();

    for browser in [&first, &second] {
        let page = browser.new_page().await.unwrap();
        let evaluated = page
            .session()
            .send(
                "Runtime.evaluate",
                json!({ "expression": "1", "returnByValue": true }),
            )
            .await
            .unwrap();
        assert_eq!(evaluated["result"]["value"], 1);
    }

    first.close().await.unwrap();
    second.close().await.unwrap();
}

/// Dropping must leave nothing behind: no process, no profile directory.
#[tokio::test]
async fn dropping_a_browser_cleans_up_after_itself() {
    let _turn = one_at_a_time().await;
    let Some(executable) = browser_or_skip() else {
        return;
    };
    let browser = Browser::launch(&executable, &options()).await.unwrap();
    let pid = format!("{browser:?}");
    drop(browser);

    // Give the kill a moment to land.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    println!("dropped {pid}");

    let leftovers = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("rchtmltopdf-{}-", std::process::id()))
        })
        .count();
    assert_eq!(
        leftovers, 0,
        "a temporary profile directory was left behind"
    );
}

/// A path that is not a browser must fail with the program's own output, not
/// hang waiting for a handshake that will never come.
#[tokio::test]
async fn launching_something_that_is_not_a_browser_fails_with_its_output() {
    let not_a_browser = Executable {
        path: "/bin/echo".into(),
        origin: rchtmltopdf_browser::Origin::Flag,
        flavour: rchtmltopdf_browser::Flavour::HeadlessShell,
    };

    let error = Browser::launch(&not_a_browser, &options())
        .await
        .expect_err("echo is not a browser");
    let message = error.to_string();
    assert!(message.contains("failed to start"), "{message}");
}

/// The descriptor plumbing on its own, with no browser involved.
///
/// A shell that copies descriptor 3 to descriptor 4 stands in for Chromium. If
/// this passes and a real launch does not, the problem is the browser or its
/// flags, not the pipe.
#[cfg(unix)]
#[tokio::test]
async fn descriptors_three_and_four_reach_the_child() {
    use rchtmltopdf_browser::launch::testing::spawn_with_pipe;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut child, mut to_child, mut from_child) =
        spawn_with_pipe("/bin/sh", &["-c", "cat <&3 >&4"]).unwrap();

    to_child.write_all(b"ping\0").await.unwrap();
    to_child.flush().await.unwrap();

    let mut buffer = [0u8; 5];
    from_child.read_exact(&mut buffer).await.unwrap();
    assert_eq!(&buffer, b"ping\0");

    let _ = child.start_kill();
}
