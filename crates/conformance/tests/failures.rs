//! What happens when something does not load, and what the process says about it
//! (D14).
//!
//! KnpSnappy raises when the exit code is non-zero **and** stderr is non-empty.
//! It is the pair that matters, so every case here asserts both, and the error
//! names are the ones an application greps for.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::{require_chromium, server};

/// A host nothing resolves: `.invalid` is reserved for exactly this (RFC 2606),
/// so the failure is DNS rather than a connection nobody is listening for.
const UNRESOLVABLE: &str = "http://conformance.invalid/missing.html";

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

/// **A document that never loaded at all is not the same as one that loaded
/// badly** (D44). A 404 came back with a body the handlers can print; a host
/// that does not resolve came back with nothing, and until now that ended the
/// conversion whatever `--load-error-handling` said. Measured against
/// wkhtmltopdf 0.12.6.1: under `skip` the file is written from the documents
/// that did load, and the exit code is 1 all the same.
#[test]
fn skip_writes_the_other_documents_when_a_host_does_not_resolve() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("failures-skip-unresolvable");
    let first = fixture::write(scratch.path(), "first.html", "<p>first</p>");
    let third = fixture::write(scratch.path(), "third.html", "<p>third</p>");

    let outcome = Run::new()
        .arg("--load-error-handling")
        .arg("skip")
        .arg(first.display().to_string())
        .arg(UNRESOLVABLE)
        .arg(third.display().to_string())
        .arg("-")
        .output();

    outcome.failed();
    let pdf = Pdf::from_bytes(&outcome.stdout);
    assert_eq!(pdf.page_count(), 2, "{}", pdf.describe());
    assert!(pdf.text().contains("first"), "{:?}", pdf.text());
    assert!(pdf.text().contains("third"), "{:?}", pdf.text());
    assert!(
        outcome.stderr.contains("(skipped)"),
        "the skipped document should be named:\n{}",
        outcome.stderr
    );
    // The request's own line, which is what names the URL under every handler
    // and carries Qt's numbers for it (#114, D48). Nothing answered, so the
    // http status is zero and the network status is Qt's HostNotFoundError.
    assert!(
        outcome.stderr.contains(&format!(
            "Failed to load {UNRESOLVABLE}, with network status code 3 and http status code 0"
        )),
        "the line a script watching stderr finds the address in:\n{}",
        outcome.stderr
    );
    assert!(
        outcome
            .stderr
            .contains("Exit with code 1 due to network error: HostNotFoundError"),
        "the line applications grep for:\n{}",
        outcome.stderr
    );
}

/// And under `ignore` it keeps its place: wkhtmltopdf prints a blank page for
/// the document that did not load, so the one behind it is still page 3 of 3.
#[test]
fn ignore_leaves_a_blank_page_where_the_document_did_not_load() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("failures-ignore-unresolvable");
    let first = fixture::write(scratch.path(), "first.html", "<p>first</p>");
    let third = fixture::write(scratch.path(), "third.html", "<p>third</p>");

    let outcome = Run::new()
        .arg("--load-error-handling")
        .arg("ignore")
        .arg(first.display().to_string())
        .arg(UNRESOLVABLE)
        .arg(third.display().to_string())
        .arg("-")
        .output();

    outcome.failed();
    let pdf = Pdf::from_bytes(&outcome.stdout);
    assert_eq!(pdf.page_count(), 3, "{}", pdf.describe());
    assert!(pdf.page_text(1).contains("first"), "{:?}", pdf.page_text(1));
    assert!(
        pdf.page_text(2).trim().is_empty(),
        "the document that did not load leaves a blank page, not an error page: {:?}",
        pdf.page_text(2)
    );
    assert!(pdf.page_text(3).contains("third"), "{:?}", pdf.page_text(3));
}

/// `abort` is unchanged, and it is the default: nothing is written at all
/// (D14), whatever else loaded.
#[test]
fn abort_writes_nothing_when_a_host_does_not_resolve() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("failures-abort-unresolvable");
    let first = fixture::write(scratch.path(), "first.html", "<p>first</p>");

    let outcome = Run::new()
        .arg(first.display().to_string())
        .arg(UNRESOLVABLE)
        .arg("-")
        .output();

    outcome.failed();
    assert!(
        outcome.stdout.is_empty(),
        "a document that failed under abort must leave no PDF (D14)"
    );
    // Ending the conversion is not a reason to end it quietly: `abort` writes
    // the same two lines the other handlers do (#114).
    assert!(
        outcome
            .stderr
            .contains(&format!("Failed to load {UNRESOLVABLE},")),
        "abort names the request that failed:\n{}",
        outcome.stderr
    );
    assert!(
        outcome
            .stderr
            .contains("Exit with code 1 due to network error: HostNotFoundError"),
        "the line applications grep for, under abort as under the rest:\n{}",
        outcome.stderr
    );
}

/// **A document missing from the disk reads like one missing from the network**
/// (#114, D48). wkhtmltopdf fetched a local file through the same stack as a
/// URL, so a path that is not there ends with the exit line too. The name is
/// ours: wkhtmltopdf reports `HostNotFoundError` here only because it parses
/// the path into `http://<first segment>/…` first, and we will not copy that.
#[test]
fn a_document_that_is_not_on_disk_ends_with_the_exit_line() {
    let scratch = Scratch::new("failures-missing-file");
    let missing = scratch.join("there-is-no-such-file.html");

    let outcome = Run::new()
        .arg(missing.display().to_string())
        .arg("-")
        .output();

    outcome.failed();
    assert!(
        outcome.stderr.contains("no such file"),
        "it still says which file:\n{}",
        outcome.stderr
    );
    assert!(
        outcome
            .stderr
            .contains("Exit with code 1 due to network error: ContentNotFoundError"),
        "and ends with the line applications grep for:\n{}",
        outcome.stderr
    );
    assert!(
        outcome.stdout.is_empty(),
        "a document that could not be read must leave no PDF (D14)"
    );
}

/// **A document missing from the disk is the handler's to judge** (D55).
/// wkhtmltopdf fetched a local file through the same stack as a URL, so a
/// path that is not there was a failed load: under `skip` it wrote the other
/// documents, under `ignore` a blank page kept the missing one's place, and
/// both exited 1. We refused the path before starting anything, whatever the
/// handler said, and wrote nothing. The name stays ours (D48):
/// `ContentNotFoundError`, with Qt's code 203 on the request line.
#[test]
fn skip_drops_a_document_that_is_not_on_disk_and_writes_the_rest() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("failures-skip-missing-file");
    let first = fixture::write(scratch.path(), "first.html", "<p>first</p>");
    let missing = scratch.join("there-is-no-such-file.html");
    let third = fixture::write(scratch.path(), "third.html", "<p>third</p>");

    let outcome = Run::new()
        .arg("--load-error-handling")
        .arg("skip")
        .arg(first.display().to_string())
        .arg(missing.display().to_string())
        .arg(third.display().to_string())
        .arg("-")
        .output();

    outcome.failed();
    let pdf = Pdf::from_bytes(&outcome.stdout);
    assert_eq!(pdf.page_count(), 2, "{}", pdf.describe());
    assert!(pdf.text().contains("first"), "{:?}", pdf.text());
    assert!(pdf.text().contains("third"), "{:?}", pdf.text());
    assert!(
        outcome.stderr.contains("(skipped)"),
        "the skipped document should be named:\n{}",
        outcome.stderr
    );
    assert!(
        outcome.stderr.contains(
            "there-is-no-such-file.html, with network status code 203 and http status code 0"
        ),
        "the request line names the file it would have read:\n{}",
        outcome.stderr
    );
    assert!(
        outcome
            .stderr
            .contains("Exit with code 1 due to network error: ContentNotFoundError"),
        "the line applications grep for:\n{}",
        outcome.stderr
    );
}

/// And under `ignore` the missing document keeps its place as a blank page,
/// exactly as one whose host did not resolve (D44).
#[test]
fn ignore_leaves_a_blank_page_where_the_document_was_not_on_disk() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("failures-ignore-missing-file");
    let first = fixture::write(scratch.path(), "first.html", "<p>first</p>");
    let missing = scratch.join("there-is-no-such-file.html");
    let third = fixture::write(scratch.path(), "third.html", "<p>third</p>");

    let outcome = Run::new()
        .arg("--load-error-handling")
        .arg("ignore")
        .arg(first.display().to_string())
        .arg(missing.display().to_string())
        .arg(third.display().to_string())
        .arg("-")
        .output();

    outcome.failed();
    let pdf = Pdf::from_bytes(&outcome.stdout);
    assert_eq!(pdf.page_count(), 3, "{}", pdf.describe());
    assert!(pdf.page_text(1).contains("first"), "{:?}", pdf.page_text(1));
    assert!(
        pdf.page_text(2).trim().is_empty(),
        "a blank page, not an error page: {:?}",
        pdf.page_text(2)
    );
    assert!(pdf.page_text(3).contains("third"), "{:?}", pdf.page_text(3));
    assert!(
        outcome.stderr.contains("(ignored)")
            && outcome
                .stderr
                .contains("Exit with code 1 due to network error: ContentNotFoundError"),
        "{}",
        outcome.stderr
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

    // The default. A missing image is not a reason to fail a conversion — but
    // it is a reason to say so, and until #114 this said nothing at all. A
    // document that renders without its stylesheet is the hardest failure to
    // diagnose precisely because it renders.
    let ignored = Run::new().arg(&url).arg("-").output();
    ignored.succeeded();
    assert!(rchtmltopdf_conformance::binary::is_pdf(&ignored.stdout));
    assert!(
        ignored.stderr.contains(".png") && ignored.stderr.contains("was not loaded"),
        "ignore names the image it ignored, and says what happened to it:\n{}",
        ignored.stderr
    );

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

/// **Not every subresource is a media file** (D49). wkhtmltopdf sorted a failed
/// request by its extension: css, js, png, jpg, jpeg and gif went to
/// `--load-media-error-handling`, and anything else set the exit code whatever
/// either handler said. A frame's document is `.html`, so a frame that cannot
/// load is exit 1 under the most lenient pair of handlers there is, with the
/// PDF written. Measured against wkhtmltopdf 0.12.6.1 (#112).
#[test]
fn a_frame_that_does_not_resolve_is_a_network_error_under_every_handler() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("failures-frame-unresolvable");
    let page = fixture::write(
        scratch.path(),
        "frame.html",
        &format!("<p>before</p><iframe src=\"{UNRESOLVABLE}\"></iframe><p>after</p>"),
    );

    for handlers in [
        vec![],
        vec![
            "--load-error-handling",
            "ignore",
            "--load-media-error-handling",
            "ignore",
        ],
    ] {
        let outcome = Run::new()
            .args(handlers.iter().copied())
            .arg(page.display().to_string())
            .arg("-")
            .output();
        outcome.failed();
        assert!(
            rchtmltopdf_conformance::binary::is_pdf(&outcome.stdout),
            "the PDF is written all the same ({handlers:?})"
        );
        // The request's own line, as for a document (D48): the only one that
        // names the frame under every handler.
        assert!(
            outcome.stderr.contains(&format!(
                "Failed to load {UNRESOLVABLE}, with network status code 3 and http status code 0"
            )),
            "the line a script watching stderr finds the address in ({handlers:?}):\n{}",
            outcome.stderr
        );
        assert!(
            outcome
                .stderr
                .contains("Exit with code 1 due to network error: HostNotFoundError"),
            "the line applications grep for ({handlers:?}):\n{}",
            outcome.stderr
        );
    }
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
