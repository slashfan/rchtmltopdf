//! The table of contents: the HTML wkhtmltopdf's default stylesheet produced,
//! written directly rather than transformed (D41).
//!
//! wkhtmltopdf built the outline as XML, transformed it with an XSL stylesheet
//! and rendered the result as an object of its own. The transform is not
//! reproduced. Chromium does still perform it — the prototype worked on 153 —
//! but Chrome removes XSLT in 158, in November 2026, and D09 prefers whatever
//! browser the machine already has, so that path has an expiry date rather
//! than a bug. Rust has no XSLT engine worth linking. So the markup that
//! stylesheet *would* have produced is written here, from the same entries,
//! with the four `TOC Options` substituted where it carried them:
//!
//! | option | where the stylesheet carried it |
//! | --- | --- |
//! | `--toc-header-text` | the `h1` above the list |
//! | `--toc-level-indentation` | `ul ul {padding-left: …}` |
//! | `--toc-text-size-shrink` | `ul ul {font-size: …%}` |
//! | `--disable-dotted-lines` | the `div` rule's `border-bottom` |
//!
//! `--xsl-style-sheet` has no equivalent, and says so on the command line.
//!
//! # The numbers are pages of the file
//!
//! An entry's number is what [`crate::numbering::dump_page`] answers — the
//! physical page plus the document's `--page-offset` — measured on wkhtmltopdf
//! 0.12.6.1 and the same rule the outline dump follows (D40). A cover counts,
//! and so do the table of contents' own pages: with a three-page one, the
//! first heading of the document behind it is numbered 4.

use rchtmltopdf_core::settings::TocSettings;
use rchtmltopdf_pdf::OutlineItem;
use std::fmt::Write;

/// One line of the table of contents.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Heading level, 1 for an `h1`. Nesting follows it.
    pub level: usize,
    pub title: String,
    /// The page of the file the heading landed on, offset applied. What is
    /// *printed*, which `--page-offset` moves.
    pub page: i64,
    /// Where the heading actually is, for the link: the page of the finished
    /// file and the place on it, which no offset touches (D42).
    pub target: Target,
}

/// Where an entry's link goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub page: usize,
    pub left: f64,
    pub top: f64,
}

/// Every entry of the table of contents, in the order the documents come.
///
/// The documents were merged without the tables of contents — nobody knows how
/// long one is until it has been laid out — so `moved` carries a page of that
/// merge to the page it landed on in the finished file, and `number` turns
/// that into the number printed for it: [`crate::numbering::dump_page`], so a
/// `--page-offset` and a cover are accounted for exactly as they are in the
/// outline dump (D40).
///
/// `contents` is where each table of contents itself landed, with its heading.
/// wkhtmltopdf lists those like any other heading — `toc a.html` prints "Table
/// of Contents 1" above the document's own — because by the time it builds the
/// list, the table's page has been printed and carries an `h1`.
///
/// Order is the finished file's. Sorting by the page an entry landed on is
/// enough to interleave the tables with the headings, because a table's first
/// page is its own: no heading shares it.
pub fn entries(
    outline: &[OutlineItem],
    moved: impl Fn(usize) -> usize,
    contents: &[(usize, &str)],
    number: impl Fn(usize) -> i64,
) -> Vec<Entry> {
    let mut placed: Vec<(usize, Entry)> = Vec::new();
    walk(outline, 1, &mut |level, page, title, left, top| {
        let page = moved(page);
        placed.push((
            page,
            Entry {
                level,
                title: title.to_string(),
                page: number(page),
                target: Target { page, left, top },
            },
        ));
    });
    for (page, title) in contents {
        placed.push((
            *page,
            Entry {
                level: 1,
                title: (*title).to_string(),
                page: number(*page),
                // Its own heading is at the top of its own first page: the
                // table has not been printed yet, so there is no destination
                // to read, and the heading is the first thing on it.
                target: Target {
                    page: *page,
                    left: 0.0,
                    top: TOP_OF_THE_PAGE,
                },
            },
        ));
    }
    placed.sort_by_key(|(page, _)| *page);
    placed.into_iter().map(|(_, entry)| entry).collect()
}

/// High enough to be the top of any paper this prints on, for a link that has
/// no destination of its own to read.
///
/// A destination past the top of the page is not an error: a reader clamps it,
/// and lands at the top, which is where the table's own heading is.
const TOP_OF_THE_PAGE: f64 = 10_000.0;

/// Every outline entry, in reading order, with the level its nesting gives it.
fn walk(items: &[OutlineItem], level: usize, visit: &mut impl FnMut(usize, usize, &str, f64, f64)) {
    for item in items {
        visit(level, item.page, &item.title, item.left, item.top);
        walk(&item.children, level + 1, visit);
    }
}

/// The table of contents as a complete HTML document.
pub fn document(entries: &[Entry], toc: &TocSettings) -> String {
    let mut out = String::from(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Table of Contents</title>\n",
    );
    out.push_str(&style(toc));
    let _ = write!(
        out,
        "</head>\n<body>\n<h1>{}</h1>\n",
        escape(&toc.header_text)
    );
    write_level(&mut out, entries, &mut 0, 1, toc.links);
    out.push_str("</body>\n</html>\n");
    out
}

/// The stylesheet, with the options where the default XSL carried them.
///
/// `font-family: arial` is the default stylesheet's, kept as it was written:
/// a document that asks for a family the machine does not have gets the
/// browser's fallback, which is what wkhtmltopdf got too.
fn style(toc: &TocSettings) -> String {
    let dotted = if toc.dotted_lines {
        "          div {border-bottom: 1px dashed rgb(200,200,200);}\n"
    } else {
        ""
    };
    format!(
        "<style>\n\
         \x20         h1 {{\n\
         \x20           text-align: center;\n\
         \x20           font-size: 20px;\n\
         \x20           font-family: arial;\n\
         \x20         }}\n\
         {dotted}\
         \x20         span {{float: right;}}\n\
         \x20         li {{list-style: none;}}\n\
         \x20         ul {{\n\
         \x20           font-size: 20px;\n\
         \x20           font-family: arial;\n\
         \x20         }}\n\
         \x20         ul ul {{font-size: {shrink}%; }}\n\
         \x20         ul {{padding-left: 0em;}}\n\
         \x20         ul ul {{padding-left: {indent};}}\n\
         \x20         a {{text-decoration:none; color: black;}}\n\
         </style>\n",
        shrink = percent(toc.text_size_shrink),
        indent = escape(&toc.level_indentation),
    )
}

/// A shrink factor as the percentage the stylesheet wrote: 0.8 is `80`.
///
/// Trailing zeroes are dropped so the common factors read as integers, which
/// is what the default stylesheet says and what a reader comparing the two
/// would expect.
fn percent(factor: f64) -> String {
    let percent = factor * 100.0;
    if (percent - percent.round()).abs() < 1e-9 {
        format!("{}", percent.round())
    } else {
        let mut text = format!("{percent:.4}");
        while text.ends_with('0') {
            text.pop();
        }
        text.trim_end_matches('.').to_string()
    }
}

/// Write every entry at `level`, recursing into the deeper ones that follow.
///
/// The entries arrive flat, in reading order, and the nesting is rebuilt from
/// their levels. A level that jumps — an `h3` under an `h1`, which is ordinary
/// HTML — opens one list, not two: the stylesheet nested by *element*, so two
/// lists would indent it twice and shrink it twice.
fn write_level(out: &mut String, entries: &[Entry], at: &mut usize, level: usize, links: bool) {
    out.push_str("<ul>\n");
    while *at < entries.len() {
        let entry = &entries[*at];
        if entry.level < level {
            break;
        }
        if entry.level > level {
            // Deeper: a nested list inside the item just written.
            write_level(out, entries, at, entry.level, links);
            continue;
        }
        *at += 1;
        // The href names the heading rather than pointing at it: only the
        // browser knows where this line lands on the page, so it writes the
        // annotation and the `pdf` crate turns the marker into a destination
        // once every page of the finished file has a number (D42).
        let href = match links {
            true => format!(
                " href=\"{}{},{},{}\"",
                rchtmltopdf_pdf::CONTENTS_SCHEME,
                entry.target.page,
                entry.target.left,
                entry.target.top
            ),
            false => String::new(),
        };
        let _ = write!(
            out,
            "<li><div><a{href}>{} </a><span> {} </span></div>",
            escape(&entry.title),
            entry.page
        );
        // Children of this entry, if the next one is deeper.
        if entries.get(*at).is_some_and(|next| next.level > level) {
            write_level(out, entries, at, entries[*at].level, links);
        }
        out.push_str("</li>\n");
    }
    out.push_str("</ul>\n");
}

/// HTML-escape. A heading is the document's text, and an ampersand in one is
/// ordinary.
fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for character in raw.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: usize, title: &str, page: i64) -> Entry {
        Entry {
            level,
            title: title.into(),
            page,
            target: Target {
                page: page.max(0) as usize,
                left: 0.0,
                top: 0.0,
            },
        }
    }

    fn default_document(entries: &[Entry]) -> String {
        document(entries, &TocSettings::default())
    }

    fn item(title: &str, page: usize, children: Vec<OutlineItem>) -> OutlineItem {
        OutlineItem {
            title: title.into(),
            page,
            left: 0.0,
            top: 0.0,
            children,
        }
    }

    /// The page of the file, printed as it is.
    fn plain(page: usize) -> i64 {
        page as i64
    }

    /// **A heading is numbered where it lands, not where it was merged.**
    /// The documents were merged without the table of contents: "One" sat on
    /// page 1 of that merge and lands on page 2 behind a one-page table.
    #[test]
    fn a_heading_is_numbered_where_it_lands() {
        let outline = [
            item("One", 1, vec![item("One A", 1, vec![])]),
            item("Two", 2, vec![]),
        ];
        let entries = entries(
            &outline,
            |page| page + 1,
            &[(1, "Table of Contents")],
            plain,
        );
        assert_eq!(
            entries
                .iter()
                .map(|e| (e.level, e.title.as_str(), e.page))
                .collect::<Vec<_>>(),
            [
                (1, "Table of Contents", 1),
                (1, "One", 2),
                (2, "One A", 2),
                (1, "Two", 3),
            ]
        );
    }

    /// A table of contents in the middle of the line is listed in the middle
    /// of its own list: the order is the finished file's.
    #[test]
    fn a_table_is_listed_where_it_sits() {
        let outline = [item("One", 1, vec![]), item("Two", 2, vec![])];
        // The table sits between the two documents: page 1 stays, page 2 moves.
        let entries = entries(
            &outline,
            |page| if page > 1 { page + 1 } else { page },
            &[(2, "Contents")],
            plain,
        );
        assert_eq!(
            entries
                .iter()
                .map(|e| (e.title.as_str(), e.page))
                .collect::<Vec<_>>(),
            [("One", 1), ("Contents", 2), ("Two", 3)]
        );
    }

    /// The numbers are whatever the mapping makes of them, which is how
    /// `--page-offset` reaches the list (D40).
    #[test]
    fn the_numbers_go_through_the_mapping() {
        let outline = [item("One", 1, vec![])];
        let entries = entries(&outline, |page| page, &[], |page| page as i64 + 100);
        assert_eq!(entries[0].page, 101);
    }

    /// Nothing to list, and the table still lists itself.
    #[test]
    fn nothing_to_list_still_lists_the_table_itself() {
        let entries = entries(&[], |page| page, &[(1, "Table of Contents")], plain);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Table of Contents");
    }

    /// The shape the default stylesheet produced: a heading, then one `li` per
    /// entry carrying the title and the page, nested by level.
    #[test]
    fn the_shape_is_what_the_default_stylesheet_produced() {
        let html =
            default_document(&[entry(1, "One", 2), entry(2, "One A", 2), entry(1, "Two", 3)]);
        assert!(html.contains("<h1>Table of Contents</h1>"), "{html}");
        assert!(html.contains(">One </a><span> 2 </span></div>"), "{html}");
        assert!(html.contains(">One A </a><span> 2 </span></div>"), "{html}");
        // "One A" is inside the list belonging to "One", not a sibling of it.
        let one = html.find("One <").expect("the first entry");
        let one_a = html.find("One A").expect("the nested entry");
        let two = html.find("Two <").expect("the last entry");
        assert!(one < one_a && one_a < two, "{html}");
        assert_eq!(html.matches("<ul>").count(), 2, "one nested list: {html}");
    }

    /// Each option lands where the default stylesheet carried it.
    #[test]
    fn the_options_land_where_the_stylesheet_carried_them() {
        let toc = TocSettings {
            header_text: "Sommaire".into(),
            level_indentation: "3em".into(),
            text_size_shrink: 0.5,
            dotted_lines: false,
            links: true,
        };
        let html = document(&[entry(1, "One", 1)], &toc);
        assert!(html.contains("<h1>Sommaire</h1>"), "{html}");
        assert!(html.contains("ul ul {padding-left: 3em;}"), "{html}");
        assert!(html.contains("ul ul {font-size: 50%; }"), "{html}");
        assert!(!html.contains("border-bottom"), "{html}");
    }

    /// The defaults are the ones wkhtmltopdf's help states, and the stylesheet
    /// it dumps carries them as `1em`, `80%` and a dashed rule.
    #[test]
    fn the_defaults_are_the_stylesheets_own() {
        let html = default_document(&[entry(1, "One", 1)]);
        assert!(html.contains("ul ul {padding-left: 1em;}"), "{html}");
        assert!(html.contains("ul ul {font-size: 80%; }"), "{html}");
        assert!(
            html.contains("div {border-bottom: 1px dashed rgb(200,200,200);}"),
            "{html}"
        );
    }

    /// **The href names the heading, it does not point at it.** Only the
    /// browser knows where the line lands, so it writes the annotation and the
    /// marker is turned into a destination after the merge (D42).
    #[test]
    fn an_entry_names_the_heading_it_links_to() {
        let entry = Entry {
            level: 1,
            title: "One".into(),
            page: 12,
            target: Target {
                page: 2,
                left: 34.5,
                top: 803.25,
            },
        };
        let html = default_document(&[entry]);
        assert!(
            html.contains("href=\"rchtmltopdf-contents:2,34.5,803.25\""),
            "{html}"
        );
        // The number printed is the offset one, the target is the real page.
        assert!(html.contains("<span> 12 </span>"), "{html}");
    }

    /// `--disable-toc-links` writes the entry without an href, so the browser
    /// writes no annotation at all and there is nothing to point anywhere.
    #[test]
    fn disable_toc_links_leaves_the_entries_unlinked() {
        let toc = TocSettings {
            links: false,
            ..TocSettings::default()
        };
        let html = document(&[entry(1, "One", 2)], &toc);
        assert!(!html.contains("href"), "{html}");
        assert!(
            html.contains("<a>One </a>"),
            "the entry is still there: {html}"
        );
    }

    /// A factor that is not a round percentage keeps its digits rather than
    /// being rounded into a different size.
    #[test]
    fn a_shrink_factor_survives_as_a_percentage() {
        assert_eq!(percent(0.8), "80");
        assert_eq!(percent(1.0), "100");
        assert_eq!(percent(0.5), "50");
        assert_eq!(percent(0.755), "75.5");
        assert_eq!(percent(0.333), "33.3");
    }

    /// **A level that jumps opens one list, not two.** An `h3` directly under
    /// an `h1` is ordinary HTML, and nesting by element means two lists would
    /// indent and shrink it twice over.
    #[test]
    fn a_skipped_level_opens_one_list() {
        let html = default_document(&[entry(1, "One", 1), entry(3, "Deep", 1)]);
        assert_eq!(html.matches("<ul>").count(), 2, "{html}");
        assert!(html.contains(">Deep </a>"), "{html}");
    }

    /// Back out to a shallower level: the deeper list closes and the next
    /// entry is a sibling of the first again.
    #[test]
    fn coming_back_out_closes_the_deeper_list() {
        let html =
            default_document(&[entry(1, "One", 1), entry(2, "One A", 1), entry(1, "Two", 2)]);
        assert_eq!(html.matches("<ul>").count(), 2, "{html}");
        assert_eq!(html.matches("</ul>").count(), 2, "{html}");
    }

    /// Somebody's heading is somebody's text.
    #[test]
    fn a_title_is_escaped() {
        let html = default_document(&[entry(1, "Terms & <Conditions>", 1)]);
        assert!(html.contains("Terms &amp; &lt;Conditions&gt;"), "{html}");
        assert!(!html.contains("<Conditions>"), "{html}");
    }

    /// And so is the header text, which is also somebody's.
    #[test]
    fn the_header_text_is_escaped() {
        let toc = TocSettings {
            header_text: "R&D <draft>".into(),
            ..TocSettings::default()
        };
        let html = document(&[], &toc);
        assert!(html.contains("<h1>R&amp;D &lt;draft&gt;</h1>"), "{html}");
    }

    /// A document with no headings still produces a table of contents, with
    /// its heading and an empty list. wkhtmltopdf printed the page too.
    #[test]
    fn no_entries_is_still_a_document() {
        let html = default_document(&[]);
        assert!(html.contains("<h1>Table of Contents</h1>"), "{html}");
        assert!(html.contains("<ul>\n</ul>"), "{html}");
    }

    /// A page number is written as it comes, negative included: a
    /// `--page-offset` below zero is the user's arithmetic, not ours (D40).
    #[test]
    fn a_number_is_written_as_it_comes() {
        let html = default_document(&[entry(1, "One", -2)]);
        assert!(html.contains("<span> -2 </span>"), "{html}");
    }
}
