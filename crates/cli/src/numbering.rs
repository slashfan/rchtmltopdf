//! What the page numbers say on every page of the merged document.
//!
//! wkhtmltopdf's placeholders count in two frames at once. `[page]` and
//! `[topage]` count across the whole output; `[sitepage]` and `[sitepages]`
//! count within the document the page came from; `[frompage]` is where that
//! document started in the first frame. A cover counts in neither: the page
//! after it is page one, and `[topage]` leaves it out. `--page-offset` shifts
//! the first frame for the document it was written on.
//!
//! None of this can be decided before printing, because nobody knows how many
//! pages a document has until it has been laid out (#39). So this runs after
//! the merge, on the page counts the merge reports, and feeds the band sheets.
//!
//! # Two numbers, not one
//!
//! [`number`] answers what a band prints. [`dump_page`] answers what
//! `--dump-outline` writes, and the two part company as soon as a cover or an
//! offset is on the command line: the dump numbers a page of the file, which a
//! cover is one of, and adds the offset of the document the page belongs to.
//! Both were measured on wkhtmltopdf 0.12.6.1 (D40).
//!
//! # Sections
//!
//! `[section]`, `[subsection]` and `[subsubsection]` name the heading in force
//! on the page: the last `h1`, `h2` or `h3` at or before it, within the same
//! document. They are read from the outline Chromium wrote (D36), which is why
//! a band that uses them asks for the outline to be generated even under
//! `--no-outline`.

use rchtmltopdf_browser::placeholder::Numbers;
use rchtmltopdf_pdf::OutlineItem;

/// One document's share of the merged output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// How many pages it printed.
    pub pages: usize,
    /// Whether its pages count: a cover's do not.
    pub counted: bool,
    /// `--page-offset`, as written on this document.
    pub page_offset: i64,
}

/// The numbers for every page of the output, in order, each with the index of
/// the part it belongs to.
pub fn number(parts: &[Part], outline: &[OutlineItem]) -> Vec<(usize, Numbers)> {
    let total: i64 = parts
        .iter()
        .filter(|part| part.counted)
        .map(|part| part.pages as i64)
        .sum();
    let headings = flatten(outline);

    let mut out = Vec::new();
    let mut running = 0i64;
    let mut physical = 0usize;
    for (index, part) in parts.iter().enumerate() {
        let first_physical = physical + 1;
        let last_physical = physical + part.pages;
        let from = running + 1;
        for within in 1..=part.pages {
            physical += 1;
            if part.counted {
                running += 1;
            }
            let heading = |level: usize| -> String {
                headings
                    .iter()
                    .rfind(|(l, page, _)| {
                        *l == level
                            && *page <= physical
                            && (first_physical..=last_physical).contains(page)
                    })
                    .map(|(_, _, title)| title.clone())
                    .unwrap_or_default()
            };
            out.push((
                index,
                Numbers {
                    page: part.page_offset + running,
                    topage: part.page_offset + total,
                    frompage: part.page_offset + from,
                    sitepage: within as i64,
                    sitepages: part.pages as i64,
                    section: heading(1),
                    subsection: heading(2),
                    subsubsection: heading(3),
                },
            ));
        }
    }
    out
}

/// The number `--dump-outline` writes for a page of the merged file.
///
/// Not `[page]`, and the difference is measured rather than chosen: on
/// wkhtmltopdf 0.12.6.1 a cover counts here — the first heading after a
/// one-page cover is on page 2 in the dump and on page 1 in the footer — and
/// what shifts it is `--page-offset`, added to the physical page. The offset
/// applied is the one written on the document the page came from, which is
/// where this parts company with wkhtmltopdf and why (D40).
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

/// Every outline entry as `(level, page, title)`, in reading order.
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
            counted: true,
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

    /// A cover does not count: the page after it is page one and the total
    /// leaves it out.
    #[test]
    fn a_cover_is_not_counted() {
        let cover = Part {
            pages: 1,
            counted: false,
            page_offset: 0,
        };
        let numbered = number(&[cover, part(2)], &[]);
        assert_eq!(pages(&numbered)[1], (1, 2, 1, 1, 2));
        assert_eq!(pages(&numbered)[2], (2, 2, 1, 2, 2));
        // The cover's own numbers, should a band be written on it anyway: the
        // count as it stands, and the total as everyone else sees it.
        assert_eq!(pages(&numbered)[0], (0, 2, 1, 1, 1));
    }

    /// `--page-offset` shifts the first frame for the document it was written
    /// on, and nothing else.
    #[test]
    fn the_offset_shifts_the_document_it_was_written_on() {
        let shifted = Part {
            pages: 2,
            counted: true,
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

    /// The heading in force on a page is the last one at or before it, and a
    /// heading in another document is not in force here.
    #[test]
    fn sections_follow_the_headings_within_a_document() {
        let outline = vec![
            item(
                "One",
                1,
                vec![item("One A", 1, vec![item("Deep", 2, vec![])])],
            ),
            item("Two", 3, vec![]),
            item("Appendix", 4, vec![]),
        ];
        let numbered = number(&[part(3), part(1)], &outline);
        let sections: Vec<(&str, &str, &str)> = numbered
            .iter()
            .map(|(_, n)| {
                (
                    n.section.as_str(),
                    n.subsection.as_str(),
                    n.subsubsection.as_str(),
                )
            })
            .collect();
        assert_eq!(
            sections,
            [
                ("One", "One A", ""),
                ("One", "One A", "Deep"),
                ("Two", "One A", "Deep"),
                ("Appendix", "", ""),
            ]
        );
    }

    /// **The dump's number is not the band's.** A cover is a page of the file,
    /// so the heading after it is on page 2 there and on page 1 in the footer.
    #[test]
    fn the_dump_numbers_a_page_of_the_file_and_a_cover_is_one() {
        let cover = Part {
            pages: 1,
            counted: false,
            page_offset: 0,
        };
        let parts = [cover, part(2)];
        assert_eq!(
            [
                dump_page(&parts, 1),
                dump_page(&parts, 2),
                dump_page(&parts, 3)
            ],
            [1, 2, 3]
        );
        // What the band prints on those same three pages.
        assert_eq!(pages(&number(&parts, &[]))[1].0, 1);
    }

    /// The offset is the one written on the document the page came from, so
    /// two documents shift by their own.
    #[test]
    fn the_dump_adds_the_offset_of_the_documents_own_pages() {
        let parts = [
            Part {
                pages: 2,
                counted: true,
                page_offset: 10,
            },
            Part {
                pages: 1,
                counted: true,
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
            counted: true,
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
