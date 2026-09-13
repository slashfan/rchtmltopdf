//! The table of contents, generated rather than transformed (#42, D41).
//!
//! Every number asserted here was measured on wkhtmltopdf 0.12.6.1 first, on
//! the same command line, and the entries and page counts below are the ones
//! it produces.
//!
//! **What is read, and what is not.** The headings of a content document are
//! bold and the vendored font has one weight, so the text of *those* pages is
//! not extractable (see `fixtures/fonts/README.md`). Nothing here reads it.
//! The table of contents is our own document and its entries are not bold, so
//! its lines are read as text; the heading at the top of it is read off the
//! outline, which Chromium takes from the DOM rather than from the glyphs.
//!
//! **The two CSS options are measured on the rule under each entry.** It is
//! not a rectangle: Chromium draws a dashed border as a run of four-point
//! subpaths, filled in one go, which is why `inspect::painted_paths` exists.
//! Where that rule starts is where its entry starts, so the indentation is
//! read off it, and turning the dashes off leaves none of them behind.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::{Link, Pdf, Subpath};
use rchtmltopdf_conformance::require_chromium;

/// Two chapters over two pages, the first with a section under it.
///
/// No digits in a heading: the numbers are read off the page by parsing, and a
/// title that is a number would be read as one.
const CHAPTERS: &str = "<h1>Chapter One</h1><p>x</p>\
                        <h2>Section One</h2><p>x</p>\
                        <div style=\"page-break-before:always\"></div>\
                        <h1>Chapter Two</h1><p>x</p>";
const APPENDIX: &str = "<h1>Appendix</h1><p>x</p>";

fn fixtures(scratch: &Scratch) -> (String, String) {
    (
        fixture::write(scratch.path(), "chapters.html", CHAPTERS)
            .display()
            .to_string(),
        fixture::write(scratch.path(), "appendix.html", APPENDIX)
            .display()
            .to_string(),
    )
}

fn convert(args: &[&str]) -> Pdf {
    let outcome = Run::new().args(args.iter().copied()).arg("-").output();
    outcome.succeeded();
    Pdf::from_bytes(&outcome.stdout)
}

/// The numbers printed down the right-hand side of a table of contents, in the
/// order the entries are listed.
///
/// The default stylesheet floats them, so Chromium paints them in a run of
/// their own rather than each beside its entry: they come out of the page
/// together. That is why no fixture here has a digit in a heading.
fn numbers(pdf: &Pdf, pages: std::ops::RangeInclusive<usize>) -> Vec<i64> {
    pages
        .flat_map(|page| {
            pdf.page_text(page)
                .split_whitespace()
                .filter_map(|word| word.parse::<i64>().ok())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// How many times a title appears on a page.
fn listed(pdf: &Pdf, page: usize, title: &str) -> usize {
    pdf.page_text(page).matches(title).count()
}

/// **The entries, and the pages they point at.** `toc chapters.html` puts the
/// table on page 1 and the document behind it, so the chapter that was page 1
/// of the document is page 2 of the file — which is the number the table
/// prints. The first number is the table's own entry.
#[test]
fn the_table_lists_the_headings_with_the_pages_they_landed_on() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-basic");
    let (chapters, _) = fixtures(&scratch);

    let pdf = convert(&["toc", &chapters]);
    assert_eq!(pdf.page_count(), 3, "{}", pdf.describe());
    assert_eq!(numbers(&pdf, 1..=1), [1, 2, 2, 3]);

    // In the order the document reads: chapter, its section, next chapter.
    let page = pdf.page_text(1);
    let at = |title: &str| {
        page.find(title)
            .unwrap_or_else(|| panic!("{title:?}: {page}"))
    };
    assert!(at("Chapter One") < at("Section One"), "{page}");
    assert!(at("Section One") < at("Chapter Two"), "{page}");
}

/// **The table lists itself.** By the time wkhtmltopdf builds the list, the
/// table's own page has been printed and carries an `h1`, so "Table of
/// Contents 1" is the first line of its own list. Reproduced rather than
/// tidied away: it is what a migrated document looks like today.
#[test]
fn the_table_lists_its_own_heading_like_any_other() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-self");
    let (chapters, _) = fixtures(&scratch);

    let pdf = convert(&["toc", &chapters]);
    // Twice on the page: the heading at the top, and the entry for it.
    assert_eq!(
        listed(&pdf, 1, "Table of Contents"),
        2,
        "{}",
        pdf.page_text(1)
    );
    assert_eq!(numbers(&pdf, 1..=1)[0], 1, "and it is on page one");
}

/// **A table of contents longer than a page settles.** Its own length moves
/// every number it prints, and a table of sixty entries runs past one page, so
/// the first heading behind it is numbered from the far side of the *whole*
/// table. A single pass would have numbered it 2, from the far side of a table
/// one page long.
#[test]
fn a_table_over_several_pages_numbers_the_pages_behind_it() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-long");
    // Sixty headings over a few pages: it is the table that has to be long
    // here, not the document behind it.
    let mut body = String::new();
    for number in 0..60u8 {
        // Lettered, not numbered: a digit in a title would be read as a page.
        let (first, second) = (
            char::from(b'A' + number / 26),
            char::from(b'A' + number % 26),
        );
        body.push_str(&format!("<h1>Heading {first}{second}</h1><p>x</p>"));
    }
    let many = fixture::write(scratch.path(), "many.html", &body)
        .display()
        .to_string();

    // How long the table is, measured rather than assumed.
    let alone = convert(&[&many]).page_count();
    let pdf = convert(&["toc", &many]);
    let table = pdf.page_count() - alone;
    assert!(table >= 2, "the table should not fit on one page: {table}");

    let numbers = numbers(&pdf, 1..=table);
    assert_eq!(numbers.len(), 61, "its own entry and sixty headings");
    assert_eq!(numbers[0], 1, "the table itself starts on page one");
    assert_eq!(
        numbers[1],
        (table + 1) as i64,
        "the first heading sits behind the whole table, not behind one page of it"
    );
    assert_eq!(
        numbers[60],
        pdf.page_count() as i64,
        "and the last heading is on the last page"
    );
}

/// Where it is written is where its pages go, and the entries are in the order
/// the finished file reads: the document before it, then itself, then the
/// document after.
#[test]
fn a_table_sits_where_it_was_written() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-middle");
    let (chapters, appendix) = fixtures(&scratch);

    let pdf = convert(&[&chapters, "toc", &appendix]);
    assert_eq!(pdf.page_count(), 4, "{}", pdf.describe());
    // Chapter One 1, Section One 1, Chapter Two 2, the table 3, Appendix 4.
    assert_eq!(numbers(&pdf, 3..=3), [1, 1, 2, 3, 4]);
}

/// A cover is a page of the file, so it moves the numbers the table prints,
/// exactly as it moves the outline dump's (D40) and unlike `[page]`.
#[test]
fn a_cover_moves_the_numbers_the_table_prints() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-cover");
    let (chapters, appendix) = fixtures(&scratch);

    let pdf = convert(&["cover", &appendix, "toc", &chapters]);
    // The cover is page 1, the table page 2, the chapters 3 and 4.
    assert_eq!(numbers(&pdf, 2..=2), [2, 3, 3, 4]);
}

/// `--page-offset` reaches the table the same way, because the number printed
/// is the one the outline dump writes.
#[test]
fn the_page_offset_reaches_the_table() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-offset");
    let (chapters, _) = fixtures(&scratch);

    let pdf = convert(&["--page-offset", "100", "toc", &chapters]);
    assert_eq!(numbers(&pdf, 1..=1), [101, 102, 102, 103]);
}

/// `--toc-header-text` names the heading above the list, and the entry the
/// table makes of itself.
///
/// The heading is read off the outline rather than the page: Chromium takes an
/// outline title from the DOM, so this does not depend on the runner having a
/// bold face for the family the default stylesheet asks for.
#[test]
fn the_header_text_names_the_heading() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-header");
    let (chapters, _) = fixtures(&scratch);

    let pdf = convert(&["toc", "--toc-header-text", "Sommaire", &chapters]);
    let titles: Vec<String> = pdf
        .outline()
        .into_iter()
        .map(|bookmark| bookmark.title)
        .collect();
    assert!(titles.contains(&"Sommaire".to_string()), "{titles:?}");
    assert!(
        !titles.contains(&"Table of Contents".to_string()),
        "the default should be gone: {titles:?}"
    );
    assert_eq!(listed(&pdf, 1, "Sommaire"), 2, "{}", pdf.page_text(1));
}

/// The dashes of the rule drawn under each entry, in the grey the default
/// stylesheet asks for — `rgb(200,200,200)`, which is 0.7843 per channel.
///
/// Each dash is a subpath of its own, so there are many per entry.
fn dashes(pdf: &Pdf, page: usize) -> Vec<Subpath> {
    pdf.painted_paths(page)
        .into_iter()
        .filter(|path| path.is_about(0.7843, 0.7843, 0.7843))
        .collect()
}

/// `--disable-dotted-lines` takes the rule out from under every entry.
#[test]
fn the_dotted_lines_can_be_turned_off() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-dotted");
    let (chapters, _) = fixtures(&scratch);

    let dotted = convert(&["toc", &chapters]);
    assert!(
        dashes(&dotted, 1).len() > 20,
        "four entries of dashes: {}",
        dashes(&dotted, 1).len()
    );

    let plain = convert(&["toc", "--disable-dotted-lines", &chapters]);
    assert_eq!(dashes(&plain, 1).len(), 0, "none should be left");
    // The entries are still there: it is the rule that went, not the list.
    assert_eq!(numbers(&plain, 1..=1), [1, 2, 2, 3]);
}

/// `--toc-level-indentation` moves each level further in, and only the deeper
/// ones: the rule under an entry is the entry's own box, so where it starts is
/// where the entry starts.
#[test]
fn the_indentation_moves_the_deeper_entries() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-indent");
    let (chapters, _) = fixtures(&scratch);

    let narrow = convert(&["toc", "--toc-level-indentation", "1em", &chapters]);
    let wide = convert(&["toc", "--toc-level-indentation", "6em", &chapters]);
    let (shallow_narrow, deep_narrow) = edges(&narrow);
    let (shallow_wide, deep_wide) = edges(&wide);

    assert!(
        deep_narrow > shallow_narrow + 1.0,
        "a section is already indented under its chapter: {shallow_narrow} then {deep_narrow}"
    );
    assert!(
        deep_wide > deep_narrow + 10.0,
        "and a wider setting moves it further: {deep_narrow} then {deep_wide}"
    );
    assert!(
        (shallow_wide - shallow_narrow).abs() <= 1.0,
        "while the chapter above it stays put: {shallow_narrow} then {shallow_wide}"
    );
}

/// Where the shallowest and the deepest entry of a table of contents begin, in
/// points from the left edge of the page.
///
/// A rule is a row of dashes across the page, so the entry's own left edge is
/// the leftmost dash *of its row* — the rightmost dash of any row is near the
/// page's other side and says nothing about indentation. The rows are found by
/// the height the dashes sit at.
fn edges(pdf: &Pdf) -> (f64, f64) {
    let mut rows: Vec<(i64, f64)> = Vec::new();
    for dash in dashes(pdf, 1) {
        let row = (dash.bounds.bottom * 10.0).round() as i64;
        match rows.iter_mut().find(|(at, _)| *at == row) {
            Some((_, left)) => *left = left.min(dash.bounds.left),
            None => rows.push((row, dash.bounds.left)),
        }
    }
    assert!(rows.len() >= 2, "the table should have rules at two levels");
    let lefts: Vec<f64> = rows.iter().map(|(_, left)| *left).collect();
    (
        lefts.iter().copied().fold(f64::MAX, f64::min),
        lefts.iter().copied().fold(f64::MIN, f64::max),
    )
}

/// **An entry links to the heading it names** (#43, D42). The pages the links
/// land on are the pages the entries print, and the table's own entry points
/// at the table.
#[test]
fn the_entries_link_to_the_headings_they_name() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-links");
    let (chapters, _) = fixtures(&scratch);

    let pdf = convert(&["toc", &chapters]);
    assert_eq!(
        pdf.links(1),
        [
            Link::Internal { page: 1 },
            Link::Internal { page: 2 },
            Link::Internal { page: 2 },
            Link::Internal { page: 3 },
        ],
        "the table itself, then the chapter, its section and the next chapter"
    );
    // The same pages the entries print, so a reader following one arrives
    // where the number said it would.
    assert_eq!(numbers(&pdf, 1..=1), [1, 2, 2, 3]);
    // And the links are on the table's page only.
    assert!(pdf.links(2).is_empty() && pdf.links(3).is_empty());
}

/// `--disable-toc-links` writes the entries without a link, so there is no
/// annotation at all rather than one that goes nowhere.
#[test]
fn disable_toc_links_leaves_the_entries_unlinked() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-nolinks");
    let (chapters, _) = fixtures(&scratch);

    let pdf = convert(&["toc", "--disable-toc-links", &chapters]);
    assert!(pdf.links(1).is_empty(), "{:?}", pdf.links(1));
    // The list is untouched: it is the links that went.
    assert_eq!(numbers(&pdf, 1..=1), [1, 2, 2, 3]);
}

/// **A table's links are not the document's links.** `--disable-internal-links`
/// and `--disable-external-links` are about the links in the pages being
/// converted; measured on wkhtmltopdf 0.12.6.1, neither touches the table of
/// contents, which keeps its four either way (D42).
#[test]
fn the_link_options_of_a_document_do_not_reach_the_table() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-policy");
    let (chapters, _) = fixtures(&scratch);

    for option in ["--disable-internal-links", "--disable-external-links"] {
        let pdf = convert(&[option, "toc", &chapters]);
        assert_eq!(
            pdf.links(1).len(),
            4,
            "{option} should leave the table alone: {:?}",
            pdf.links(1)
        );
    }
}

/// A table of contents on its own converts: there is nothing to list, and
/// wkhtmltopdf prints the page rather than refusing.
#[test]
fn a_table_of_contents_alone_still_converts() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let pdf = convert(&["toc"]);
    assert_eq!(pdf.page_count(), 1, "{}", pdf.describe());
}

/// `--xsl-style-sheet` is accepted, warned about and ignored: the table is
/// generated rather than transformed (D41), and the command line still
/// converts rather than failing on it.
#[test]
fn a_stylesheet_is_accepted_and_says_it_is_ignored() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("contents-xsl");
    let (chapters, _) = fixtures(&scratch);
    let stylesheet = fixture::write(scratch.path(), "mine.xsl", "<xsl:stylesheet/>")
        .display()
        .to_string();

    let outcome = Run::new()
        .args(["toc", "--xsl-style-sheet", &stylesheet, &chapters, "-"])
        .output();
    outcome.succeeded();
    let said = &outcome.stderr;
    assert!(said.contains("--xsl-style-sheet"), "{said}");
    assert!(said.contains("no Chromium equivalent"), "{said}");
    // And the table is there all the same.
    let pdf = Pdf::from_bytes(&outcome.stdout);
    assert_eq!(numbers(&pdf, 1..=1), [1, 2, 2, 3]);
}
