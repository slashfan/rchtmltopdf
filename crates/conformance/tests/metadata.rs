//! What the finished file says about itself.
//!
//! # Why this needs a real browser
//!
//! The unit tests in `crates/pdf` build a document by hand, which proves the
//! read-modify-write path but not that it survives **Chromium's** output.
//! Chromium writes cross-reference streams and object streams, and a library
//! that re-serialises those wrongly produces a file that still starts `%PDF-`
//! and opens in nothing. So the round trip is asserted here, on the real thing.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::require_chromium;

/// Three pages, by explicit breaks, with something to find on each.
const THREE_PAGES: &str = "<div style=\"page-break-after:always\">ALPHA</div>\
                           <div style=\"page-break-after:always\">BETA</div>\
                           <div>GAMMA</div>";

fn convert(scratch: &Scratch, options: &[&str]) -> Pdf {
    let page = fixture::write(scratch.path(), "p.html", THREE_PAGES);
    let outcome = Run::new()
        .args(options.iter().copied())
        .arg(page.display().to_string())
        .arg("-")
        .output();
    outcome.succeeded();
    Pdf::from_bytes(&outcome.stdout)
}

/// The print call takes the title from the document's own `<title>` and offers
/// no override, so this is the only way `--title` can mean anything.
#[test]
fn title_sets_the_documents_title() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("metadata-title");

    let pdf = convert(&scratch, &["--title", "Invoice 42"]);
    assert_eq!(pdf.info("Title").as_deref(), Some("Invoice 42"));
}

/// **Without `--title`, the document keeps its own.** The print call derives it
/// from the `<title>` element, and for most documents that is the only title
/// there will ever be — throwing it away to write a producer would be a worse
/// trade than not writing one.
///
/// Every fixture here carries `<title>conformance</title>`.
#[test]
fn a_document_without_the_option_keeps_the_title_it_had() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("metadata-own-title");

    let pdf = convert(&scratch, &[]);
    assert_eq!(pdf.info("Title").as_deref(), Some("conformance"));
}

/// **A document with no `<title>` element leaves the file's title empty**
/// (D52). wkhtmltopdf 0.12.6.1 writes an empty one — `pdfinfo` prints a bare
/// `Title:` — where Chromium writes the file name, which is not a title
/// anybody gave the document.
#[test]
fn a_document_with_no_title_element_gives_the_file_an_empty_one() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("metadata-untitled");
    let page = scratch.join("untitled.html");
    std::fs::write(
        &page,
        fixture::document(THREE_PAGES).replace("<title>conformance</title>", ""),
    )
    .expect("writable");
    let outcome = Run::new().arg(page.display().to_string()).arg("-").output();
    outcome.succeeded();
    let pdf = Pdf::from_bytes(&outcome.stdout);
    assert_eq!(pdf.info("Title").as_deref(), Some(""));
}

/// The old dictionary is edited rather than orphaned, so nothing left in the
/// file still claims the browser made it. A reader follows the trailer and would
/// never notice; anybody looking at the bytes would.
#[test]
fn nothing_in_the_file_still_names_the_browser_as_the_producer() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("metadata-no-orphan");
    let page = fixture::write(scratch.path(), "p.html", THREE_PAGES);

    let outcome = Run::new().arg(page.display().to_string()).arg("-").output();
    outcome.succeeded();
    assert!(
        !String::from_utf8_lossy(&outcome.stdout).contains("Skia/PDF"),
        "Chromium's own Info dictionary was left in the file"
    );
}

/// Written as UTF-16 with a byte order mark, because a PDF string without one is
/// Latin-1 and `Facture n°42` is not. Invoices in French are the ordinary case.
#[test]
fn a_title_outside_ascii_survives_the_file_format() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("metadata-accents");

    let pdf = convert(&scratch, &["--title", "Facture n°42 — Février"]);
    assert_eq!(pdf.info("Title").as_deref(), Some("Facture n°42 — Février"));
}

/// The producer is this program, not the browser it drove. Anybody opening the
/// file and asking what made it should get an answer that can be acted on.
#[test]
fn the_producer_names_this_program_and_its_version() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("metadata-producer");

    let pdf = convert(&scratch, &[]);
    let producer = pdf.info("Producer").unwrap_or_default();
    assert!(producer.starts_with("rchtmltopdf "), "{producer:?}");
    assert!(
        producer.chars().any(|c| c.is_ascii_digit()),
        "should carry a version: {producer:?}"
    );
}

/// `D:YYYYMMDDHHmmSSOHH'mm'`, which is the only date syntax a PDF has. The
/// trailing apostrophe is not optional, and readers that validate reject the
/// date without it.
#[test]
fn the_creation_date_is_a_valid_pdf_date() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("metadata-date");

    let date = convert(&scratch, &[])
        .info("CreationDate")
        .expect("a document should be dated");

    assert!(date.starts_with("D:"), "{date:?}");
    assert!(date.ends_with('\''), "{date:?}");
    // D: plus fourteen digits, then the zone.
    let digits: String = date[2..16].to_string();
    assert!(
        digits.chars().all(|c| c.is_ascii_digit()),
        "{date:?} should start with fourteen digits"
    );
    let year: i32 = digits[..4].parse().expect("a year");
    assert!(year >= 2024, "{date:?}");
}

/// **The one that matters for V2.** Rewriting the file must not change a thing
/// about the document, and Chromium's output is the shape that finds out — the
/// hand-built fixture in `crates/pdf` cannot.
#[test]
fn rewriting_a_chromium_pdf_does_not_change_the_document() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("metadata-round-trip");

    let pdf = convert(&scratch, &["--title", "Round trip"]);
    assert_eq!(pdf.page_count(), 3, "{}", pdf.describe());

    let text = pdf.text();
    for mark in ["ALPHA", "BETA", "GAMMA"] {
        assert!(text.contains(mark), "{mark} missing from {text:?}");
    }

    // Every page keeps its paper, which is what a mangled page tree loses first.
    let first = pdf.media_box(1);
    for page in 2..=3 {
        let box_ = pdf.media_box(page);
        assert!(
            box_.is_about(first.width(), first.height()),
            "page {page} lost its paper: {}",
            pdf.describe()
        );
    }
}
