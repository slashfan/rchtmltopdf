//! Every shape the brief promises for V0, and the ways it is allowed to fail.
//!
//! A URL, a local file and standard input, going to a file or to standard
//! output. Assertions are structural, never pixel (D15): what comes back has to
//! be a PDF, and on failure there must be no PDF at all.

use rchtmltopdf_conformance::binary::{Run, is_pdf};
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::require_chromium;
use rchtmltopdf_conformance::server::{self, Server};

fn pdf_on_disk(path: &std::path::Path) -> Vec<u8> {
    let bytes = std::fs::read(path).expect("a converted document should be on disk");
    assert!(
        is_pdf(&bytes),
        "not a PDF, first bytes were {:?}",
        String::from_utf8_lossy(&bytes[..bytes.len().min(32)])
    );
    bytes
}

// --- the four shapes ---------------------------------------------------------

#[test]
fn a_local_file_becomes_a_pdf_on_disk() {
    let Some(browser) = require_chromium() else {
        return;
    };
    eprintln!("using {} from {}", browser.path.display(), browser.origin);

    let scratch = Scratch::new("local-file");
    let page = fixture::write(scratch.path(), "page.html", "<h1>hello</h1>");
    let output = scratch.join("out.pdf");

    Run::new()
        .arg(page.display().to_string())
        .arg(output.display().to_string())
        .output()
        .succeeded();

    pdf_on_disk(&output);
}

#[test]
fn a_url_becomes_a_pdf() {
    let Some(_browser) = require_chromium() else {
        return;
    };

    let server = Server::start();
    let scratch = Scratch::new("url");
    let output = scratch.join("out.pdf");

    Run::new()
        .arg(server.url(server::PAGE))
        .arg(output.display().to_string())
        .output()
        .succeeded();

    // Proving it fetched *that* URL, rather than producing a plausible blank
    // page from somewhere else.
    let pdf = Pdf::from_bytes(&pdf_on_disk(&output));
    assert!(
        pdf.text().contains(server::SENTINEL),
        "the served document is not what was printed: {:?}",
        pdf.text()
    );
}

#[test]
fn standard_input_becomes_a_pdf() {
    let Some(_browser) = require_chromium() else {
        return;
    };

    let scratch = Scratch::new("stdin");
    let output = scratch.join("out.pdf");

    Run::new()
        .stdin(fixture::document("<p>FROM-STANDARD-INPUT</p>"))
        .arg("-")
        .arg(output.display().to_string())
        .output()
        .succeeded();

    let pdf = Pdf::from_bytes(&pdf_on_disk(&output));
    assert!(
        pdf.text().contains("FROM-STANDARD-INPUT"),
        "{:?}",
        pdf.text()
    );
}

/// The shape a web application uses: nothing touches the filesystem.
#[test]
fn the_document_can_go_to_standard_output() {
    let Some(_browser) = require_chromium() else {
        return;
    };

    let outcome = Run::new()
        .stdin(fixture::document("<p>THROUGH-THE-PIPE</p>"))
        .arg("-")
        .arg("-")
        .output();
    outcome.succeeded();

    assert!(
        is_pdf(&outcome.stdout),
        "stdout should be the PDF and nothing else, got {:?}",
        String::from_utf8_lossy(&outcome.stdout[..outcome.stdout.len().min(64)])
    );
    let pdf = Pdf::from_bytes(&outcome.stdout);
    assert!(pdf.text().contains("THROUGH-THE-PIPE"), "{:?}", pdf.text());
}

// --- and the ways it fails ---------------------------------------------------

/// The pair that matters: no PDF **and** a non-zero exit with something on
/// stderr (D14). A conversion that fails and leaves a plausible-looking file
/// behind is worse than one that fails loudly, because a billing pipeline will
/// happily post it.
#[test]
fn a_document_that_cannot_be_read_writes_no_pdf() {
    let Some(_browser) = require_chromium() else {
        return;
    };

    let scratch = Scratch::new("missing-input");
    let output = scratch.join("out.pdf");

    Run::new()
        .arg(scratch.join("no-such-page.html").display().to_string())
        .arg(output.display().to_string())
        .output()
        .failed();

    assert!(
        !output.exists(),
        "a failed conversion must not leave a file behind"
    );
}

/// The bug that #57 was filed for: an unreachable URL used to report success.
#[test]
fn an_unreachable_url_fails_and_writes_no_pdf() {
    let Some(_browser) = require_chromium() else {
        return;
    };

    let scratch = Scratch::new("unreachable");
    let output = scratch.join("out.pdf");

    Run::new()
        .arg(server::unreachable_url())
        .arg(output.display().to_string())
        .output()
        .failed();

    assert!(!output.exists(), "an error page is not a document");
}

/// A page that never finishes has to be cut off, and the browser has to go with
/// it (D16). A leaked Chromium holds memory for ever and, in a queue worker,
/// accumulates one per job.
///
/// The leak check is the profile directory rather than a process scan. Each
/// conversion creates `rchtmltopdf-<pid>-<n>-<nanos>` under the temporary
/// directory and removes it on `Drop`, so a name that survives is proof that a
/// browser was never torn down — and because the names are unique, it cannot be
/// confused with another test's browser or with a Chrome the developer happens
/// to have open.
#[test]
fn a_hanging_page_stops_at_the_deadline_and_leaves_nothing_behind() {
    let Some(_browser) = require_chromium() else {
        return;
    };

    let server = Server::start();
    let scratch = Scratch::new("timeout");
    let output = scratch.join("out.pdf");

    let before = profile_directories();
    let outcome = Run::new()
        .args(["--timeout", "1"])
        .arg(server.url(server::HANG))
        .arg(output.display().to_string())
        .output();
    outcome.failed();

    assert!(
        outcome.stderr.to_lowercase().contains("timed out")
            || outcome.stderr.to_lowercase().contains("timeout"),
        "the failure should name the deadline: {:?}",
        outcome.stderr
    );
    assert!(!output.exists(), "a timeout must not write a partial PDF");

    let leaked: Vec<_> = profile_directories().difference(&before).cloned().collect();
    assert!(
        leaked.is_empty(),
        "a browser profile outlived the conversion: {leaked:?}"
    );
}

/// A browser profile is `rchtmltopdf-<pid>-<counter>-<nanos>`, so the segment
/// after the prefix is a number.
///
/// Checking the prefix alone is not enough, and the first run of this test
/// proved it by reporting a leak that was this suite's own
/// `rchtmltopdf-conformance-url-14174` scratch directory, created by a test
/// running beside it.
fn is_browser_profile(name: &str) -> bool {
    name.strip_prefix("rchtmltopdf-")
        .and_then(|rest| rest.split('-').next())
        .is_some_and(|segment| !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()))
}

fn profile_directories() -> std::collections::HashSet<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return std::collections::HashSet::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_browser_profile)
        })
        .collect()
}
