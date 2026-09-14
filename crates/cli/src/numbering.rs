//! What the page numbers say on every page of the merged document.
//!
//! wkhtmltopdf's placeholders count in two frames at once. `[page]` and
//! `[topage]` count across the whole output; `[sitepage]` and `[sitepages]`
//! count within the document the page came from; `[frompage]` is where that
//! document started in the first frame. A cover counts like anything else —
//! the page behind it is page two (D45) — it is only the band it does not
//! get. `--page-offset` shifts the first frame for the document it was
//! written on.
//!
//! None of this can be decided before printing, because nobody knows how many
//! pages a document has until it has been laid out (#39). So this runs after
//! the merge, on the page counts the merge reports, and feeds the band sheets.
//!
//! # Two questions, one number
//!
//! [`number`] answers what a band prints. [`dump_page`] answers what
//! `--dump-outline` writes. D40 had them parting company over a cover, which
//! the dump counted and the band did not; measuring wkhtmltopdf 0.12.6.1
//! again showed the band counting it too (D45), and the number both answer
//! with is now the page of the file plus the offset of the document it came
//! from. They stay two functions because they are two questions — #44 has the
//! cross-document offset still to settle, and the answers may part again —
//! but a case where they differ today would be a bug in one of them.
//!
//! # Sections
//!
//! `[section]`, `[subsection]` and `[subsubsection]` name the heading in force
//! on the page, and wkhtmltopdf's rule for that is narrower than it looks
//! (D47): for each level, the **first** heading that begins on a page is the
//! one that page names, and a page where none begins keeps what the page
//! before it named — across a document boundary as well. So two `h2` on one
//! page name the first, and the page after them still names the first.
//!
//! They are read from the outline Chromium wrote (D36), which is why a band
//! that uses them asks for the outline to be generated even under
//! `--no-outline`.

use rchtmltopdf_browser::placeholder::Numbers;
use rchtmltopdf_pdf::OutlineItem;

/// One document's share of the merged output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// How many pages it printed.
    pub pages: usize,
    /// `--page-offset`, as written on this document.
    pub page_offset: i64,
}

/// The numbers for every page of the output, in order, each with the index of
/// the part it belongs to.
pub fn number(parts: &[Part], outline: &[OutlineItem]) -> Vec<(usize, Numbers)> {
    let total: i64 = parts.iter().map(|part| part.pages as i64).sum();
    let headings = flatten(outline);

    // wkhtmltopdf's header/footer cache, one slot per level, carried from page
    // to page and never reset (D47). A page fills a slot only if the slot is
    // still the one before it, which is what makes the *first* heading on the
    // page win over the ones after it.
    let mut in_force = [String::new(), String::new(), String::new()];

    let mut out = Vec::new();
    let mut physical = 0usize;
    for (index, part) in parts.iter().enumerate() {
        let from = (physical + 1) as i64;
        for within in 1..=part.pages {
            physical += 1;
            for (level, slot) in in_force.iter_mut().enumerate() {
                if let Some((_, _, title)) = headings
                    .iter()
                    .find(|(l, page, _)| *l == level + 1 && *page == physical)
                {
                    slot.clone_from(title);
                }
            }
            out.push((
                index,
                Numbers {
                    page: part.page_offset + physical as i64,
                    topage: part.page_offset + total,
                    frompage: part.page_offset + from,
                    sitepage: within as i64,
                    sitepages: part.pages as i64,
                    section: in_force[0].clone(),
                    subsection: in_force[1].clone(),
                    subsubsection: in_force[2].clone(),
                },
            ));
        }
    }
    out
}

/// The number `--dump-outline` writes for a page of the merged file.
///
/// The page of the file plus `--page-offset`, which is what [`number`] answers
/// too since a cover was measured counting in both (D45). The offset applied
/// is the one written on the document the page came from, which is where this
/// parts company with wkhtmltopdf and why (D40).
///
/// `physical` is 1-based. A page past the last part keeps the last part's
/// offset; the outline is read off the merged file, so there is none.
pub fn dump_page(parts: &[Part], physical: usize) -> i64 {
    let mut seen = 0usize;
    let mut offset = 0i64;
    for part in parts {
        offset = part.page_offset;
        seen += part.pages;
        if physical <= seen {
            break;
        }
    }
    physical as i64 + offset
}

/// Every outline entry as `(level, page, title)`, in **reading order**, which
/// is the order the cache in [`number`] depends on: the first entry it finds
/// for a page is the first heading that begins there.
fn flatten(items: &[OutlineItem]) -> Vec<(usize, usize, String)> {
    fn walk(items: &[OutlineItem], level: usize, out: &mut Vec<(usize, usize, String)>) {
        for item in items {
            out.push((level, item.page, item.title.clone()));
            walk(&item.children, level + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(items, 1, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(pages: usize) -> Part {
        Part {
            pages,
            page_offset: 0,
        }
    }

    fn pages(numbered: &[(usize, Numbers)]) -> Vec<(i64, i64, i64, i64, i64)> {
        numbered
            .iter()
            .map(|(_, n)| (n.page, n.topage, n.frompage, n.sitepage, n.sitepages))
            .collect()
    }

    /// **The two frames.** `[page]` runs across both documents, `[sitepage]`
    /// restarts, and `[frompage]` says where the second one began.
    #[test]
    fn the_numbers_run_across_documents_and_restart_within_them() {
        let numbered = number(&[part(3), part(2)], &[]);
        assert_eq!(
            pages(&numbered),
            [
                (1, 5, 1, 1, 3),
                (2, 5, 1, 2, 3),
                (3, 5, 1, 3, 3),
                (4, 5, 4, 1, 2),
                (5, 5, 4, 2, 2),
            ]
        );
        let parts: Vec<usize> = numbered.iter().map(|(part, _)| *part).collect();
        assert_eq!(parts, [0, 0, 0, 1, 1]);
    }

    /// **A cover counts** (D45): the page behind it is page two, the total
    /// includes it, and the document behind it began on page two.
    #[test]
    fn a_cover_counts_like_any_other_page() {
        let numbered = number(&[part(1), part(2)], &[]);
        assert_eq!(pages(&numbered)[1], (2, 3, 2, 1, 2));
        assert_eq!(pages(&numbered)[2], (3, 3, 2, 2, 2));
        // The cover's own numbers, should a band be written on it anyway. It
        // never is: `cover` clears the bands before the object's own options
        // are read.
        assert_eq!(pages(&numbered)[0], (1, 3, 1, 1, 1));
    }

    /// `--page-offset` shifts the first frame for the document it was written
    /// on, and nothing else.
    #[test]
    fn the_offset_shifts_the_document_it_was_written_on() {
        let shifted = Part {
            pages: 2,
            page_offset: 10,
        };
        let numbered = number(&[part(1), shifted], &[]);
        assert_eq!(pages(&numbered)[0], (1, 3, 1, 1, 1));
        assert_eq!(pages(&numbered)[1], (12, 13, 12, 1, 2));
        assert_eq!(pages(&numbered)[2], (13, 13, 12, 2, 2));
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

    fn sections(numbered: &[(usize, Numbers)]) -> Vec<(&str, &str, &str)> {
        numbered
            .iter()
            .map(|(_, n)| {
                (
                    n.section.as_str(),
                    n.subsection.as_str(),
                    n.subsubsection.as_str(),
                )
            })
            .collect()
    }

    /// **The first heading on the page wins, and the page after it keeps that
    /// one** (D47). Two `h2` and two `h3` begin on page 1; the band names
    /// `One A` and `Deep`, not the last of each. Page 2 begins no heading at
    /// all, so it names what page 1 named — `One A` again, not `One B`.
    #[test]
    fn a_page_names_the_first_heading_that_begins_on_it() {
        let outline = vec![
            item(
                "One",
                1,
                vec![
                    item(
                        "One A",
                        1,
                        vec![item("Deep", 1, vec![]), item("Deeper", 1, vec![])],
                    ),
                    item("One B", 1, vec![]),
                ],
            ),
            item("Two", 3, vec![]),
        ];
        let numbered = number(&[part(3)], &outline);
        assert_eq!(
            sections(&numbered),
            [
                ("One", "One A", "Deep"),
                ("One", "One A", "Deep"),
                ("Two", "One A", "Deep"),
            ]
        );
    }

    /// **Nothing resets at a document boundary** (D47). The second document
    /// brings its own `h1` and nothing below it, so its page names that `h1`
    /// and keeps the `h2` and `h3` the first document left in force.
    #[test]
    fn a_heading_stays_in_force_into_the_next_document() {
        let outline = vec![
            item("One", 1, vec![item("One A", 1, vec![])]),
            item("Appendix", 2, vec![]),
        ];
        let numbered = number(&[part(1), part(1)], &outline);
        assert_eq!(
            sections(&numbered),
            [("One", "One A", ""), ("Appendix", "One A", "")]
        );
    }

    /// **The dump and the band agree about a cover** (D45). Both number a page
    /// of the file, so the heading after a one-page cover is on page 2 in the
    /// dump and the footer under it says 2 as well.
    #[test]
    fn the_dump_numbers_a_page_of_the_file_and_so_does_the_band() {
        let parts = [part(1), part(2)];
        assert_eq!(
            [
                dump_page(&parts, 1),
                dump_page(&parts, 2),
                dump_page(&parts, 3)
            ],
            [1, 2, 3]
        );
        // What the band prints on those same three pages.
        let numbered = number(&parts, &[]);
        assert_eq!(
            [
                pages(&numbered)[0].0,
                pages(&numbered)[1].0,
                pages(&numbered)[2].0
            ],
            [1, 2, 3]
        );
    }

    /// The offset is the one written on the document the page came from, so
    /// two documents shift by their own.
    #[test]
    fn the_dump_adds_the_offset_of_the_documents_own_pages() {
        let parts = [
            Part {
                pages: 2,
                page_offset: 10,
            },
            Part {
                pages: 1,
                page_offset: 100,
            },
        ];
        let numbered: Vec<i64> = (1..=3).map(|page| dump_page(&parts, page)).collect();
        assert_eq!(numbered, [11, 12, 103]);
    }

    /// No offset anywhere leaves the physical page alone, which is what every
    /// dump said before #37.
    #[test]
    fn the_dump_without_an_offset_is_the_physical_page() {
        let parts = [part(2), part(1)];
        let numbered: Vec<i64> = (1..=3).map(|page| dump_page(&parts, page)).collect();
        assert_eq!(numbered, [1, 2, 3]);
        assert_eq!(dump_page(&[], 1), 1);
    }

    /// A negative offset is written as the negative number it comes to.
    /// wkhtmltopdf underflows here and prints 4294967295; that is a bug, not a
    /// format (D40).
    #[test]
    fn a_negative_offset_goes_below_zero_rather_than_wrapping() {
        let parts = [Part {
            pages: 2,
            page_offset: -3,
        }];
        assert_eq!([dump_page(&parts, 1), dump_page(&parts, 2)], [-2, -1]);
    }

    #[test]
    fn nothing_to_number_is_nothing() {
        assert!(number(&[], &[]).is_empty());
        assert!(number(&[part(0)], &[]).is_empty());
    }
}
