//! `--header-html` and `--footer-html`: a band that is a document (D39).
//!
//! wkhtmltopdf loads the document once per page with the placeholders
//! appended as a query string, and its manual carries a `subst()` script that
//! reads them back. That script is the fixture here, verbatim, because it is
//! what every migrated header does. The geometry is wkhtmltopdf's other rule
//! for these bands, which is not the text band's: the document's height *is*
//! the margin unless the margin was written.
//!
//! The instrument is the same as `bands.rs`: a block filling the content area,
//! whose top edge is where the content starts.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::{POINTS_PER_INCH, Pdf, Rect};
use rchtmltopdf_conformance::require_chromium;
use std::path::Path;

/// A block filling the content area, so where it lands measures the margin.
const BODY: &str = "<div style=\"background:#000;width:100%;height:200pt\"></div>";

/// Three pages, by explicit breaks rather than by how text happens to wrap.
const THREE_PAGES: &str = "<div style=\"page-break-after:always\">one</div>\
                           <div style=\"page-break-after:always\">two</div>\
                           <div>three</div>";

/// The script from wkhtmltopdf's manual, as written there.
const SUBST: &str = "<script>\
function subst() {\
    var vars = {};\
    var query_strings_from_url = document.location.search.substring(1).split('&');\
    for (var query_string in query_strings_from_url) {\
        if (query_strings_from_url.hasOwnProperty(query_string)) {\
            var temp_var = query_strings_from_url[query_string].split('=', 2);\
            vars[temp_var[0]] = decodeURI(temp_var[1]);\
        }\
    }\
    var css_selector_classes = ['page', 'frompage', 'topage', 'webpage', 'section', \
'subsection', 'date', 'isodate', 'time', 'title', 'doctitle', 'sitepage', 'sitepages'];\
    for (var css_class in css_selector_classes) {\
        if (css_selector_classes.hasOwnProperty(css_class)) {\
            var element = document.getElementsByClassName(css_selector_classes[css_class]);\
            for (var j = 0; j < element.length; ++j) {\
                element[j].textContent = vars[css_selector_classes[css_class]];\
            }\
        }\
    }\
}\
</script>";

/// The manual's header, with the script run once the body exists.
fn numbered_header() -> String {
    format!(
        "{SUBST}<table style=\"border-bottom: 1px solid black; width: 100%\"><tr>\
         <td class=\"section\"></td>\
         <td style=\"text-align:right\">HDR <span class=\"page\"></span> of \
         <span class=\"topage\"></span></td>\
         </tr></table><script>subst()</script>"
    )
}

/// A band document of a known height, so the margin it becomes is known.
fn block_band(height_mm: f64) -> String {
    format!(
        "<style>body {{ margin: 0 }}</style>\
         <div style=\"background:#00f;width:100%;height:{height_mm}mm\">BAND</div>"
    )
}

fn convert(options: &[&str], inputs: &[&Path]) -> Pdf {
    let outcome = Run::new()
        .args(options.iter().copied())
        .args(inputs.iter().map(|path| path.display().to_string()))
        .arg("-")
        .output();
    outcome.succeeded();
    Pdf::from_bytes(&outcome.stdout)
}

fn mm(value: f64) -> f64 {
    value / 25.4 * POINTS_PER_INCH
}

/// The content area on a page. The page paints its own background across
/// the whole of it, and that is the tallest box narrower than the paper; the
/// band's block and the fixture's are both shorter.
fn content(pdf: &Pdf, page: usize) -> Rect {
    let paper = pdf.media_box(page);
    let mut inside: Vec<Rect> = pdf
        .painted_boxes(page)
        .into_iter()
        .filter(|painted| painted.width() < paper.width() * 0.99 && painted.height() > 150.0)
        .collect();
    inside.sort_by(|a, b| b.height().partial_cmp(&a.height()).unwrap());
    *inside
        .first()
        .unwrap_or_else(|| panic!("nothing was painted: {}", pdf.describe()))
}

/// The band document's own block, which is shorter than the content's.
fn band_block(pdf: &Pdf, page: usize) -> Rect {
    let paper = pdf.media_box(page);
    pdf.painted_boxes(page)
        .into_iter()
        .filter(|painted| painted.width() < paper.width() * 0.99)
        .find(|painted| (painted.height() - mm(20.0)).abs() < 1.5)
        .unwrap_or_else(|| panic!("the band's block was not painted: {}", pdf.describe()))
}

/// The manual's own example, on every page: the script reads the numbers off
/// the query string, so `[page]` and `[topage]` reach a document that never
/// mentions them.
#[test]
fn the_documented_header_is_framed_on_every_page_with_its_numbers() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("band-documents-numbers");
    let header = fixture::write(scratch.path(), "header.html", &numbered_header());
    let page = fixture::write(scratch.path(), "p.html", THREE_PAGES);

    let pdf = convert(&["--header-html", &header.display().to_string()], &[&page]);
    assert_eq!(pdf.page_count(), 3, "{}", pdf.describe());
    for number in 1..=3 {
        let text: String = pdf
            .page_text(number)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            text.contains(&format!("HDR{number}of3")),
            "page {number} reads {text:?}"
        );
    }
}

/// And across documents, where wkhtmltopdf's own templates could not count:
/// the fourth page of two documents says four of five.
#[test]
fn the_numbers_count_across_documents_sharing_the_header() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("band-documents-across");
    let header = fixture::write(scratch.path(), "header.html", &numbered_header());
    let a = fixture::write(scratch.path(), "a.html", THREE_PAGES);
    let b = fixture::write(
        scratch.path(),
        "b.html",
        "<div style=\"page-break-after:always\">four</div><div>five</div>",
    );

    let pdf = convert(&["--header-html", &header.display().to_string()], &[&a, &b]);
    assert_eq!(pdf.page_count(), 5, "{}", pdf.describe());
    let fourth: String = pdf
        .page_text(4)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(fourth.contains("HDR4of5"), "{fourth:?}");
}

/// **wkhtmltopdf's rule.** Nothing written for `--margin-top`, so the
/// document's height is the margin: a 20mm header puts the content 20mm down,
/// not the default 10mm, and the header itself starts at the paper's edge.
#[test]
fn an_unwritten_margin_becomes_the_documents_height() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("band-documents-auto");
    let header = fixture::write(scratch.path(), "header.html", &block_band(20.0));
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let pdf = convert(&["--header-html", &header.display().to_string()], &[&page]);
    let paper = pdf.media_box(1);
    let content = content(&pdf, 1);
    assert!(
        (paper.top - content.top - mm(20.0)).abs() < 1.5,
        "the content should start 20mm down, it starts {:.2}pt down: {}",
        paper.top - content.top,
        pdf.describe()
    );
    let band = band_block(&pdf, 1);
    assert!(
        (paper.top - band.top).abs() < 1.0,
        "the band should start at the paper's edge: {band:?}"
    );
    assert!(pdf.text().contains("BAND"), "{}", pdf.text());
}

/// Spacing opens a gap below the document, as it does below a text band.
#[test]
fn spacing_is_added_below_the_documents_height() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("band-documents-spacing");
    let header = fixture::write(scratch.path(), "header.html", &block_band(20.0));
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let pdf = convert(
        &[
            "--header-html",
            &header.display().to_string(),
            "--header-spacing",
            "5",
        ],
        &[&page],
    );
    let paper = pdf.media_box(1);
    let content = content(&pdf, 1);
    assert!(
        (paper.top - content.top - mm(25.0)).abs() < 1.5,
        "20mm of header and 5mm of spacing should put the content 25mm down, it is {:.2}pt down",
        paper.top - content.top
    );
}

/// `--margin-top 30mm` written: the document is fitted into the margin, its
/// bottom on the margin line, and the content is where the margin says.
#[test]
fn a_written_margin_fits_the_document_against_the_content() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("band-documents-named");
    let header = fixture::write(scratch.path(), "header.html", &block_band(20.0));
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let pdf = convert(
        &[
            "--header-html",
            &header.display().to_string(),
            "--margin-top",
            "30mm",
        ],
        &[&page],
    );
    let paper = pdf.media_box(1);
    let content = content(&pdf, 1);
    assert!(
        (paper.top - content.top - mm(30.0)).abs() < 1.5,
        "the content should start at the written margin, it starts {:.2}pt down",
        paper.top - content.top
    );
    let band = band_block(&pdf, 1);
    assert!(
        (paper.top - band.bottom - mm(30.0)).abs() < 1.5,
        "the band's bottom should sit on the margin line: {band:?}"
    );
}

/// The footer is the mirror: anchored to the paper's bottom, and its height
/// is the bottom margin when none was written.
#[test]
fn a_footer_document_is_anchored_to_the_bottom_and_sizes_the_margin() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("band-documents-footer");
    let footer = fixture::write(scratch.path(), "footer.html", &block_band(20.0));
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let pdf = convert(&["--footer-html", &footer.display().to_string()], &[&page]);
    let paper = pdf.media_box(1);
    let band = band_block(&pdf, 1);
    assert!(
        (band.bottom - paper.bottom).abs() < 1.0,
        "the band should end at the paper's edge: {band:?}"
    );
    let content = content(&pdf, 1);
    assert!(
        (content.bottom - paper.bottom - mm(20.0)).abs() < 1.5,
        "the content should end 20mm up, it ends {:.2}pt up: {}",
        content.bottom - paper.bottom,
        pdf.describe()
    );
}

/// A band document is named on the command line the way the input is, so it
/// is read under the same rule: what *it* reaches on the disk needs
/// `--enable-local-file-access`, and is refused without it — a warning, and
/// exit 1 with the PDF written, as for the input (D49). wkhtmltopdf took the
/// exit code from its header loader as it did from the page loader.
#[test]
fn a_band_document_reads_the_disk_under_the_same_rule_as_the_input() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("band-documents-files");
    fixture::write(scratch.path(), "style.css", "body { color: red }");
    let header = fixture::write(
        scratch.path(),
        "header.html",
        "<link rel=\"stylesheet\" href=\"style.css\"><div>HDR</div>",
    );
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let refused = Run::new()
        .arg("--header-html")
        .arg(header.display().to_string())
        .arg(page.display().to_string())
        .arg("-")
        .output();
    refused.failed();
    assert!(
        rchtmltopdf_conformance::binary::is_pdf(&refused.stdout),
        "a refusal writes the PDF and exits 1; it does not throw the document away"
    );
    assert!(
        refused.stderr.contains("style.css") && refused.stderr.contains("local file access"),
        "the stylesheet should have been refused with a warning: {}",
        refused.stderr
    );
    assert!(
        refused
            .stderr
            .contains("Exit with code 1 due to network error: ContentAccessDenied"),
        "the line applications grep for:\n{}",
        refused.stderr
    );

    let allowed = Run::new()
        .arg("--enable-local-file-access")
        .arg("--header-html")
        .arg(header.display().to_string())
        .arg(page.display().to_string())
        .arg("-")
        .output();
    allowed.succeeded();
    assert!(
        !allowed.stderr.contains("style.css"),
        "nothing should have been refused: {}",
        allowed.stderr
    );
}

/// A header named on the command line that is not there fails before a
/// browser starts, as a missing input does.
#[test]
fn a_missing_band_document_fails_like_a_missing_input() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("band-documents-missing");
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let outcome = Run::new()
        .arg("--header-html")
        .arg(scratch.join("nowhere.html").display().to_string())
        .arg(page.display().to_string())
        .arg("-")
        .output();
    outcome.failed();
    assert!(
        outcome.stderr.contains("nowhere.html"),
        "{}",
        outcome.stderr
    );
}

/// **An empty band option converts, and changes nothing** (D59).
///
/// A template engine that renders a header into a string writes
/// `--header-html ""` when the document has no header, and Snappy passes it
/// on. Measured on wkhtmltopdf 0.12.6.1: the conversion succeeds and the page
/// is what it would have been without the option. Here it used to be read as
/// a file called "", which is not there, so a whole class of real documents
/// failed with `exit 1` over an option nobody meant to set.
///
/// The content's top edge is the instrument, as everywhere in this file: an
/// ignored band leaves the margin alone, a band that was taken seriously would
/// push the content down.
#[test]
fn an_empty_band_document_is_ignored_rather_than_read() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("band-documents-empty");
    let page = fixture::write(scratch.path(), "page.html", BODY);

    let plain = convert(&[], &[&page]);
    let empty = convert(&["--header-html", "", "--footer-html", ""], &[&page]);

    assert_eq!(empty.page_count(), plain.page_count());
    let top = |pdf: &Pdf| pdf.largest_painted_box(1).top;
    assert!(
        (top(&empty) - top(&plain)).abs() < 1.0,
        "an ignored band should leave the content where it was: {} against {}",
        top(&empty),
        top(&plain)
    );
}
