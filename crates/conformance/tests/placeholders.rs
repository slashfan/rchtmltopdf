//! wkhtmltopdf's `[page]`, `[date]` and the rest, in a real document.
//!
//! The page numbers are the only ones the browser answers, and they are the only
//! ones a unit test cannot check: nobody knows how many pages a document has
//! until it has been laid out, which happens inside the print call.

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

/// The three that name a position in the outline. Empty rather than their own
/// name, and said once on stderr rather than silently.
#[test]
fn what_needs_an_outline_is_empty_and_reported() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-outline");
    let document = page(&scratch);

    let (pdf, outcome) = run(&document, &["--footer-center", "in [section] here"]);
    let text = flat(&pdf);
    assert!(text.contains("in") && text.contains("here"), "{text:?}");
    assert!(
        !text.contains("section"),
        "the placeholder should expand to nothing, not to its own name: {text:?}"
    );
    assert!(
        outcome.stderr.contains("[section]") && outcome.stderr.contains("V2"),
        "should say what is missing and when:\n{}",
        outcome.stderr
    );
    assert_eq!(
        outcome.stderr.matches("[section]").count(),
        1,
        "once per name, however many pages:\n{}",
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
