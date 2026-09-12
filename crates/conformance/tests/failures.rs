//! What happens when something does not load, and what the process says about it
//! (D14).
//!
//! KnpSnappy raises when the exit code is non-zero **and** stderr is non-empty.
//! It is the pair that matters, so every case here asserts both, and the error
//! names are the ones an application greps for.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::{require_chromium, server};

/// A page that is not there at all. A 404 is not a failure to Chromium — the
/// bytes came back — and it was one to Qt, which is where the name comes from.
#[test]
fn a_missing_document_is_a_content_not_found_error() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();

    let outcome = Run::new()
        .arg(server.url("/definitely-not-here"))
        .arg("-")
        .output();
    outcome.failed();
    assert!(
        outcome.stderr.contains("ContentNotFoundError"),
        "applications grep for this name:\n{}",
        outcome.stderr
    );
    assert!(
        outcome.stdout.is_empty(),
        "a failed document must leave no PDF (D14)"
    );
}

/// A host that does not resolve, which is the other name applications branch on.
#[test]
fn a_host_that_does_not_resolve_is_a_host_not_found_error() {
    let Some(_browser) = require_chromium() else {
        return;
    };

    let outcome = Run::new()
        .arg("http://conformance.invalid/anything")
        .arg("-")
        .output();
    outcome.failed();
    assert!(
        outcome.stderr.contains("HostNotFoundError"),
        "{}",
        outcome.stderr
    );
}

/// `--load-error-handling ignore` prints whatever did load, which for a 404 is
/// the server's own error page. Faithful rather than useful, and it is the
/// option's documented meaning.
#[test]
fn ignore_prints_the_document_that_did_load() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();

    let outcome = Run::new()
        .arg("--load-error-handling")
        .arg("ignore")
        .arg(server.url("/definitely-not-here"))
        .arg("-")
        .output();
    outcome.succeeded();
    assert!(
        rchtmltopdf_conformance::binary::is_pdf(&outcome.stdout),
        "ignore should still produce a document"
    );
}

/// **The case D14 is most specific about.** A subresource that fails under
/// `abort` produces the PDF *and* exits 1, with a line applications grep for.
/// The default is `ignore`, so the same document converts cleanly without it.
#[test]
fn a_failed_subresource_is_judged_by_the_media_handler() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();
    let url = server.url(server::MISSING_MEDIA);

    // The default. A missing image is not a reason to fail a conversion.
    let ignored = Run::new().arg(&url).arg("-").output();
    ignored.succeeded();
    assert!(rchtmltopdf_conformance::binary::is_pdf(&ignored.stdout));

    // `abort`: the document is still written, and the exit code still says so.
    let aborted = Run::new()
        .arg("--load-media-error-handling")
        .arg("abort")
        .arg(&url)
        .arg("-")
        .output();
    aborted.failed();
    assert!(
        rchtmltopdf_conformance::binary::is_pdf(&aborted.stdout),
        "abort writes the PDF and exits 1; it does not throw the document away"
    );
    assert!(
        aborted
            .stderr
            .contains("Exit with code 1 due to network error: ContentNotFoundError"),
        "wkhtmltopdf's exact line, which applications grep for:\n{}",
        aborted.stderr
    );

    // `skip`: reported, and exit 0.
    let skipped = Run::new()
        .arg("--load-media-error-handling")
        .arg("skip")
        .arg(&url)
        .arg("-")
        .output();
    skipped.succeeded();
    assert!(
        skipped.stderr.contains("ContentNotFoundError"),
        "skip still says what it skipped:\n{}",
        skipped.stderr
    );
}

/// Chromium asks for a favicon on every navigation and wkhtmltopdf never did.
/// Counting it would fail a conversion under `abort` for a file the document
/// never mentioned.
#[test]
fn a_missing_favicon_is_not_a_failure() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();

    // `/page` has no favicon and does not ask for one; Chromium does anyway.
    Run::new()
        .arg("--load-media-error-handling")
        .arg("abort")
        .arg(server.url(server::PAGE))
        .arg("-")
        .output()
        .succeeded();
}

/// Progress goes to stderr, in wkhtmltopdf's shape, and `-q` stops it.
#[test]
fn progress_is_reported_and_quiet_silences_it() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();
    let url = server.url(server::PAGE);

    let noisy = Run::new().arg(&url).arg("-").output();
    noisy.succeeded();
    for line in ["Loading page (1/2)", "Printing pages (2/2)", "Done"] {
        assert!(
            noisy.stderr.contains(line),
            "{line:?} missing from:\n{}",
            noisy.stderr
        );
    }

    // Nothing on stderr at all, which is what a successful quiet run means to a
    // wrapper that reads the pair.
    let quiet = Run::new().arg("-q").arg(&url).arg("-").output();
    quiet.succeeded();
    assert!(
        quiet.stderr.trim().is_empty(),
        "-q should say nothing:\n{}",
        quiet.stderr
    );
}
