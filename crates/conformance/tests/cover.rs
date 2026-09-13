//! The cover object (#37).
//!
//! "A cover objects puts the content of a single webpage into the output
//! document, the page does not appear in the table of contents, and does not
//! have headers and footers." The outline half waits for #40; the bands half
//! is asserted here, together with the consequence wkhtmltopdf's users rely
//! on: **a cover does not count**, so the first page after it is page one.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::require_chromium;

// A `div`, not a heading: a heading is bold, the vendored font has no bold
// face, and what Chromium draws for a synthesised bold is not extractable as
// text. See `fixtures/fonts/README.md`.
const COVER: &str = "<div>COVERTEXT</div>";
const TWO_PAGES: &str = "<div style=\"page-break-after:always\">ALPHA</div><div>BETA</div>";

fn fixtures(scratch: &Scratch) -> (String, String) {
    let cover = fixture::write(scratch.path(), "cover.html", COVER);
    let body = fixture::write(scratch.path(), "body.html", TWO_PAGES);
    (cover.display().to_string(), body.display().to_string())
}

/// A footer given as a default reaches the pages and not the cover, and the
/// cover's page counts all the same (D45): the page behind it is page two.
#[test]
fn a_cover_has_no_bands_and_counts_all_the_same() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("cover-bands");
    let (cover, body) = fixtures(&scratch);

    let outcome = Run::new()
        .arg("--footer-center")
        .arg("P[page]/[topage]")
        .arg("cover")
        .arg(&cover)
        .arg(&body)
        .arg("-")
        .output();
    outcome.succeeded();
    let pdf = Pdf::from_bytes(&outcome.stdout);

    assert_eq!(pdf.page_count(), 3, "{}", pdf.describe());
    let first = pdf.page_text(1);
    assert!(first.contains("COVERTEXT"), "{first}");
    assert!(!first.contains("P1"), "the cover got the footer: {first}");
    // The cover is page one, so the first page behind it is page two, and the
    // total counts all three. Whitespace is stripped because each number is
    // its own text run in the band, and extraction puts its own spacing
    // between runs.
    assert!(numbers(&pdf, 2).contains("P2/3"), "{}", pdf.page_text(2));
    assert!(numbers(&pdf, 3).contains("P3/3"), "{}", pdf.page_text(3));
}

/// A page's text with every space removed.
fn numbers(pdf: &Pdf, page: usize) -> String {
    pdf.page_text(page)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// "All options that can be specified for a page object can also be specified
/// for a cover." A band written after `cover` is the cover's own.
#[test]
fn a_band_written_after_cover_is_the_covers_own() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("cover-own-band");
    let (cover, body) = fixtures(&scratch);

    let outcome = Run::new()
        .arg("cover")
        .arg(&cover)
        .arg("--footer-center")
        .arg("COVERBAND")
        .arg(&body)
        .arg("-")
        .output();
    outcome.succeeded();
    let pdf = Pdf::from_bytes(&outcome.stdout);

    assert_eq!(pdf.page_count(), 3);
    assert!(
        pdf.page_text(1).contains("COVERBAND"),
        "{}",
        pdf.page_text(1)
    );
    assert!(
        !pdf.page_text(2).contains("COVERBAND"),
        "{}",
        pdf.page_text(2)
    );
}

/// A cover on its own is a document like any other.
#[test]
fn a_cover_alone_still_converts() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("cover-alone");
    let (cover, _) = fixtures(&scratch);

    let outcome = Run::new().arg("cover").arg(&cover).arg("-").output();
    outcome.succeeded();
    let pdf = Pdf::from_bytes(&outcome.stdout);
    assert_eq!(pdf.page_count(), 1);
    assert!(pdf.text().contains("COVERTEXT"));
}
