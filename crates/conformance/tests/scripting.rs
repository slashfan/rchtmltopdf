//! The two options that put something of the user's own into the page.
//!
//! Both are asserted through the page count: a stylesheet or a script that makes
//! the document two pages instead of one. That measures the effect rather than
//! the mechanism, which matters here more than usual — injecting a stylesheet
//! and having it apply are different things, and so are running a script and
//! waiting for it.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::{require_chromium, server};
use std::path::{Path, PathBuf};

/// A document with a block that is nothing until something gives it a height.
const BODY: &str = "<div id=\"pad\"></div><p>after</p>";

const TALL: &str = "#pad { height: 400mm; }";

fn page(scratch: &Scratch) -> PathBuf {
    fixture::write(scratch.path(), "page.html", BODY)
}

fn convert(page: &Path, options: &[&str]) -> Pdf {
    let outcome = Run::new()
        .args(options.iter().copied())
        .arg(page.display().to_string())
        .arg("-")
        .output();
    outcome.succeeded();
    Pdf::from_bytes(&outcome.stdout)
}

#[test]
fn a_user_stylesheet_named_by_path_is_applied() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("user-stylesheet-path");
    let page = page(&scratch);
    let sheet = scratch.join("user.css");
    std::fs::write(&sheet, TALL).expect("the stylesheet should be writable");

    assert_eq!(convert(&page, &[]).page_count(), 1);
    let styled = convert(&page, &["--user-style-sheet", &sheet.display().to_string()]);
    assert_eq!(
        styled.page_count(),
        2,
        "the stylesheet should have applied: {}",
        styled.describe()
    );
}

/// The same option, given a URL instead. The browser fetches this one, and it
/// has to be in the document before the network is judged idle or the page is
/// printed while the stylesheet is still arriving.
#[test]
fn a_user_stylesheet_named_by_url_is_fetched_and_waited_for() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("user-stylesheet-url");
    let page = page(&scratch);
    let server = server::Server::start();

    let styled = convert(
        &page,
        &["--user-style-sheet", &server.url(server::STYLESHEET)],
    );
    assert_eq!(
        styled.page_count(),
        2,
        "the stylesheet should have arrived before printing: {}",
        styled.describe()
    );
}

/// A user stylesheet named on the command line is read by us, so the policy
/// that stops a *document* reading the disk has nothing to say about it — even
/// though the default is that a local document may read nothing beside it (D10).
#[test]
fn a_user_stylesheet_is_not_subject_to_the_local_file_policy() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("user-stylesheet-policy");
    let page = page(&scratch);
    let sheet = scratch.join("user.css");
    std::fs::write(&sheet, TALL).expect("the stylesheet should be writable");

    // No --enable-local-file-access, and no --allow.
    let styled = convert(&page, &["--user-style-sheet", &sheet.display().to_string()]);
    assert_eq!(styled.page_count(), 2, "{}", styled.describe());
}

/// Injection goes through the protocol rather than through the page, so turning
/// the page's own scripting off must not turn it off too.
#[test]
fn a_user_stylesheet_still_applies_with_javascript_disabled() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("user-stylesheet-nojs");
    let page = page(&scratch);
    let sheet = scratch.join("user.css");
    std::fs::write(&sheet, TALL).expect("the stylesheet should be writable");

    let styled = convert(
        &page,
        &[
            "--disable-javascript",
            "--user-style-sheet",
            &sheet.display().to_string(),
        ],
    );
    assert_eq!(
        styled.page_count(),
        2,
        "the page's scripting is off, not ours: {}",
        styled.describe()
    );
}

/// Two scripts, where the second only works if the first has already run. Order
/// is the whole assertion: run them the other way round and the page is one.
#[test]
fn scripts_run_in_the_order_they_were_written() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("run-script-order");
    let page = page(&scratch);

    let ran = convert(
        &page,
        &[
            "--run-script",
            "window.wanted = '400mm'",
            "--run-script",
            "document.getElementById('pad').style.height = window.wanted",
        ],
    );
    assert_eq!(
        ran.page_count(),
        2,
        "both scripts should have run, in order: {}",
        ran.describe()
    );
}

/// A script that returns a promise is awaited, not started and abandoned. The
/// height arrives a third of a second late, which is longer than the JavaScript
/// delay the page has already served out.
#[test]
fn a_script_that_returns_a_promise_is_waited_for() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("run-script-promise");
    let page = page(&scratch);

    let awaited = convert(
        &page,
        &[
            "--run-script",
            "new Promise(resolve => setTimeout(() => { \
             document.getElementById('pad').style.height = '400mm'; resolve(); }, 300))",
        ],
    );
    assert_eq!(
        awaited.page_count(),
        2,
        "the promise should have been awaited: {}",
        awaited.describe()
    );
}

/// A throw is not a protocol error: the command succeeds and the throw is a
/// field in the reply. Ignoring that field is how a script that fails every time
/// looks like one that works, so this asserts the pair KnpSnappy reads — a
/// non-zero exit and something on stderr — and that no document is left behind.
#[test]
fn a_script_that_throws_fails_the_conversion_and_names_itself() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("run-script-throws");
    let page = page(&scratch);
    let output = scratch.join("out.pdf");

    let outcome = Run::new()
        .arg("--run-script")
        .arg("throw new Error('deliberate')")
        .arg(page.display().to_string())
        .arg(output.display().to_string())
        .output();
    outcome.failed();

    assert!(
        outcome.stderr.contains("deliberate"),
        "should carry what was thrown:\n{}",
        outcome.stderr
    );
    assert!(
        outcome.stderr.contains("throw new Error"),
        "should name the script, since a command line usually carries several:\n{}",
        outcome.stderr
    );
    assert!(
        !output.exists(),
        "a failed conversion must leave no document behind"
    );
}

/// A stylesheet that is not there was named on the command line, so it is a
/// mistake to report rather than a subresource to shrug at.
#[test]
fn a_user_stylesheet_that_is_missing_is_reported() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("user-stylesheet-missing");
    let page = page(&scratch);
    let missing = scratch.join("not-here.css");

    let outcome = Run::new()
        .arg("--user-style-sheet")
        .arg(missing.display().to_string())
        .arg(page.display().to_string())
        .arg("-")
        .output();
    outcome.failed();
    assert!(
        outcome.stderr.contains("not-here.css"),
        "should name the file:\n{}",
        outcome.stderr
    );
}
