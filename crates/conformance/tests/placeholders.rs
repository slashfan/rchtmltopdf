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
            "[doctitle]",
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

/// And defines only those: a pair named after a built-in is shadowed by it
/// (#111). The measured command line is the harness's, `--replace page`
/// against a footer that asks for `[page]`.
#[test]
fn a_replacement_does_not_shadow_a_built_in() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-replace-built-in");
    let document = page(&scratch);

    let (pdf, _) = run(
        &document,
        &[
            "--replace",
            "page",
            "SHADOWED",
            "--footer-center",
            "PAGE=[page]",
        ],
    );
    let text = flat(&pdf);
    for expected in ["PAGE=1", "PAGE=2", "PAGE=3"] {
        assert!(text.contains(expected), "{expected} missing from {text:?}");
    }
    assert!(!text.contains("SHADOWED"), "{text:?}");
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
/// attributes around it. `[doctitle]` is the one `--title` answers for (#110).
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
            "[doctitle]",
        ],
    );
    let text = flat(&pdf);
    assert!(text.contains("Tom & Jerry"), "{text:?}");
    // Drawn as text rather than interpreted as markup.
    assert!(text.contains("<b>bold</b>"), "{text:?}");
}

/// **`[title]` is the document's own `<title>`, `[doctitle]` the file's**
/// (#110). The command line is the harness's, with `--title` given so the two
/// cannot be confused: wkhtmltopdf 0.12.6.1 prints the element in one and the
/// option in the other, and we printed the option in both.
#[test]
fn title_is_the_documents_element_and_doctitle_the_command_lines() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-title");
    let document = fixture::write(scratch.path(), "t.html", "<p>one</p>");
    std::fs::write(
        &document,
        std::fs::read_to_string(&document)
            .expect("readable")
            .replace(
                "<title>conformance</title>",
                "<title>The title element of the document</title>",
            ),
    )
    .expect("writable");

    let (pdf, _) = run(
        &document,
        &[
            "--title",
            "The title given on the command line",
            "--footer-center",
            "TITLE=[title]",
            "--footer-right",
            "DOCTITLE=[doctitle]",
        ],
    );
    let text = flat(&pdf);
    assert!(
        text.contains("TITLE=The title element of the document"),
        "{text:?}"
    );
    assert!(
        text.contains("DOCTITLE=The title given on the command line"),
        "{text:?}"
    );
}

/// With several documents each page says its own, and the file's is the first
/// document's when `--title` was not given — the same title the merge puts in
/// the Info dictionary (#107).
#[test]
fn each_document_prints_its_own_title_and_the_file_takes_the_firsts() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("placeholders-two-titles");
    let named = |name: &str, body: &str| {
        let path = scratch.join(name);
        std::fs::write(
            &path,
            fixture::document(body).replace(
                "<title>conformance</title>",
                &format!("<title>{}</title>", name.trim_end_matches(".html")),
            ),
        )
        .expect("writable");
        path.display().to_string()
    };
    let first = named("First.html", "<p>one</p>");
    let second = named("Second.html", "<p>two</p>");

    let outcome = Run::new()
        .args(["--footer-center", "T=[title] D=[doctitle]"])
        .arg(&first)
        .arg(&second)
        .arg("-")
        .output();
    outcome.succeeded();
    let pdf = Pdf::from_bytes(&outcome.stdout);
    assert_eq!(pdf.page_count(), 2, "{}", pdf.describe());
    assert!(
        pdf.page_text(1)
            .replace('\n', "")
            .contains("T=First D=First"),
        "{:?}",
        pdf.page_text(1)
    );
    assert!(
        pdf.page_text(2)
            .replace('\n', "")
            .contains("T=Second D=First"),
        "{:?}",
        pdf.page_text(2)
    );
}
