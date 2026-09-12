//! Several documents on one command line, combined into one PDF (#36).
//!
//! The unit tests in `crates/pdf` prove the merge on documents built by hand.
//! What they cannot prove is that it survives **Chromium's** output — object
//! streams, cross-reference streams, a subset font per document — and that
//! each document was printed with its own options rather than the first one's.
//! Both are asserted here, on the real thing.
//!
//! Every assertion is per page: a sentinel is on the page its document was
//! given at, and on no other. `text()` over the whole file would pass with the
//! documents in any order.

use rchtmltopdf_conformance::binary::{Run, is_pdf};
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::require_chromium;
use rchtmltopdf_conformance::server::Server;

/// Two pages, by an explicit break.
const TWO_PAGES: &str = "<div style=\"page-break-after:always\">ALPHA</div><div>BETA</div>";
const ONE_PAGE: &str = "<div>GAMMA</div>";

/// A4, in points. The media box lands on Chromium's grid, so `is_about`.
const A4: (f64, f64) = (595.276, 841.89);

fn fixtures(scratch: &Scratch) -> (String, String) {
    let a = fixture::write(scratch.path(), "a.html", TWO_PAGES);
    let b = fixture::write(scratch.path(), "b.html", ONE_PAGE);
    (a.display().to_string(), b.display().to_string())
}

#[test]
fn documents_are_combined_in_command_line_order() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("merge-order");
    let (a, b) = fixtures(&scratch);

    let outcome = Run::new().arg(&a).arg(&b).arg("-").output();
    outcome.succeeded();
    let pdf = Pdf::from_bytes(&outcome.stdout);

    assert_eq!(pdf.page_count(), 3, "{}", pdf.describe());
    assert!(pdf.page_text(1).contains("ALPHA"));
    assert!(pdf.page_text(2).contains("BETA"));
    assert!(pdf.page_text(3).contains("GAMMA"));
    assert!(
        !pdf.page_text(3).contains("ALPHA"),
        "the first document's text turned up on the second's page"
    );

    // The paper survived the move: every page still knows its own size once
    // the tree it inherited from is gone.
    for page in 1..=3 {
        let paper = pdf.media_box(page);
        assert!(paper.is_about(A4.0, A4.1), "page {page} is {paper:?}");
    }
}

/// The reverse order gives the reverse document, which is what proves the
/// order comes from the command line rather than from anything in the files.
#[test]
fn the_order_is_the_command_lines() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("merge-reversed");
    let (a, b) = fixtures(&scratch);

    let outcome = Run::new().arg(&b).arg(&a).arg("-").output();
    outcome.succeeded();
    let pdf = Pdf::from_bytes(&outcome.stdout);

    assert_eq!(pdf.page_count(), 3);
    assert!(pdf.page_text(1).contains("GAMMA"));
    assert!(pdf.page_text(2).contains("ALPHA"));
}

/// **Each document is printed with its own options.** A footer given after
/// the first input belongs to it alone, and one given after the second to the
/// second. Printing everything with the first document's options would put
/// the first footer on every page.
#[test]
fn each_document_is_printed_with_its_own_options() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("merge-own-options");
    let (a, b) = fixtures(&scratch);

    let outcome = Run::new()
        .arg(&a)
        .arg("--footer-center")
        .arg("FIRSTBAND")
        .arg(&b)
        .arg("--footer-center")
        .arg("SECONDBAND")
        .arg("-")
        .output();
    outcome.succeeded();
    let pdf = Pdf::from_bytes(&outcome.stdout);

    assert_eq!(pdf.page_count(), 3);
    for page in 1..=2 {
        let text = pdf.page_text(page);
        assert!(text.contains("FIRSTBAND"), "page {page}: {text}");
        assert!(!text.contains("SECONDBAND"), "page {page}: {text}");
    }
    let last = pdf.page_text(3);
    assert!(last.contains("SECONDBAND"), "{last}");
    assert!(!last.contains("FIRSTBAND"), "{last}");
}

/// Some options are decided on the browser's command line rather than over the
/// protocol, so a document that differs in one needs a browser of its own. The
/// conversion restarts one rather than printing the document wrong.
#[test]
fn a_document_that_needs_a_different_browser_gets_one() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("merge-relaunch");
    let (a, b) = fixtures(&scratch);

    let outcome = Run::new()
        .arg(&a)
        .arg(&b)
        .arg("--no-images")
        .arg("-")
        .output();
    outcome.succeeded();
    let pdf = Pdf::from_bytes(&outcome.stdout);

    assert_eq!(pdf.page_count(), 3);
    assert!(pdf.page_text(1).contains("ALPHA"));
    assert!(pdf.page_text(3).contains("GAMMA"));
}

/// wkhtmltopdf's output carried the first document's title, and `--title`
/// still wins over it.
#[test]
fn the_title_is_the_first_documents_unless_given() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("merge-title");
    let first =
        fixture::document(ONE_PAGE).replace("<title>conformance</title>", "<title>First</title>");
    let second =
        fixture::document(ONE_PAGE).replace("<title>conformance</title>", "<title>Second</title>");
    assert!(
        first.contains("<title>First</title>"),
        "the fixture's title moved"
    );
    let a = scratch.join("a.html");
    let b = scratch.join("b.html");
    std::fs::write(&a, first).expect("writable");
    std::fs::write(&b, second).expect("writable");
    let (a, b) = (a.display().to_string(), b.display().to_string());

    let own = Run::new().arg(&a).arg(&b).arg("-").output();
    own.succeeded();
    assert_eq!(
        Pdf::from_bytes(&own.stdout).info("Title").as_deref(),
        Some("First")
    );

    let given = Run::new()
        .arg("--title")
        .arg("Given")
        .arg(&a)
        .arg(&b)
        .arg("-")
        .output();
    given.succeeded();
    assert_eq!(
        Pdf::from_bytes(&given.stdout).info("Title").as_deref(),
        Some("Given")
    );
}

/// `--load-error-handling skip` finally means what it says: the document that
/// failed is dropped, the others are converted, and the exit code is 0 (D14).
/// The default, `abort`, still means exit 1 and no PDF.
#[test]
fn skip_drops_the_document_that_failed_and_keeps_the_rest() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = Server::start();
    let scratch = Scratch::new("merge-skip");
    let (a, b) = fixtures(&scratch);
    let missing = server.url("/definitely-not-here");

    let skipped = Run::new()
        .arg("--load-error-handling")
        .arg("skip")
        .arg(&a)
        .arg(&missing)
        .arg(&b)
        .arg("-")
        .output();
    skipped.succeeded();
    let pdf = Pdf::from_bytes(&skipped.stdout);
    assert_eq!(pdf.page_count(), 3, "{}", pdf.describe());
    assert!(pdf.page_text(1).contains("ALPHA"));
    assert!(pdf.page_text(3).contains("GAMMA"));
    assert!(
        skipped.stderr.contains("skipped") && skipped.stderr.contains("definitely-not-here"),
        "skip still says what it skipped:\n{}",
        skipped.stderr
    );

    let aborted = Run::new().arg(&a).arg(&missing).arg(&b).arg("-").output();
    aborted.failed();
    assert!(
        !is_pdf(&aborted.stdout),
        "abort must leave no document behind"
    );
    assert!(
        aborted.stderr.contains("ContentNotFoundError"),
        "{}",
        aborted.stderr
    );
}

/// When every document was skipped there is nothing to write, and saying so
/// beats writing an empty file.
#[test]
fn skipping_every_document_is_a_failure_that_says_so() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = Server::start();

    let outcome = Run::new()
        .arg("--load-error-handling")
        .arg("skip")
        .arg(server.url("/nope-one"))
        .arg(server.url("/nope-two"))
        .arg("-")
        .output();
    outcome.failed();
    assert!(outcome.stdout.is_empty(), "nothing should be written");
    assert!(
        outcome.stderr.contains("nothing to convert")
            && outcome.stderr.contains("nope-one")
            && outcome.stderr.contains("nope-two"),
        "{}",
        outcome.stderr
    );
}
