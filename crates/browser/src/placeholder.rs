//! wkhtmltopdf's `[page]`, `[date]` and the rest, expanded into a band.
//!
//! # Why none of these are Chromium's
//!
//! A print template understands five classes of its own — `pageNumber`,
//! `totalPages`, `date`, `title`, `url` — and none of them is used. Three are a
//! different program's idea of the answer: Chromium's date format is not
//! wkhtmltopdf's, and its `title` and `url` come from the page rather than
//! from the command line, where `--title` lives. The two page numbers were
//! used until D38, and could count only within the document being printed:
//! `[page]` restarted at one for every document of a conversion. Now the
//! bands are printed after everything else, when the counts are known, and
//! every number arrives here in [`Numbers`], worked out by the conversion.
//!
//! # Escaping happens here
//!
//! The text is somebody's document title and the result is HTML, so each
//! literal run and each substituted value is escaped on the way. Nothing
//! reaches the output unescaped.

use rchtmltopdf_core::Clock;
use rchtmltopdf_core::settings::Pair;

/// What a band's text is expanded against.
///
/// Owns its strings. It is carried inside the conversion plan, which is compared
/// in tests and has to outlive the settings it was built from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// The document as it was written on the command line, for `[webpage]`.
    pub webpage: String,
    /// `--title`, for `[title]` and `[doctitle]`.
    pub title: String,
    /// `--replace`, which defines placeholders of the user's own and is
    /// consulted before any of the built-in ones.
    pub replacements: Vec<Pair>,
    /// Read once per conversion and carried, so a document whose header and
    /// footer both say `[time]` cannot print two different times.
    pub clock: Clock,
}

/// What one page's numbers are, in wkhtmltopdf's two frames (#39).
///
/// `page` and `topage` count across the whole output; `sitepage` and
/// `sitepages` within the document the page came from; `frompage` is where
/// that document began in the first frame. A cover is in neither count. The
/// three sections name the heading in force on the page, from the outline.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Numbers {
    pub page: i64,
    pub topage: i64,
    pub frompage: i64,
    pub sitepage: i64,
    pub sitepages: i64,
    pub section: String,
    pub subsection: String,
    pub subsubsection: String,
}

/// The placeholders that need the page counts, and so the outline for the
/// three that name a heading. Used to decide whether a band asks for the
/// outline to be generated at all.
pub const SECTION_PLACEHOLDERS: &[&str] = &["section", "subsection", "subsubsection"];

/// Whether a band's text names a heading.
pub fn names_a_section(text: &str) -> bool {
    SECTION_PLACEHOLDERS
        .iter()
        .any(|name| text.contains(&format!("[{name}]")))
}

/// Expand one band cell into markup.
pub fn expand(text: &str, context: &Context, numbers: &Numbers) -> String {
    let mut html = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(open) = rest.find('[') {
        html.push_str(&escape(&rest[..open]));
        let after = &rest[open + 1..];

        let Some(close) = after.find(']') else {
            // An unclosed bracket is text. Somebody's footer says "[draft".
            html.push_str(&escape(&rest[open..]));
            return html;
        };

        let name = &after[..close];
        match resolve(name, context, numbers) {
            Some(value) => html.push_str(&escape(&value)),
            // Not a placeholder at all: `[see note]` is text.
            None => html.push_str(&escape(&rest[open..open + close + 2])),
        }
        rest = &after[close + 1..];
    }

    html.push_str(&escape(rest));
    html
}

fn resolve(name: &str, context: &Context, numbers: &Numbers) -> Option<String> {
    // `--replace` first, so a user who defines `[page]` gets their own answer.
    // Its value is inserted literally and never rescanned, so a replacement
    // containing `[page]` prints those six characters.
    if let Some(pair) = context.replacements.iter().find(|pair| pair.name == name) {
        return Some(pair.value.clone());
    }

    Some(match name {
        // --- the counts, worked out after printing (#39) ----------------------
        "page" => numbers.page.to_string(),
        "topage" => numbers.topage.to_string(),
        "frompage" => numbers.frompage.to_string(),
        "sitepage" => numbers.sitepage.to_string(),
        "sitepages" => numbers.sitepages.to_string(),
        "section" => numbers.section.clone(),
        "subsection" => numbers.subsection.clone(),
        "subsubsection" => numbers.subsubsection.clone(),

        // --- from the command line ------------------------------------------
        "webpage" => context.webpage.clone(),
        "title" | "doctitle" => context.title.clone(),

        // --- rendered here, not by the browser ---------------------------------
        "date" => context.clock.date(),
        "isodate" => context.clock.iso(),
        "time" => context.clock.time(),

        _ => return None,
    })
}

/// HTML-escape. A band's text is somebody's document title.
pub(crate) fn escape(raw: &str) -> String {
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

    fn clock() -> Clock {
        Clock {
            year: 2026,
            month: 9,
            day: 12,
            hour: 14,
            minute: 5,
            second: 9,
            utc_offset_seconds: 2 * 3600,
        }
    }

    fn context(replacements: &[Pair]) -> Context {
        Context {
            webpage: "https://example.com/invoice".into(),
            title: "Invoice 42".into(),
            replacements: replacements.to_vec(),
            clock: clock(),
        }
    }

    fn numbers() -> Numbers {
        Numbers {
            page: 4,
            topage: 5,
            frompage: 4,
            sitepage: 1,
            sitepages: 2,
            section: "Chapter Two".into(),
            subsection: "Section 2.1".into(),
            subsubsection: String::new(),
        }
    }

    fn html(text: &str) -> String {
        expand(text, &context(&[]), &numbers())
    }

    /// The literal example from the brief, from `grammar.rs`, and from the
    /// README's own first code block.
    #[test]
    fn the_example_everything_quotes_expands() {
        assert_eq!(html("Page [page] / [topage]"), "Page 4 / 5");
    }

    /// Two frames: across the output, and within the document (#39).
    #[test]
    fn the_counts_come_from_the_conversion_in_both_frames() {
        assert_eq!(html("[page]/[topage]"), "4/5");
        assert_eq!(html("[sitepage]/[sitepages]"), "1/2");
        assert_eq!(html("[frompage]"), "4");
    }

    /// The heading in force on the page, and nothing where there is none.
    #[test]
    fn the_sections_name_the_headings_in_force() {
        assert_eq!(
            html("[section] / [subsection] / [subsubsection]"),
            "Chapter Two / Section 2.1 / "
        );
        assert!(names_a_section("in [section] here"));
        assert!(names_a_section("[subsubsection]"));
        assert!(!names_a_section("[page] of [topage]"));
        assert!(!names_a_section("section"));
    }

    #[test]
    fn the_command_line_answers_the_rest() {
        assert_eq!(html("[webpage]"), "https://example.com/invoice");
        assert_eq!(html("[title]"), "Invoice 42");
        assert_eq!(html("[doctitle]"), "Invoice 42");
        assert_eq!(html("[date]"), "2026-09-12");
        assert_eq!(html("[time]"), "14:05:09");
        assert_eq!(html("[isodate]"), "2026-09-12T14:05:09+02:00");
    }

    #[test]
    fn a_western_offset_is_signed_the_other_way() {
        let behind = Clock {
            utc_offset_seconds: -(5 * 3600 + 30 * 60),
            ..clock()
        };
        assert!(behind.iso().ends_with("-05:30"), "{}", behind.iso());
    }

    /// `--replace` defines placeholders of the user's own, and is consulted
    /// before the built-in ones so it can override them.
    #[test]
    fn replace_defines_placeholders_and_wins() {
        let replacements = [
            Pair {
                name: "client".into(),
                value: "Acme Ltd".into(),
            },
            Pair {
                name: "page".into(),
                value: "none of your business".into(),
            },
        ];
        let expansion = expand("[client] [page]", &context(&replacements), &numbers());
        assert_eq!(expansion, "Acme Ltd none of your business");
    }

    /// Literal, not a pattern: a replacement value is inserted as it is and
    /// never rescanned, so it cannot expand to something else.
    #[test]
    fn a_replacement_value_is_not_expanded_again() {
        let replacements = [Pair {
            name: "x".into(),
            value: "[page]".into(),
        }];
        let expansion = expand("[x]", &context(&replacements), &numbers());
        assert_eq!(expansion, "[page]");
    }

    /// Everything substituted is escaped. A title with an ampersand in it is
    /// ordinary, and so is a heading.
    #[test]
    fn substituted_values_are_escaped() {
        let context = Context {
            title: "Tom & Jerry <b>".into(),
            ..context(&[])
        };
        assert_eq!(
            expand("[title]", &context, &numbers()),
            "Tom &amp; Jerry &lt;b&gt;"
        );
        let numbers = Numbers {
            section: "Terms & <Conditions>".into(),
            ..numbers()
        };
        assert_eq!(
            expand("[section]", &context, &numbers),
            "Terms &amp; &lt;Conditions&gt;"
        );
    }

    #[test]
    fn a_replacement_value_is_escaped_too() {
        let replacements = [Pair {
            name: "x".into(),
            value: "<script>alert(1)</script>".into(),
        }];
        let expansion = expand("[x]", &context(&replacements), &numbers());
        assert!(!expansion.contains("<script>"), "{expansion}");
    }

    /// Brackets are ordinary punctuation in a footer. Anything that is not a
    /// placeholder stays as it was written.
    #[test]
    fn text_that_merely_looks_like_a_placeholder_is_text() {
        assert_eq!(html("[see note]"), "[see note]");
        assert_eq!(html("[draft"), "[draft");
        assert_eq!(html("]stray["), "]stray[");
        assert_eq!(html("[]"), "[]");
    }

    #[test]
    fn text_around_a_placeholder_survives() {
        assert_eq!(html("v1.2 — [frompage] of many"), "v1.2 — 4 of many");
        assert_eq!(html(""), "");
        assert_eq!(html("no placeholders here"), "no placeholders here");
    }

    /// A title given to `--title` is not there by default, and an empty
    /// expansion is better than the word "title" printed on every page.
    #[test]
    fn an_unset_title_expands_to_nothing() {
        let context = Context {
            title: String::new(),
            ..context(&[])
        };
        assert_eq!(expand("[title]", &context, &numbers()), "");
    }
}
