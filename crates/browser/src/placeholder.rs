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
    /// What `[doctitle]` prints: `--title`, or the first document's own title
    /// when it was not given — the title the finished file carries either way.
    /// The plan fills it from the command line; the fallback is not known
    /// until the documents have been printed, so the conversion fills that in
    /// (#110).
    pub title: String,
    /// What `[title]` prints: **this** document's own `<title>`, which is not
    /// the file's as soon as a conversion has two documents. Chromium writes
    /// it into each printed part, so like a band document's height (D39) it is
    /// a measurement the plan cannot make: empty here until the part has been
    /// printed.
    pub document_title: String,
    /// `--replace`, which defines placeholders of the user's own. A pair named
    /// after a built-in never shadows it, in a text band or in a query string
    /// alike (#111).
    pub replacements: Vec<Pair>,
    /// Read once per conversion and carried, so a document whose header and
    /// footer both say `[time]` cannot print two different times.
    pub clock: Clock,
}

/// What one page's numbers are, in wkhtmltopdf's two frames (#39).
///
/// `page` and `topage` count across the whole output; `sitepage` and
/// `sitepages` within the object the page came from. `frompage` is in neither
/// frame: it is the first page of the output, the same number on every page,
/// which is what wkhtmltopdf fills it with (D51). A cover's pages are in both
/// counts (D45); what a cover has not got is a band to print them in. The
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
    if let Some(value) = built_in(name, context, numbers) {
        return Some(value);
    }

    // `--replace` defines placeholders of the user's own, and only those: a
    // pair named after a built-in is shadowed by it rather than the other way
    // round (#111). Its value is inserted literally and never rescanned, so a
    // replacement containing `[page]` prints those six characters.
    context
        .replacements
        .iter()
        .find(|pair| pair.name == name)
        .map(|pair| pair.value.clone())
}

/// One built-in placeholder's value, `--replace` out of the picture.
fn built_in(name: &str, context: &Context, numbers: &Numbers) -> Option<String> {
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

        // --- from the command line and the documents --------------------------
        "webpage" => context.webpage.clone(),
        // Two different questions, and wkhtmltopdf answers them differently
        // (`pdfconverter.cc` lines 590-591): `[title]` is the page object's own
        // title, `[doctitle]` the title of the document being produced.
        "title" => context.document_title.clone(),
        "doctitle" => context.title.clone(),

        // --- rendered here, not by the browser ---------------------------------
        "date" => context.clock.date(),
        "isodate" => context.clock.iso(),
        "time" => context.clock.time(),

        _ => return None,
    })
}

/// The same values as a query string, for a band that is a document.
///
/// wkhtmltopdf hands `--header-html` its placeholders "in get fashion": the
/// document is loaded once per page with `?page=3&topage=9&...` appended, and
/// its own script reads `document.location.search`. The names are the
/// placeholders' without the brackets, `--replace` pairs are added first and
/// a built-in name wins over a replacement of the same name, which is the
/// order wkhtmltopdf fills them in and what a text band does too. Every value
/// is percent-encoded, so the
/// documented `decodeURI` reads it back as itself.
pub fn query(context: &Context, numbers: &Numbers) -> String {
    let mut pairs: Vec<(String, String)> = context
        .replacements
        .iter()
        .map(|pair| (pair.name.clone(), pair.value.clone()))
        .collect();
    for name in BUILT_IN {
        let value = built_in(name, context, numbers).unwrap_or_default();
        pairs.retain(|(existing, _)| existing != name);
        pairs.push((name.to_string(), value));
    }
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", percent_encode(name), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Every built-in placeholder, in the order the query string names them.
const BUILT_IN: &[&str] = &[
    "page",
    "topage",
    "frompage",
    "sitepage",
    "sitepages",
    "section",
    "subsection",
    "subsubsection",
    "webpage",
    "title",
    "doctitle",
    "date",
    "isodate",
    "time",
];

/// A band document's URL with the query string attached.
///
/// Appended to a query the URL already has, and kept ahead of a fragment, so
/// `header.html?theme=dark#top` still says both.
pub fn with_query(url: &str, query: &str) -> String {
    let (base, fragment) = match url.find('#') {
        Some(at) => (&url[..at], &url[at..]),
        None => (url, ""),
    };
    let joiner = if base.contains('?') { '&' } else { '?' };
    format!("{base}{joiner}{query}{fragment}")
}

/// Percent-encode everything outside the unreserved set.
///
/// Bytes, not characters, so a title in any script arrives as UTF-8 the way
/// `decodeURI` expects it. `&`, `=` and `#` are among what is encoded, so a
/// value cannot end its own pair or start a fragment.
fn percent_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
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
            document_title: "The document's own title".into(),
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
        assert_eq!(html("[title]"), "The document&#39;s own title");
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

    /// `--replace` defines placeholders of the user's own, and a built-in name
    /// wins over a pair that shadows it (#111): wkhtmltopdf fills its hash with
    /// the replacements and assigns the built-ins after, overwriting them.
    #[test]
    fn a_built_in_wins_over_a_replacement_of_the_same_name() {
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
        assert_eq!(expansion, "Acme Ltd 4");
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
            document_title: "Tom & Jerry <b>".into(),
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

    // --- the query string, for a band that is a document ----------------------

    /// The names are the placeholders' without the brackets, and the numbers
    /// are the page's: this is what the documented `subst()` reads.
    #[test]
    fn the_query_names_every_placeholder_with_the_pages_numbers() {
        let query = query(&context(&[]), &numbers());
        for expected in [
            "page=4",
            "topage=5",
            "frompage=4",
            "sitepage=1",
            "sitepages=2",
            "section=Chapter%20Two",
            "subsection=Section%202.1",
            "subsubsection=",
            "webpage=https%3A%2F%2Fexample.com%2Finvoice",
            "title=The%20document%27s%20own%20title",
            "doctitle=Invoice%2042",
            "date=",
            "isodate=2026-09-12",
            "time=",
        ] {
            assert!(
                query.split('&').any(|pair| pair.starts_with(expected)),
                "{expected:?} missing from {query:?}"
            );
        }
    }

    /// A value is somebody's title, and `&`, `=` and a space in it must not
    /// end the pair or start another. `decodeURI` reads the encoding back.
    #[test]
    fn values_are_percent_encoded() {
        let mut with_title = context(&[]);
        with_title.title = "Tom & Jerry = friends".into();
        let query = query(&with_title, &numbers());
        assert!(
            query.contains("title=Tom%20%26%20Jerry%20%3D%20friends"),
            "{query}"
        );
        assert_eq!(
            query
                .split('&')
                .filter(|pair| pair.starts_with("title="))
                .count(),
            1
        );
    }

    /// `--replace` pairs are added, and a built-in name wins over a
    /// replacement of the same name: that is the order wkhtmltopdf fills its
    /// hash in, and since #111 what a text band does as well.
    #[test]
    fn replacements_are_added_and_built_ins_win_over_them() {
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
        let query = query(&context(&replacements), &numbers());
        assert!(query.contains("client=Acme%20Ltd"), "{query}");
        assert!(query.contains("page=4"), "{query}");
        assert!(!query.contains("business"), "{query}");
    }

    #[test]
    fn the_query_joins_what_the_url_already_says() {
        assert_eq!(
            with_query("file:///h.html", "page=1"),
            "file:///h.html?page=1"
        );
        assert_eq!(
            with_query("http://x/h?theme=dark", "page=1"),
            "http://x/h?theme=dark&page=1"
        );
        assert_eq!(
            with_query("file:///h.html#top", "page=1"),
            "file:///h.html?page=1#top"
        );
    }

    /// A title nobody gave and no document carried is not there, and an empty
    /// expansion is better than the word "title" printed on every page.
    #[test]
    fn an_unset_title_expands_to_nothing() {
        let context = Context {
            title: String::new(),
            document_title: String::new(),
            ..context(&[])
        };
        assert_eq!(expand("[title]", &context, &numbers()), "");
        assert_eq!(expand("[doctitle]", &context, &numbers()), "");
    }

    /// **`[title]` is the document's own and `[doctitle]` the file's** (#110).
    /// They part company as soon as a conversion has more than one document,
    /// or a `--title` that is not any document's.
    #[test]
    fn title_is_the_documents_own_and_doctitle_the_files() {
        let context = Context {
            title: "What the file is called".into(),
            document_title: "What this document is called".into(),
            ..context(&[])
        };
        assert_eq!(
            expand("[title] / [doctitle]", &context, &numbers()),
            "What this document is called / What the file is called"
        );
    }
}
