//! `--dump-outline`: the outline, in the XML wkhtmltopdf wrote.
//!
//! The shape is documented in wkhtmltopdf's own help and consumed by scripts,
//! so it is reproduced rather than tidied: a root `outline` in the
//! `http://wkhtmltopdf.org/outline` namespace, `item` elements nested as the
//! headings were, each with `title`, `page`, `link` and `backLink`.
//!
//! `link` and `backLink` named anchors wkhtmltopdf planted in the document so
//! a table of contents could point at a section and the section back at it.
//! Nothing plants those yet (#43), so both are written empty rather than
//! filled with a name nothing in the file answers to. The attributes stay,
//! because a consumer that reads them by name should find them.
//!
//! `page` is not the page of the file: it is the number `--page-offset` makes
//! of it, which is why [`xml`] is handed a mapping rather than reading
//! `item.page` (#37, D40). A cover is still a page here, unlike in `[page]`.

use rchtmltopdf_pdf::OutlineItem;
use std::fmt::Write;

/// The outline as a complete XML document.
///
/// `page` maps an item's page of the file to the number written for it —
/// [`crate::numbering::dump_page`] in a conversion, identity in a test that
/// does not care.
pub fn xml(items: &[OutlineItem], page: impl Fn(usize) -> i64) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <outline xmlns=\"http://wkhtmltopdf.org/outline\">\n",
    );
    for item in items {
        write_item(&mut out, item, 1, &page);
    }
    out.push_str("</outline>\n");
    out
}

fn write_item(out: &mut String, item: &OutlineItem, depth: usize, page: &impl Fn(usize) -> i64) {
    let pad = "  ".repeat(depth);
    let _ = write!(
        out,
        "{pad}<item title=\"{}\" page=\"{}\" link=\"\" backLink=\"\"",
        escape(&item.title),
        page(item.page)
    );
    if item.children.is_empty() {
        out.push_str("/>\n");
        return;
    }
    out.push_str(">\n");
    for child in &item.children {
        write_item(out, child, depth + 1, page);
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

    /// The page of the file, written as it is.
    fn identity(page: usize) -> i64 {
        page as i64
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

    #[test]
    fn the_shape_is_wkhtmltopdfs() {
        let out = xml(
            &[
                item("One", 1, vec![item("One A", 1, vec![])]),
                item("Two", 2, vec![]),
            ],
            identity,
        );
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
            xml(&[], identity),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <outline xmlns=\"http://wkhtmltopdf.org/outline\">\n\
             </outline>\n"
        );
    }

    /// A heading is user text, and `Terms & "Conditions" <draft>` is an
    /// ordinary one.
    #[test]
    fn a_title_is_escaped_for_an_attribute() {
        let out = xml(
            &[item("Terms & \"Conditions\" <draft> 'v2'", 3, vec![])],
            identity,
        );
        assert!(
            out.contains(
                "title=\"Terms &amp; &quot;Conditions&quot; &lt;draft&gt; &apos;v2&apos;\""
            ),
            "{out}"
        );
    }

    #[test]
    fn a_control_character_is_dropped_rather_than_written() {
        let out = xml(&[item("A\u{0}B", 1, vec![])], identity);
        assert!(out.contains("title=\"AB\""), "{out}");
    }

    /// Every item is written through the mapping, a nested one included, and
    /// a number it makes negative is written as one (#37).
    #[test]
    fn the_page_is_whatever_the_mapping_makes_of_it() {
        let items = [item("One", 1, vec![item("One A", 2, vec![])])];
        let out = xml(&items, |page| page as i64 + 10);
        assert!(out.contains("title=\"One\" page=\"11\""), "{out}");
        assert!(out.contains("title=\"One A\" page=\"12\""), "{out}");
        let out = xml(&items, |page| page as i64 - 3);
        assert!(out.contains("title=\"One\" page=\"-2\""), "{out}");
    }

    #[test]
    fn text_outside_ascii_is_written_as_it_is() {
        let out = xml(&[item("Résumé — plan", 1, vec![])], identity);
        assert!(out.contains("title=\"Résumé — plan\""), "{out}");
    }
}
