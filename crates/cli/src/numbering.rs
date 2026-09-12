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

    #[test]
    fn nothing_to_number_is_nothing() {
        assert!(number(&[], &[]).is_empty());
        assert!(number(&[part(0)], &[]).is_empty());
    }
}
