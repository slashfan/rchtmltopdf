//! wkhtmltopdf's `[page]`, `[date]` and the rest, in a real document.
//!
//! The page numbers are the ones a unit test cannot check: nobody knows how
//! many pages a document has until it has been laid out, which happens inside
//! the print call. Since D38 the bands are drawn afterwards, from counts read
//! off the printed pages, and `numbering.rs` holds the counts across several
//! documents; this file holds the placeholders within one.

use rchtmltopdf_conformance::binary::{Outcome, Run};
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::require_chromium;
use std::path::PathBuf;

/// Three pages, by explicit breaks rather than by how text happens to wrap.
const THREE_PAGES: &str = "<div style=\"page-break-after:always\">one</div>\
                           <div style=\"page-break-after:always\">two</div>\
                           <div>three</div>";

fn page(scratch: &Scratch) -> PathBuf {
    fixture::write(scratch.path(), "p.html", THREE_PAGES)
}

/// The extracted text with its line breaks taken out.
///
/// Every text-showing operation comes out on its own line, and a placeholder is
/// its own operation: `Page [page] / [topage]` arrives as five runs. The breaks
/// are an artefact of how the text was emitted rather than anything about the
/// band, so they go before anything is asserted.
fn flat(pdf: &Pdf) -> String {
    pdf.text().replace('\n', "")
}

fn run(page: &std::path::Path, options: &[&str]) -> (Pdf, Outcome) {
    let outcome = Run::new()
        .args(options.iter().copied())
        .arg(page.display().to_string())
        .arg("-")
        .output();
    outcome.succeeded();
    (Pdf::from_bytes(&outcome.stdout), outcome)
}

/// **The literal example** from the brief, from `grammar.rs`, and from the first
/// code block in the README. It printed a warning and no footer until now.
#[test]
fn page_of_topage_is_the_example_from_the_readme() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-example");
    let document = page(&scratch);

    let (pdf, _) = run(&document, &["--footer-center", "Page [page] / [topage]"]);
    assert_eq!(pdf.page_count(), 3, "{}", pdf.describe());

    let text = flat(&pdf);
    for expected in ["Page 1 / 3", "Page 2 / 3", "Page 3 / 3"] {
        assert!(
            text.contains(expected),
            "{expected:?} missing from {text:?}"
        );
    }
}

/// One document starts at page one, and with one document the page number
/// within the document and within the file are the same. V2 splits them.
#[test]
fn the_other_counting_placeholders_agree_while_there_is_one_document() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-counting");
    let document = page(&scratch);

    let (pdf, _) = run(
        &document,
        &[
            "--footer-left",
            "[frompage]",
            "--footer-right",
            "[sitepage]/[sitepages]",
        ],
    );
    let text = flat(&pdf);
    assert!(text.contains('1'), "frompage should be 1: {text:?}");
    assert!(
        text.contains("3/3"),
        "the last page should read 3/3: {text:?}"
    );
}

/// Everything that is not a page number is rendered from the command line, so
/// none of it depends on what Chromium thinks the answer is.
#[test]
fn the_command_line_answers_the_rest() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-command-line");
    let document = page(&scratch);

    let (pdf, _) = run(
        &document,
        &[
            "--title",
            "Invoice 42",
            "--header-left",
            "[title]",
            "--header-right",
            "[webpage]",
        ],
    );
    let text = flat(&pdf);
    assert!(text.contains("Invoice 42"), "{text:?}");
    // The path a scratch directory gets is long, and a band does not wrap, so
    // only the front of it survives. That it is the input at all is the point.
    assert!(
        text.contains("rchtmltopdf-conformance"),
        "the input should appear: {text:?}"
    );
}

/// Rendered here rather than by the template's own date class, whose format is
/// not wkhtmltopdf's. The exact day is whatever today is, so this asserts the
/// shape and that the two agree with each other.
#[test]
fn the_date_and_the_time_are_rendered_by_us() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-date");
    let document = page(&scratch);

    let (pdf, _) = run(&document, &["--footer-center", "[isodate]"]);
    let text = pdf.text();

    // `2026-09-12T14:05:09+02:00`: four digits, two dashes, a T and an offset.
    let line = text
        .lines()
        .find(|line| line.contains('T') && line.contains('-'))
        .unwrap_or_else(|| panic!("no date in {text:?}"));
    assert_eq!(
        line.chars().filter(|c| c.is_ascii_digit()).count(),
        18,
        "{line:?}"
    );
    assert!(
        line.contains('+') || line.matches('-').count() == 3,
        "{line:?}"
    );
}

/// `--replace` defines placeholders of the user's own.
#[test]
fn replace_defines_a_placeholder() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-replace");
    let document = page(&scratch);

    let (pdf, _) = run(
        &document,
        &[
            "--replace",
            "client",
            "Acme Ltd",
            "--footer-center",
            "For [client]",
        ],
    );
    assert!(pdf.text().contains("For Acme Ltd"), "{:?}", pdf.text());
}

/// The three that name a heading, read from the outline (D36): the last
/// `h1`, `h2` or `h3` at or before the page.
#[test]
fn the_section_placeholders_name_the_heading_in_force() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-section");
    let document = fixture::write(
        scratch.path(),
        "s.html",
        "<h1>Alpha</h1><h2>Alpha One</h2>\
         <div style=\"page-break-after:always\"></div>\
         <p>still alpha</p>\
         <div style=\"page-break-after:always\"></div>\
         <h1>Beta</h1>",
    );

    let (pdf, outcome) = run(
        &document,
        &[
            "--no-outline",
            "--footer-center",
            "in [section] / [subsection] here",
        ],
    );
    assert_eq!(pdf.page_count(), 3, "{}", pdf.describe());
    let flatten = |page: usize| pdf.page_text(page).replace('\n', "");
    assert!(
        flatten(1).contains("in Alpha / Alpha One here"),
        "{}",
        flatten(1)
    );
    // A page with no heading of its own is still in the section that began
    // before it.
    assert!(
        flatten(2).contains("in Alpha / Alpha One here"),
        "{}",
        flatten(2)
    );
    assert!(
        flatten(3).contains("in Beta / Alpha One here"),
        "{}",
        flatten(3)
    );
    // `--no-outline` kept the outline out of the file, and the band still had
    // its headings: the outline was generated for the band's sake.
    assert!(pdf.outline().is_empty());
    assert!(
        !outcome.stderr.contains("[section]"),
        "nothing to warn about any more:\n{}",
        outcome.stderr
    );
}

/// A band's text is somebody's document title, and the template's styles are
/// attributes around it.
#[test]
fn a_title_full_of_markup_does_not_break_the_band() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-escaping");
    let document = page(&scratch);

    let (pdf, _) = run(
        &document,
        &[
            "--title",
            "Tom & Jerry <b>bold</b>",
            "--footer-center",
            "[title]",
        ],
    );
    let text = flat(&pdf);
    assert!(text.contains("Tom & Jerry"), "{text:?}");
    // Drawn as text rather than interpreted as markup.
    assert!(text.contains("<b>bold</b>"), "{text:?}");
}
