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
//! `link` and `backLink` name the anchors wkhtmltopdf planted in the document
//! so a table of contents could point at a section and the section back at
//! it: `__WKANCHOR_` and a counter in base 36, two per item, handed out in
//! reading order across the whole conversion (D56). Its default stylesheet
//! tests for the attribute, so a dump without them makes a table without
//! links. The names are reproduced exactly, quirks included: the page objects
//! are numbered first, in command-line order, then the tables of contents,
//! and a table's items carry the same name in both attributes, because
//! wkhtmltopdf re-renders a table until it settles and copies the first name
//! into both on the second pass. An object kept out of the outline has no
//! anchors and consumes no numbers.
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
    pub role: Role,
}

/// What an object is to the outline, which decides its anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A page in the outline: named first, in command-line order.
    Document,
    /// A table of contents: named after every document, each item with the
    /// same name in both attributes.
    Contents,
    /// A cover, or `--exclude-from-outline`: an item with nothing in it and
    /// no anchors.
    Excluded,
}

/// One element of the dump, numbered and named as it will be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    pub page: i64,
    pub link: String,
    pub back_link: String,
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
    let mut entries: Vec<Entry> = objects
        .iter()
        .map(|object| {
            let last = before + object.pages;
            let entry = Entry {
                title: object.title.clone(),
                page: numbering::dump_object(before, offset),
                link: String::new(),
                back_link: String::new(),
                children: headings
                    .iter()
                    .filter(|heading| heading.page > before && heading.page <= last)
                    .map(|heading| numbered(heading, offset))
                    .collect(),
            };
            before = last;
            entry
        })
        .collect();

    // The anchors, in the order wkhtmltopdf handed them out: every document
    // as it was preprocessed, then every table of contents as it was built.
    let mut counter = 0u64;
    for role in [Role::Document, Role::Contents] {
        for (object, entry) in objects.iter().zip(entries.iter_mut()) {
            if object.role == role {
                name(entry, role, &mut counter);
            }
        }
    }
    entries
}

/// Two names per item, in reading order, from a counter that never resets.
///
/// A table of contents takes the first of its two into both attributes:
/// wkhtmltopdf builds a table, measures it and builds it again until its
/// length settles, and the second build copies `anchor` into `tocAnchor`
/// (`OutlineItem::fillAnchors`, `outline.cc` 0.12.6). The second number is
/// still consumed.
fn name(entry: &mut Entry, role: Role, counter: &mut u64) {
    let first = anchor(*counter);
    let second = anchor(*counter + 1);
    *counter += 2;
    entry.link = first.clone();
    entry.back_link = match role {
        Role::Contents => first,
        Role::Document | Role::Excluded => second,
    };
    for child in &mut entry.children {
        name(child, role, counter);
    }
}

/// `__WKANCHOR_` and the number in base 36, lowercase: `QString::number(n, 36)`.
fn anchor(number: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut digits = Vec::new();
    let mut rest = number;
    loop {
        digits.push(DIGITS[(rest % 36) as usize] as char);
        rest /= 36;
        if rest == 0 {
            break;
        }
    }
    let mut out = String::from("__WKANCHOR_");
    out.extend(digits.iter().rev());
    out
}

/// A heading and what hangs under it, with every page offset.
fn numbered(item: &OutlineItem, offset: i64) -> Entry {
    Entry {
        title: item.title.clone(),
        page: numbering::dump_page(item.page, offset),
        link: String::new(),
        back_link: String::new(),
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
        "{pad}<item title=\"{}\" page=\"{}\" link=\"{}\" backLink=\"{}\"",
        escape(&entry.title),
        entry.page,
        escape(&entry.link),
        escape(&entry.back_link)
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
            link: String::new(),
            back_link: String::new(),
            children,
        }
    }

    /// An entry with its anchors, as a document's are: two numbers.
    fn linked(title: &str, page: i64, first: u64, children: Vec<Entry>) -> Entry {
        Entry {
            link: anchor(first),
            back_link: anchor(first + 1),
            ..entry(title, page, children)
        }
    }

    /// The anchors as a table of contents' are: the first number in both.
    fn aliased(title: &str, page: i64, first: u64, children: Vec<Entry>) -> Entry {
        Entry {
            link: anchor(first),
            back_link: anchor(first),
            ..entry(title, page, children)
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
            role: Role::Document,
        }
    }

    fn excluded(pages: usize) -> Object {
        Object {
            title: String::new(),
            pages,
            role: Role::Excluded,
        }
    }

    fn contents(pages: usize) -> Object {
        Object {
            title: "Table of Contents".into(),
            pages,
            role: Role::Contents,
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
                linked(
                    "Three",
                    0,
                    0,
                    vec![
                        linked("Alpha", 1, 2, vec![]),
                        linked("Beta", 2, 4, vec![linked("Beta A", 2, 6, vec![])]),
                        linked("Gamma", 3, 8, vec![]),
                    ]
                ),
                linked("One", 3, 10, vec![linked("Solo", 4, 12, vec![])]),
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
            dump(&[excluded(2), object("One", 1)], &headings, 0),
            [
                entry("", 0, vec![]),
                linked("One", 2, 0, vec![linked("Solo", 3, 2, vec![])]),
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
                linked("A", 10, 0, vec![linked("Alpha", 11, 2, vec![])]),
                linked("B", 11, 4, vec![linked("Solo", 12, 6, vec![])]),
            ]
        );
    }

    /// A heading pointing at no page falls in no object.
    #[test]
    fn a_heading_that_points_nowhere_is_left_out() {
        let headings = [item("Lost", 0, vec![]), item("Found", 1, vec![])];
        assert_eq!(
            dump(&[object("A", 1)], &headings, 0),
            [linked("A", 0, 0, vec![linked("Found", 1, 2, vec![])])]
        );
    }

    /// **The anchors, as measured** (D56). `one.html toc three.html` on
    /// wkhtmltopdf 0.12.6.1: `One` 0/1 and `Solo` 2/3, then `Three` 4/5 over
    /// `Alpha` 6/7, `Beta` 8/9, `Gamma` a/b — the table in between is named
    /// **after** them, `c/c` for its item and `e/e` for its heading, the
    /// second of each pair consumed and unused.
    #[test]
    fn a_table_of_contents_is_named_after_the_documents_with_one_name_twice() {
        let headings = [
            item("Solo", 1, vec![]),
            item("Table of Contents", 2, vec![]),
            item("Alpha", 3, vec![]),
            item("Beta", 4, vec![]),
            item("Gamma", 5, vec![]),
        ];
        assert_eq!(
            dump(
                &[object("One", 1), contents(1), object("Three", 3)],
                &headings,
                0
            ),
            [
                linked("One", 0, 0, vec![linked("Solo", 1, 2, vec![])]),
                aliased(
                    "Table of Contents",
                    1,
                    12,
                    vec![aliased("Table of Contents", 2, 14, vec![])]
                ),
                linked(
                    "Three",
                    2,
                    4,
                    vec![
                        linked("Alpha", 3, 6, vec![]),
                        linked("Beta", 4, 8, vec![]),
                        linked("Gamma", 5, 10, vec![]),
                    ]
                ),
            ]
        );
    }

    /// Base 36, lowercase, as `QString::number(n, 36)` writes it.
    #[test]
    fn anchors_count_in_base_36() {
        assert_eq!(anchor(0), "__WKANCHOR_0");
        assert_eq!(anchor(10), "__WKANCHOR_a");
        assert_eq!(anchor(35), "__WKANCHOR_z");
        assert_eq!(anchor(36), "__WKANCHOR_10");
        assert_eq!(anchor(1295), "__WKANCHOR_zz");
    }

    #[test]
    fn the_anchors_are_written_as_attributes() {
        let out = xml(&[linked("One", 1, 0, vec![])]);
        assert!(
            out.contains("link=\"__WKANCHOR_0\" backLink=\"__WKANCHOR_1\""),
            "{out}"
        );
    }

    #[test]
    fn no_objects_is_no_entries() {
        assert!(dump(&[], &[item("Ghost", 1, vec![])], 0).is_empty());
    }
}
