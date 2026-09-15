//! The outline — bookmarks — derived from the headings (#40).
//!
//! Chromium builds it during the print call, nested by heading level with a
//! destination on each entry, so what is asserted here is that the command
//! line reaches it: on by default, off with `--no-outline`, cut to
//! `--outline-depth`, joined across documents in order, left out for a cover
//! or an excluded document, and written out by `--dump-outline` in the XML
//! wkhtmltopdf's scripts read.
//!
//! Headings are bold and the vendored font has one weight, so the *text* of
//! these pages is not extractable (see `fixtures/fonts/README.md`). Nothing
//! here reads it: the outline carries the titles itself.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::{Bookmark, Pdf};
use rchtmltopdf_conformance::require_chromium;

/// Two chapters over two pages; the first has a section with a subsection.
const CHAPTERS: &str = "<h1>Chapter One</h1><p>x</p>\
                        <h2>Section One</h2><p>x</p>\
                        <h3>Deep One</h3><p>x</p>\
                        <div style=\"page-break-before:always\"></div>\
                        <h1>Chapter Two</h1><p>x</p>\
                        <h2>Section Two</h2>";
const APPENDIX: &str = "<h1>Appendix</h1><p>x</p>";

fn bookmark(level: usize, title: &str, page: usize) -> Bookmark {
    Bookmark {
        level,
        title: title.into(),
        page,
    }
}

fn fixtures(scratch: &Scratch) -> (String, String) {
    let chapters = fixture::write(scratch.path(), "chapters.html", CHAPTERS);
    let appendix = fixture::write(scratch.path(), "appendix.html", APPENDIX);
    (
        chapters.display().to_string(),
        appendix.display().to_string(),
    )
}

/// Every `page` attribute of a dump, in the order it was written.
fn pages(dump: &std::path::Path) -> Vec<i64> {
    let xml = std::fs::read_to_string(dump).expect("the dump should be written");
    xml.split("page=\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(|page| page.parse().expect("a page attribute should be a number"))
        .collect()
}

fn convert(args: &[&str]) -> Pdf {
    let outcome = Run::new().args(args.iter().copied()).arg("-").output();
    outcome.succeeded();
    Pdf::from_bytes(&outcome.stdout)
}

#[test]
fn the_headings_become_a_nested_outline_by_default() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("outline-default");
    let (chapters, _) = fixtures(&scratch);

    let pdf = convert(&[&chapters]);
    assert_eq!(pdf.page_count(), 2, "{}", pdf.describe());
    assert_eq!(
        pdf.outline(),
        [
            bookmark(1, "Chapter One", 1),
            bookmark(2, "Section One", 1),
            bookmark(3, "Deep One", 1),
            bookmark(1, "Chapter Two", 2),
            bookmark(2, "Section Two", 2),
        ]
    );
}

#[test]
fn no_outline_leaves_none() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("outline-none");
    let (chapters, _) = fixtures(&scratch);

    let pdf = convert(&["--no-outline", &chapters]);
    assert!(pdf.outline().is_empty(), "{:?}", pdf.outline());
    // And `--outline` after it turns it back on: the last one written wins.
    let pdf = convert(&["--no-outline", "--outline", &chapters]);
    assert_eq!(pdf.outline().len(), 5);
}

#[test]
fn the_depth_bounds_the_tree() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("outline-depth");
    let (chapters, _) = fixtures(&scratch);

    let pdf = convert(&["--outline-depth", "2", &chapters]);
    assert_eq!(
        pdf.outline(),
        [
            bookmark(1, "Chapter One", 1),
            bookmark(2, "Section One", 1),
            bookmark(1, "Chapter Two", 2),
            bookmark(2, "Section Two", 2),
        ]
    );

    let pdf = convert(&["--outline-depth", "1", &chapters]);
    assert_eq!(
        pdf.outline(),
        [bookmark(1, "Chapter One", 1), bookmark(1, "Chapter Two", 2)]
    );
}

/// **Across documents.** The second document's entries follow the first's and
/// point at the pages they landed on after the merge.
#[test]
fn the_outlines_of_several_documents_are_joined_in_order() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("outline-joined");
    let (chapters, appendix) = fixtures(&scratch);

    let pdf = convert(&[&chapters, &appendix]);
    assert_eq!(pdf.page_count(), 3);
    let outline = pdf.outline();
    assert_eq!(outline.len(), 6, "{outline:?}");
    assert_eq!(outline[0], bookmark(1, "Chapter One", 1));
    assert_eq!(outline[5], bookmark(1, "Appendix", 3));

    // The reverse order gives the reverse outline.
    let pdf = convert(&[&appendix, &chapters]);
    let outline = pdf.outline();
    assert_eq!(outline[0], bookmark(1, "Appendix", 1));
    assert_eq!(outline[1], bookmark(1, "Chapter One", 2));
}

/// A cover "does not appear in the table of contents", and an excluded page
/// object contributes nothing either.
#[test]
fn a_cover_and_an_excluded_document_stay_out_of_the_outline() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("outline-excluded");
    let (chapters, appendix) = fixtures(&scratch);

    let pdf = convert(&["cover", &appendix, &chapters]);
    let titles: Vec<String> = pdf.outline().into_iter().map(|b| b.title).collect();
    assert!(!titles.contains(&"Appendix".to_string()), "{titles:?}");
    assert_eq!(titles.len(), 5);
    // And the chapters still point at their own pages, after the cover.
    assert_eq!(pdf.outline()[0].page, 2);

    let pdf = convert(&[&chapters, &appendix, "--exclude-from-outline"]);
    let titles: Vec<String> = pdf.outline().into_iter().map(|b| b.title).collect();
    assert!(!titles.contains(&"Appendix".to_string()), "{titles:?}");

    // Excluded as a default, included again for one document.
    let pdf = convert(&[
        "--exclude-from-outline",
        &chapters,
        &appendix,
        "--include-in-outline",
    ]);
    assert_eq!(pdf.outline(), [bookmark(1, "Appendix", 3)]);
}

/// The XML wkhtmltopdf's scripts read, written even when the file itself
/// carries no outline, and describing what the file carries when it does.
#[test]
fn dump_outline_writes_wkhtmltopdfs_xml() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("outline-dump");
    let (chapters, appendix) = fixtures(&scratch);
    let dump = scratch.join("outline.xml");
    let dump_path = dump.display().to_string();

    let pdf = convert(&[
        "--dump-outline",
        &dump_path,
        "--outline-depth",
        "2",
        &chapters,
        &appendix,
    ]);
    assert_eq!(pdf.outline().len(), 5);
    let xml = std::fs::read_to_string(&dump).expect("the dump should be written");
    assert!(
        xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<outline xmlns=\"http://wkhtmltopdf.org/outline\">"),
        "{xml}"
    );
    for expected in [
        "<item title=\"Chapter One\" page=\"1\" link=\"\" backLink=\"\">",
        "<item title=\"Section One\" page=\"1\" link=\"\" backLink=\"\"/>",
        "<item title=\"Chapter Two\" page=\"2\" link=\"\" backLink=\"\">",
        "<item title=\"Appendix\" page=\"3\" link=\"\" backLink=\"\"/>",
    ] {
        assert!(xml.contains(expected), "{expected:?} missing from:\n{xml}");
    }
    assert!(
        !xml.contains("Deep One"),
        "the depth applies to the dump too:\n{xml}"
    );

    // `--no-outline --dump-outline`: the dump is the point, and it still comes.
    let quiet = scratch.join("quiet.xml");
    let pdf = convert(&[
        "--no-outline",
        "--dump-outline",
        &quiet.display().to_string(),
        &chapters,
    ]);
    assert!(pdf.outline().is_empty());
    let xml = std::fs::read_to_string(&quiet).expect("the dump should be written");
    assert!(xml.contains("Chapter Two"), "{xml}");
}

/// **`--page-offset` shifts what the dump says, and a cover still counts in
/// it** (#37, D40, D51).
///
/// Measured on wkhtmltopdf 0.12.6.1: three headings of one document dump as
/// `1 2 3`, and `--page-offset 10` turns them into `11 12 13`. The offset is
/// the conversion's, so writing it after the last document shifts the entries
/// of the first one too.
///
/// A cover is a page of the file here as it is in `[page]` (D45), so the
/// heading behind a one-page cover moves up by one.
#[test]
fn the_dump_carries_the_page_offset_and_counts_a_cover() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("outline-dump-offset");
    let (chapters, appendix) = fixtures(&scratch);

    let shifted = scratch.join("shifted.xml");
    convert(&[
        "--dump-outline",
        &shifted.display().to_string(),
        "--page-offset",
        "10",
        &chapters,
        &appendix,
    ]);
    assert_eq!(
        pages(&shifted),
        [11, 11, 11, 12, 12, 13],
        "the offset reaches every entry: {}",
        std::fs::read_to_string(&shifted).unwrap_or_default()
    );

    // Written after the last document, and still the whole dump's (D51).
    // This was the last thing telling the dump's number apart from the one a
    // band prints, and the measurement took it away.
    let apart = scratch.join("apart.xml");
    convert(&[
        "--dump-outline",
        &apart.display().to_string(),
        &chapters,
        &appendix,
        "--page-offset",
        "100",
    ]);
    assert_eq!(pages(&apart), [101, 101, 101, 102, 102, 103]);

    // A cover counts in the dump as it does in `[page]`, so the heading after
    // it moves up by one.
    let behind = scratch.join("behind.xml");
    convert(&[
        "--dump-outline",
        &behind.display().to_string(),
        "cover",
        &appendix,
        &chapters,
    ]);
    assert_eq!(pages(&behind), [2, 2, 2, 3, 3]);
}

/// A heading's title comes through as text: markup gone, entities resolved,
/// and an accent — which the PDF stores as UTF-16 — read back as written.
#[test]
fn a_title_survives_markup_entities_and_accents() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("outline-title");
    let page = fixture::write(
        scratch.path(),
        "title.html",
        "<h1>R&eacute;sum&eacute; &amp; <em>plan</em> &lt;draft&gt;</h1><p>x</p>",
    );

    let pdf = convert(&[&page.display().to_string()]);
    assert_eq!(pdf.outline(), [bookmark(1, "Résumé & plan <draft>", 1)]);
}
