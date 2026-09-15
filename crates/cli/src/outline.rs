//! `--dump-outline`: the outline, in the XML wkhtmltopdf wrote.
//!
//! The shape is documented in wkhtmltopdf's own help and consumed by scripts,
//! so it is reproduced rather than tidied: a root `outline` in the
//! `http://wkhtmltopdf.org/outline` namespace, then **one `item` per object**
//! of the output — a page, a cover, a table of contents — in the order they
//! were written, each wrapping that object's headings nested as they were,
//! every element with `title`, `page`, `link` and `backLink` (D52).
//!
//! The object's own item is not a bookmark: the file's outline is the flat
//! run of headings, in wkhtmltopdf's files as in ours. It is a level the dump
//! alone has, titled with the document's `<title>` — a table of contents with
//! its caption — and numbered with the pages before it, so the first object
//! is `page="0"`. A cover, or a document `--exclude-from-outline`, keeps its
//! item with `title=""` and nothing under it.
//!
//! `link` and `backLink` named anchors wkhtmltopdf planted in the document so
//! a table of contents could point at a section and the section back at it.
//! Nothing plants those yet (#43), so both are written empty rather than
//! filled with a name nothing in the file answers to. The attributes stay,
//! because a consumer that reads them by name should find them.
//!
//! `page` is not the page of the file: it is the number `--page-offset` makes
//! of it (#37, D40, D51), which [`dump`] settles through `numbering` before
//! [`xml`] writes anything. A cover is still a page here, unlike in `[page]`.

use crate::numbering;
use rchtmltopdf_pdf::OutlineItem;
use std::fmt::Write;

/// One object of the output, as the dump needs to know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// What its item says: the document's `<title>`, the caption of a table
    /// of contents, and nothing for an object kept out of the outline.
    pub title: String,
    /// How many pages of the file it printed.
    pub pages: usize,
}

/// One element of the dump, numbered as it will be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    pub page: i64,
    pub children: Vec<Entry>,
}

/// The dump's tree: an entry per object, wrapping the headings on its pages.
///
/// `headings` is the outline of the merged file, top level first, with the
/// page of the file on every entry; each top-level heading goes under the
/// object whose pages it falls in. An entry pointing at no page at all — which
/// nothing Chromium writes does — falls in no object and is left out, rather
/// than filed under the wrong one.
pub fn dump(objects: &[Object], headings: &[OutlineItem], offset: i64) -> Vec<Entry> {
    let mut before = 0usize;
    objects
        .iter()
        .map(|object| {
            let last = before + object.pages;
            let entry = Entry {
                title: object.title.clone(),
                page: numbering::dump_object(before, offset),
                children: headings
                    .iter()
                    .filter(|heading| heading.page > before && heading.page <= last)
                    .map(|heading| numbered(heading, offset))
                    .collect(),
            };
            before = last;
            entry
        })
        .collect()
}

/// A heading and what hangs under it, with every page offset.
fn numbered(item: &OutlineItem, offset: i64) -> Entry {
    Entry {
        title: item.title.clone(),
        page: numbering::dump_page(item.page, offset),
        children: item
            .children
            .iter()
            .map(|child| numbered(child, offset))
            .collect(),
    }
}

/// The outline as a complete XML document.
pub fn xml(entries: &[Entry]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <outline xmlns=\"http://wkhtmltopdf.org/outline\">\n",
    );
    for entry in entries {
        write_entry(&mut out, entry, 1);
    }
    out.push_str("</outline>\n");
    out
}

fn write_entry(out: &mut String, entry: &Entry, depth: usize) {
    let pad = "  ".repeat(depth);
    let _ = write!(
        out,
        "{pad}<item title=\"{}\" page=\"{}\" link=\"\" backLink=\"\"",
        escape(&entry.title),
        entry.page
    );
    if entry.children.is_empty() {
        out.push_str("/>\n");
        return;
    }
    out.push_str(">\n");
    for child in &entry.children {
        write_entry(out, child, depth + 1);
    }
    let _ = writeln!(out, "{pad}</item>");
}

/// The five characters an attribute value cannot carry raw.
///
/// Control characters other than tab, newline and return are not XML 1.0
/// characters at all, in any escaping, so they are dropped: a heading that
/// carried one would otherwise make the whole document unparseable.
fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for character in raw.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(character),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, page: i64, children: Vec<Entry>) -> Entry {
        Entry {
            title: title.into(),
            page,
            children,
        }
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

    fn object(title: &str, pages: usize) -> Object {
        Object {
            title: title.into(),
            pages,
        }
    }

    #[test]
    fn the_shape_is_wkhtmltopdfs() {
        let out = xml(&[
            entry("One", 1, vec![entry("One A", 1, vec![])]),
            entry("Two", 2, vec![]),
        ]);
        assert_eq!(
            out,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <outline xmlns=\"http://wkhtmltopdf.org/outline\">\n  \
               <item title=\"One\" page=\"1\" link=\"\" backLink=\"\">\n    \
                 <item title=\"One A\" page=\"1\" link=\"\" backLink=\"\"/>\n  \
               </item>\n  \
               <item title=\"Two\" page=\"2\" link=\"\" backLink=\"\"/>\n\
             </outline>\n"
        );
    }

    #[test]
    fn an_empty_outline_is_still_a_document() {
        assert_eq!(
            xml(&[]),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <outline xmlns=\"http://wkhtmltopdf.org/outline\">\n\
             </outline>\n"
        );
    }

    /// A heading is user text, and `Terms & "Conditions" <draft>` is an
    /// ordinary one.
    #[test]
    fn a_title_is_escaped_for_an_attribute() {
        let out = xml(&[entry("Terms & \"Conditions\" <draft> 'v2'", 3, vec![])]);
        assert!(
            out.contains(
                "title=\"Terms &amp; &quot;Conditions&quot; &lt;draft&gt; &apos;v2&apos;\""
            ),
            "{out}"
        );
    }

    #[test]
    fn a_control_character_is_dropped_rather_than_written() {
        let out = xml(&[entry("A\u{0}B", 1, vec![])]);
        assert!(out.contains("title=\"AB\""), "{out}");
    }

    /// A number the offset makes negative is written as one (#37).
    #[test]
    fn a_negative_page_is_written_as_it_is() {
        let out = xml(&[entry("One", -2, vec![])]);
        assert!(out.contains("title=\"One\" page=\"-2\""), "{out}");
    }

    #[test]
    fn text_outside_ascii_is_written_as_it_is() {
        let out = xml(&[entry("Résumé — plan", 1, vec![])]);
        assert!(out.contains("title=\"Résumé — plan\""), "{out}");
    }

    /// **The measurement behind D52.** `three.html` — three `h1`, one per
    /// page — then `one.html`: wkhtmltopdf 0.12.6.1 dumps `Three` at 0 over
    /// `Alpha 1, Beta 2, Gamma 3`, then `One` at 3 over `Solo 4`.
    #[test]
    fn each_object_wraps_the_headings_on_its_own_pages() {
        let headings = [
            item("Alpha", 1, vec![]),
            item("Beta", 2, vec![item("Beta A", 2, vec![])]),
            item("Gamma", 3, vec![]),
            item("Solo", 4, vec![]),
        ];
        assert_eq!(
            dump(&[object("Three", 3), object("One", 1)], &headings, 0),
            [
                entry(
                    "Three",
                    0,
                    vec![
                        entry("Alpha", 1, vec![]),
                        entry("Beta", 2, vec![entry("Beta A", 2, vec![])]),
                        entry("Gamma", 3, vec![]),
                    ]
                ),
                entry("One", 3, vec![entry("Solo", 4, vec![])]),
            ]
        );
    }

    /// A cover or an excluded document has an item and nothing under it, and
    /// it counts its real pages in the items after it — where wkhtmltopdf
    /// counted one whatever its length, which D51 records as a defect.
    #[test]
    fn an_object_kept_out_of_the_outline_keeps_an_empty_item() {
        let headings = [item("Solo", 3, vec![])];
        assert_eq!(
            dump(&[object("", 2), object("One", 1)], &headings, 0),
            [
                entry("", 0, vec![]),
                entry("One", 2, vec![entry("Solo", 3, vec![])]),
            ]
        );
    }

    /// `--page-offset` reaches the objects' items as it reaches the headings.
    #[test]
    fn the_offset_shifts_every_entry() {
        let headings = [item("Alpha", 1, vec![]), item("Solo", 2, vec![])];
        assert_eq!(
            dump(&[object("A", 1), object("B", 1)], &headings, 10),
            [
                entry("A", 10, vec![entry("Alpha", 11, vec![])]),
                entry("B", 11, vec![entry("Solo", 12, vec![])]),
            ]
        );
    }

    /// A heading pointing at no page falls in no object.
    #[test]
    fn a_heading_that_points_nowhere_is_left_out() {
        let headings = [item("Lost", 0, vec![]), item("Found", 1, vec![])];
        assert_eq!(
            dump(&[object("A", 1)], &headings, 0),
            [entry("A", 0, vec![entry("Found", 1, vec![])])]
        );
    }

    #[test]
    fn no_objects_is_no_entries() {
        assert!(dump(&[], &[item("Ghost", 1, vec![])], 0).is_empty());
    }
}
