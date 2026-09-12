//! wkhtmltopdf's `[page]`, `[date]` and the rest, expanded into a band.
//!
//! # Why these are not Chromium's
//!
//! A print template understands five classes of its own — `pageNumber`,
//! `totalPages`, `date`, `title`, `url` — and only the first two are used here.
//! The other three are a different program's idea of the answer: Chromium's date
//! format is not wkhtmltopdf's, and its `title` and `url` come from the page
//! rather than from the command line, where `--title` lives. Everything except
//! the page numbers is therefore rendered here, from settings.
//!
//! The page numbers cannot be. Nobody knows how many pages a document has until
//! it has been laid out, and that happens inside the print call, so `[page]` and
//! `[topage]` become the two spans Chromium fills in for itself.
//!
//! # Escaping happens here, not around here
//!
//! The text is somebody's document title and the result is HTML, so the obvious
//! move is to escape the lot. That cannot work: `[page]` has to come out as a
//! `<span>`, and a `&lt;span&gt;` prints as itself. So the text is walked
//! instead, escaping each literal run and each substituted value, and inserting
//! the spans raw. Nothing reaches the output unescaped except markup this module
//! wrote.

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

/// The result of expanding one band cell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expansion {
    /// Ready to drop into a template.
    pub html: String,
    /// Placeholders that are real in wkhtmltopdf and empty here, in the order
    /// they were met. The binary says so once per name.
    pub unsupported: Vec<&'static str>,
}

/// Placeholders that need something V1 does not have.
///
/// All three name a position in the document outline, which is #40's work and
/// does not exist until a table of contents does. They expand to nothing rather
/// than to their own name, because a footer reading `[section]` on every page of
/// a printed invoice is worse than a blank.
const NEEDS_AN_OUTLINE: &[&str] = &["section", "subsection", "subsubsection"];

/// Expand one band cell.
pub fn expand(text: &str, context: &Context) -> Expansion {
    let mut expansion = Expansion::default();
    let mut rest = text;

    while let Some(open) = rest.find('[') {
        expansion.html.push_str(&escape(&rest[..open]));
        let after = &rest[open + 1..];

        let Some(close) = after.find(']') else {
            // An unclosed bracket is text. Somebody's footer says "[draft".
            expansion.html.push_str(&escape(&rest[open..]));
            return expansion;
        };

        let name = &after[..close];
        match resolve(name, context) {
            Some(Resolved::Text(value)) => expansion.html.push_str(&escape(&value)),
            Some(Resolved::Markup(html)) => expansion.html.push_str(html),
            Some(Resolved::Nothing(known)) => {
                if !expansion.unsupported.contains(&known) {
                    expansion.unsupported.push(known);
                }
            }
            // Not a placeholder at all: `[see note]` is text.
            None => expansion
                .html
                .push_str(&escape(&rest[open..open + close + 2])),
        }
        rest = &after[close + 1..];
    }

    expansion.html.push_str(&escape(rest));
    expansion
}

enum Resolved {
    /// A value to escape and insert.
    Text(String),
    /// Markup this module wrote, inserted as it is.
    Markup(&'static str),
    /// Recognised, and nothing to show for it yet.
    Nothing(&'static str),
}

fn resolve(name: &str, context: &Context) -> Option<Resolved> {
    // `--replace` first, so a user who defines `[page]` gets their own answer.
    // Its value is inserted literally and never rescanned, so a replacement
    // containing `[page]` prints those six characters.
    if let Some(pair) = context.replacements.iter().find(|pair| pair.name == name) {
        return Some(Resolved::Text(pair.value.clone()));
    }

    if let Some(known) = NEEDS_AN_OUTLINE.iter().find(|known| **known == name) {
        return Some(Resolved::Nothing(known));
    }

    Some(match name {
        // --- the two only the browser can answer ----------------------------
        //
        // **V2 has to split this.** With one document, the page number within
        // the document and within the whole file are the same number, and so are
        // the two totals. They stop being the same the moment `--page-offset` or
        // a second document exists, and then `[page]` counts across the file
        // while `[sitepage]` counts within the document.
        "page" | "sitepage" => Resolved::Markup("<span class=\"pageNumber\"></span>"),
        "topage" | "sitepages" => Resolved::Markup("<span class=\"totalPages\"></span>"),
        // One document starts at its first page.
        "frompage" => Resolved::Text("1".to_string()),

        // --- from the command line ------------------------------------------
        "webpage" => Resolved::Text(context.webpage.clone()),
        "title" | "doctitle" => Resolved::Text(context.title.clone()),

        // --- rendered here, not by the template ------------------------------
        "date" => Resolved::Text(context.clock.date()),
        "isodate" => Resolved::Text(context.clock.iso()),
        "time" => Resolved::Text(context.clock.time()),

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

    fn html(text: &str) -> String {
        expand(text, &context(&[])).html
    }

    /// The literal example from the brief, from `grammar.rs`, and from the
    /// README's own first code block.
    #[test]
    fn the_example_everything_quotes_expands() {
        assert_eq!(
            html("Page [page] / [topage]"),
            "Page <span class=\"pageNumber\"></span> / <span class=\"totalPages\"></span>"
        );
    }

    /// Nobody knows the page count until the document has been laid out, which
    /// happens inside the print call. These two are the only placeholders
    /// Chromium has to answer.
    #[test]
    fn only_the_page_numbers_are_left_to_the_browser() {
        assert!(html("[page]").contains("class=\"pageNumber\""));
        assert!(html("[topage]").contains("class=\"totalPages\""));
        // With one document these are the same numbers. V2 splits them.
        assert_eq!(html("[sitepage]"), html("[page]"));
        assert_eq!(html("[sitepages]"), html("[topage]"));
        assert_eq!(html("[frompage]"), "1");
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

    /// The three that name a position in the outline. Empty rather than their
    /// own name: a footer reading `[section]` on every page is worse than a
    /// blank.
    #[test]
    fn what_needs_an_outline_expands_to_nothing_and_says_so() {
        let expansion = expand("a[section]b[subsection]c", &context(&[]));
        assert_eq!(expansion.html, "abc");
        assert_eq!(expansion.unsupported, ["section", "subsection"]);
    }

    /// One line per name however many times it appears, because the alternative
    /// is a page of identical warnings.
    #[test]
    fn an_unsupported_placeholder_is_reported_once() {
        let expansion = expand("[section] [section] [section]", &context(&[]));
        assert_eq!(expansion.unsupported, ["section"]);
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
        let expansion = expand("[client] [page]", &context(&replacements));
        assert_eq!(expansion.html, "Acme Ltd none of your business");
    }

    /// Literal, not a pattern: a replacement value is inserted as it is and
    /// never rescanned, so it cannot expand to something else.
    #[test]
    fn a_replacement_value_is_not_expanded_again() {
        let replacements = [Pair {
            name: "x".into(),
            value: "[page]".into(),
        }];
        let expansion = expand("[x]", &context(&replacements));
        assert_eq!(expansion.html, "[page]");
        assert!(!expansion.html.contains("span"));
    }

    /// Everything substituted is escaped, and the spans are the only markup that
    /// survives. A title with an ampersand in it is ordinary.
    #[test]
    fn substituted_values_are_escaped() {
        let context = Context {
            title: "Tom & Jerry <b>".into(),
            ..context(&[])
        };
        let expansion = expand("[title]", &context);
        assert_eq!(expansion.html, "Tom &amp; Jerry &lt;b&gt;");
    }

    #[test]
    fn a_replacement_value_is_escaped_too() {
        let replacements = [Pair {
            name: "x".into(),
            value: "<script>alert(1)</script>".into(),
        }];
        let expansion = expand("[x]", &context(&replacements));
        assert!(!expansion.html.contains("<script>"), "{}", expansion.html);
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
        assert_eq!(html("v1.2 — [frompage] of many"), "v1.2 — 1 of many");
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
        assert_eq!(expand("[title]", &context).html, "");
    }
}
