//! Page numbers across several documents (#39), and `--page-offset` (#37).
//!
//! Nobody knows how many pages a document has until it has been laid out, and
//! `[page]` counts across every document of a conversion, so the bands are
//! drawn after the merge from counts read off the printed pages (D38). What
//! is asserted here is each of wkhtmltopdf's two frames on a real conversion:
//! across the output, and within the document.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::require_chromium;

const THREE_PAGES: &str = "<div style=\"page-break-after:always\">one</div>\
                           <div style=\"page-break-after:always\">two</div>\
                           <div>three</div>";
const TWO_PAGES: &str = "<div style=\"page-break-after:always\">four</div><div>five</div>";
const COVER: &str = "<div>COVERTEXT</div>";

fn fixtures(scratch: &Scratch) -> (String, String, String) {
    let a = fixture::write(scratch.path(), "a.html", THREE_PAGES);
    let b = fixture::write(scratch.path(), "b.html", TWO_PAGES);
    let c = fixture::write(scratch.path(), "c.html", COVER);
    (
        a.display().to_string(),
        b.display().to_string(),
        c.display().to_string(),
    )
}

fn convert(args: &[&str]) -> Pdf {
    let outcome = Run::new().args(args.iter().copied()).arg("-").output();
    outcome.succeeded();
    Pdf::from_bytes(&outcome.stdout)
}

/// A page's text with every space removed, because each number is its own
/// text run and extraction puts its own spacing between runs.
fn compact(pdf: &Pdf, page: usize) -> String {
    pdf.page_text(page)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// **The point of D38.** `[page]` and `[topage]` count across both documents;
/// a Chromium template restarted at one for the second.
#[test]
fn page_and_topage_count_across_documents() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("numbering-across");
    let (a, b, _) = fixtures(&scratch);

    let pdf = convert(&["--footer-center", "P[page]/[topage]", &a, &b]);
    assert_eq!(pdf.page_count(), 5, "{}", pdf.describe());
    for page in 1..=5 {
        let expected = format!("P{page}/5");
        assert!(
            compact(&pdf, page).contains(&expected),
            "page {page}: {}",
            compact(&pdf, page)
        );
    }
}

/// The other frame: within the document. `[sitepage]` and `[sitepages]`
/// restart at each object, and `[frompage]` belongs to neither frame — it is
/// the first page of the **output** (D51), the same number on every page.
///
/// Measured on wkhtmltopdf 0.12.6.1: two two-page documents print
/// `1|4|1|1|2`, `2|4|1|2|2`, `3|4|1|1|2`, `4|4|1|2|2` for
/// `[page]|[topage]|[frompage]|[sitepage]|[sitepages]`. `outline.cc` fills it
/// as `off+1` and never looks at the object.
#[test]
fn sitepage_and_sitepages_restart_but_frompage_is_the_first_page_of_the_output() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("numbering-within");
    let (a, b, _) = fixtures(&scratch);

    let pdf = convert(&[
        "--footer-left",
        "F[frompage]",
        "--footer-right",
        "S[sitepage]/[sitepages]",
        &a,
        &b,
    ]);
    let expected = ["F1S1/3", "F1S2/3", "F1S3/3", "F1S1/2", "F1S2/2"];
    for (page, expected) in expected.iter().enumerate() {
        let text = compact(&pdf, page + 1);
        assert!(text.contains(expected), "page {}: {text}", page + 1);
    }
}

/// A table of contents counts in `[page]` like any other object, and is its
/// own site: the page it occupies prints `1/1`, and the document behind it
/// starts its own frame again.
///
/// Measured on wkhtmltopdf 0.12.6.1: `toc a.html b.html` over a three-page and
/// a two-page document prints `1|5|1|1|1` on the table and `2|5|1|1|3` on the
/// page behind it.
#[test]
fn a_table_of_contents_counts_and_is_its_own_site() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("numbering-toc");
    let (a, b, _) = fixtures(&scratch);

    let pdf = convert(&[
        "--footer-center",
        "P[page]/[topage]S[sitepage]/[sitepages]",
        "toc",
        &a,
        &b,
    ]);
    assert_eq!(pdf.page_count(), 6, "{}", pdf.describe());
    assert!(
        compact(&pdf, 1).contains("P1/6S1/1"),
        "the table is page one of six, and one page of its own site: {}",
        compact(&pdf, 1)
    );
    assert!(
        compact(&pdf, 2).contains("P2/6S1/3"),
        "the document behind it starts its own site again: {}",
        compact(&pdf, 2)
    );
}

/// A cover counts like any other page (D45): the page behind it is page two,
/// `[topage]` includes it, and the numbering keeps running into the next
/// document. What the cover does not get is the band itself.
#[test]
fn a_cover_counts_and_the_numbering_runs_through_it() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("numbering-cover");
    let (a, b, c) = fixtures(&scratch);

    let pdf = convert(&["--footer-center", "P[page]/[topage]", "cover", &c, &a, &b]);
    assert_eq!(pdf.page_count(), 6);
    assert!(
        !compact(&pdf, 1).contains("P"),
        "the cover has no band: {}",
        compact(&pdf, 1)
    );
    for page in 2..=6 {
        let expected = format!("P{page}/6");
        assert!(
            compact(&pdf, page).contains(&expected),
            "page {page}: {}",
            compact(&pdf, page)
        );
    }

    // The cover is its own site, so the document behind it starts at one
    // again rather than continuing the cover's frame.
    let pdf = convert(&[
        "--footer-center",
        "S[sitepage]/[sitepages]",
        "cover",
        &c,
        &a,
    ]);
    assert!(compact(&pdf, 2).contains("S1/3"), "{}", compact(&pdf, 2));
}

/// **`--page-offset` is one number for the whole output** (D51), though
/// wkhtmltopdf's help lists it among the page options: it lives in `PdfGlobal`,
/// so wherever it is written it shifts every page, and the last one written
/// wins.
///
/// Measured on wkhtmltopdf 0.12.6.1 over two two-page documents:
/// `--page-offset 100` written after the second document prints
/// `101 102 103 104` of `104`, not `1 2 103 104`; and
/// `--page-offset 10 a --page-offset 100 b` prints the same `101..104`.
#[test]
fn page_offset_is_one_number_for_the_whole_output() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("numbering-offset");
    let (a, b, _) = fixtures(&scratch);

    // Written before the first document, where it reads as a default.
    let pdf = convert(&[
        "--page-offset",
        "10",
        "--footer-center",
        "P[page]/[topage]F[frompage]",
        &a,
        &b,
    ]);
    assert!(
        compact(&pdf, 1).contains("P11/15F11"),
        "{}",
        compact(&pdf, 1)
    );
    assert!(
        compact(&pdf, 5).contains("P15/15F11"),
        "the first page of the output is offset plus one: {}",
        compact(&pdf, 5)
    );

    // Written after the second document, where it still reaches the first.
    let pdf = convert(&[
        "--footer-center",
        "P[page]/[topage]",
        &a,
        &b,
        "--page-offset",
        "100",
    ]);
    for page in 1..=5 {
        let expected = format!("P{}/105", 100 + page);
        assert!(
            compact(&pdf, page).contains(&expected),
            "page {page}: {}",
            compact(&pdf, page)
        );
    }

    // Written twice, on one document each: the last one wins for both.
    let pdf = convert(&[
        "--footer-center",
        "P[page]/[topage]",
        "--page-offset",
        "10",
        &a,
        "--page-offset",
        "100",
        &b,
    ]);
    assert!(
        compact(&pdf, 1).contains("P101/105"),
        "{}",
        compact(&pdf, 1)
    );
    assert!(
        compact(&pdf, 5).contains("P105/105"),
        "{}",
        compact(&pdf, 5)
    );
}

/// The band is drawn on top of the page, so the page's own text is still
/// there under it, and the band's text is extractable like any other.
#[test]
fn the_band_and_the_page_are_both_there() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("numbering-both");
    let (a, _, _) = fixtures(&scratch);

    let pdf = convert(&["--header-left", "HEAD", "--footer-right", "FOOT", &a]);
    let text = pdf.page_text(2);
    assert!(
        text.contains("two") && text.contains("HEAD") && text.contains("FOOT"),
        "{text}"
    );
}

/// **A heading stays in force into the next document** (D47). wkhtmltopdf
/// keeps one cache of headings for the whole output and never resets it at a
/// document boundary, so a second document that brings an `h1` and nothing
/// under it names its own `h1` and the first document's `h2`.
///
/// Read from `outline.cc` 0.12.6 rather than measured: the harness corpus has
/// no multi-document case with headings (#32). It holds our own behaviour to
/// the rule until one does.
#[test]
fn a_heading_stays_in_force_into_the_next_document() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("numbering-sections-across");
    let first = fixture::write(
        scratch.path(),
        "first.html",
        "<h1>Alpha</h1><h2>Alpha One</h2><p>body</p>",
    );
    let second = fixture::write(scratch.path(), "second.html", "<h1>Beta</h1><p>body</p>");

    let pdf = convert(&[
        "--footer-center",
        "in [section] / [subsection] here",
        &first.display().to_string(),
        &second.display().to_string(),
    ]);
    assert_eq!(pdf.page_count(), 2, "{}", pdf.describe());
    let flatten = |page: usize| pdf.page_text(page).replace('\n', "");
    assert!(
        flatten(1).contains("in Alpha / Alpha One here"),
        "{}",
        flatten(1)
    );
    assert!(
        flatten(2).contains("in Beta / Alpha One here"),
        "{}",
        flatten(2)
    );
}
