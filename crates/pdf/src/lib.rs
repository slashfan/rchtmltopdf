//! PDF post-processing: metadata now, merge and outlines later.
//!
//! **No lopdf type crosses this boundary (D12).** Everything here takes bytes
//! and gives bytes back, so the library underneath can be replaced by editing
//! one crate. The conformance harness keeps the same promise on its own side of
//! the fence, in `inspect` (D25).
//!
//! # Why metadata is the first thing built here
//!
//! It looks like the smallest possible job and it is not: it establishes the
//! read, modify, write path that the V2 header overlay and the multi-document
//! merge both depend on. Object renumbering and cross-reference regeneration are
//! easier to get right on one document than to discover part-way through a merge
//! (#29).
//!
//! # The trap in a PDF string
//!
//! A PDF text string is Latin-1 unless it starts with a UTF-16 byte order mark.
//! A title of `Facture n°42` written as raw bytes is either mojibake or a broken
//! file depending on the reader, so anything outside ASCII is encoded as UTF-16
//! big endian with the mark in front. Invoices in French are the ordinary case
//! here, not the exotic one.

use lopdf::{Document, Object};
use rchtmltopdf_core::Clock;
use std::fmt;

/// Anything that went wrong reading or writing a document.
///
/// Carries a sentence rather than the library's own error type, because the
/// library's own error type is exactly what must not escape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub reason: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the PDF could not be rewritten: {}", self.reason)
    }
}

impl std::error::Error for Error {}

/// What to write into the document's Info dictionary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    /// `--title`. Left alone when absent: the print call has already taken the
    /// document's own `<title>`, and replacing that with nothing would be worse
    /// than saying nothing.
    pub title: Option<String>,
    /// What made the document. Names this program and its version.
    pub producer: String,
    /// What asked for it. The same, for a conversion with one step.
    pub creator: String,
    pub created: Clock,
}

/// Read a document, set its metadata, and write it back.
pub fn set_metadata(pdf: &[u8], metadata: &Metadata) -> Result<Vec<u8>, Error> {
    let mut document = Document::load_mem(pdf).map_err(fail)?;

    let mut info = lopdf::Dictionary::new();
    if let Some(title) = &metadata.title {
        info.set("Title", text_string(title));
    }
    info.set("Producer", text_string(&metadata.producer));
    info.set("Creator", text_string(&metadata.creator));
    let date = pdf_date(&metadata.created);
    info.set("CreationDate", text_string(&date));
    info.set("ModDate", text_string(&date));

    // Replace whatever Chromium wrote rather than merging into it: its producer
    // is not the producer any more, and a half-updated dictionary naming two
    // programs is worse than either.
    let id = document.add_object(Object::Dictionary(info));
    document.trailer.set("Info", Object::Reference(id));

    let mut out = Vec::with_capacity(pdf.len());
    document.save_to(&mut out).map_err(fail)?;
    Ok(out)
}

fn fail(error: impl fmt::Display) -> Error {
    Error {
        reason: error.to_string(),
    }
}

/// A PDF text string: Latin-1 when it can be, UTF-16 when it cannot.
///
/// The byte order mark is what tells a reader which it is holding, and without
/// it `Facture n°42` is a different string in every viewer.
fn text_string(value: &str) -> Object {
    if value.is_ascii() {
        return Object::string_literal(value);
    }

    let mut bytes = vec![0xFE, 0xFF];
    for unit in value.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Object::String(bytes, lopdf::StringFormat::Hexadecimal)
}

/// `D:YYYYMMDDHHmmSSOHH'mm'`, which is the only date syntax a PDF has.
///
/// The trailing apostrophe after the minutes is not a typo and not optional:
/// readers that validate the syntax reject the date without it, and readers that
/// do not will show it to somebody.
fn pdf_date(clock: &Clock) -> String {
    let offset = clock.utc_offset_seconds;
    let sign = match offset {
        0 => 'Z',
        _ if offset < 0 => '-',
        _ => '+',
    };
    let (hours, minutes) = (offset.abs() / 3600, (offset.abs() % 3600) / 60);

    format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}{sign}{hours:02}'{minutes:02}'",
        clock.year, clock.month, clock.day, clock.hour, clock.minute, clock.second
    )
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

    /// A two page document built by hand, so the read-modify-write path can be
    /// exercised without a browser anywhere near it.
    fn two_pages() -> Vec<u8> {
        use lopdf::dictionary;

        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let page_ids: Vec<Object> = (0..2)
            .map(|_| {
                let contents =
                    document.add_object(lopdf::Stream::new(dictionary! {}, b"BT ET".to_vec()));
                let page = document.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "Contents" => contents,
                    "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
                });
                Object::Reference(page)
            })
            .collect();

        let count = page_ids.len() as i64;
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids,
                "Count" => count,
            }),
        );
        let catalog = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog);

        let mut out = Vec::new();
        document.save_to(&mut out).expect("a hand-built PDF saves");
        out
    }

    /// Reads the Info dictionary back the way a viewer would.
    fn info(pdf: &[u8], key: &str) -> Option<String> {
        let document = Document::load_mem(pdf).expect("should parse");
        let info = document.trailer.get(b"Info").ok()?;
        let dictionary = match info {
            Object::Reference(id) => document.get_dictionary(*id).ok()?,
            Object::Dictionary(dictionary) => dictionary,
            _ => return None,
        };
        let value = dictionary.get(key.as_bytes()).ok()?.as_str().ok()?;
        Some(decode(value))
    }

    /// Latin-1, or UTF-16 when the mark says so.
    fn decode(bytes: &[u8]) -> String {
        if bytes.starts_with(&[0xFE, 0xFF]) {
            let units: Vec<u16> = bytes[2..]
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            return String::from_utf16_lossy(&units);
        }
        bytes.iter().map(|byte| *byte as char).collect()
    }

    #[test]
    fn a_title_is_written_and_reads_back() {
        let metadata = Metadata {
            title: Some("Invoice 42".into()),
            producer: "rchtmltopdf 0.0.1".into(),
            creator: "rchtmltopdf 0.0.1".into(),
            created: clock(),
        };
        let out = set_metadata(&two_pages(), &metadata).expect("should rewrite");

        assert_eq!(info(&out, "Title").as_deref(), Some("Invoice 42"));
        assert_eq!(info(&out, "Producer").as_deref(), Some("rchtmltopdf 0.0.1"));
        assert_eq!(
            info(&out, "CreationDate").as_deref(),
            Some("D:20260912140509+02'00'")
        );
    }

    /// **The point of doing this first.** Object renumbering and cross-reference
    /// regeneration are what the V2 merge depends on, and they are easier to get
    /// right here than to discover part-way through one.
    #[test]
    fn a_round_trip_keeps_the_pages() {
        let before = two_pages();
        let after = set_metadata(&before, &Metadata::default()).expect("should rewrite");

        let document = Document::load_mem(&after).expect("the result should parse");
        assert_eq!(document.get_pages().len(), 2);
        assert!(after.starts_with(b"%PDF-"));
    }

    /// Invoices in French are the ordinary case here.
    #[test]
    fn a_title_outside_ascii_survives() {
        let metadata = Metadata {
            title: Some("Facture n°42 — Février".into()),
            ..Metadata::default()
        };
        let out = set_metadata(&two_pages(), &metadata).expect("should rewrite");
        assert_eq!(
            info(&out, "Title").as_deref(),
            Some("Facture n°42 — Février")
        );
    }

    /// Left alone rather than emptied: the print call has already taken the
    /// document's own `<title>`.
    #[test]
    fn no_title_writes_no_title() {
        let out = set_metadata(&two_pages(), &Metadata::default()).expect("should rewrite");
        assert_eq!(info(&out, "Title"), None);
    }

    #[test]
    fn a_date_at_utc_says_so_rather_than_plus_nothing() {
        let utc = Clock {
            utc_offset_seconds: 0,
            ..clock()
        };
        assert_eq!(pdf_date(&utc), "D:20260912140509Z00'00'");

        let behind = Clock {
            utc_offset_seconds: -(5 * 3600 + 30 * 60),
            ..clock()
        };
        assert_eq!(pdf_date(&behind), "D:20260912140509-05'30'");
    }

    /// Something that is not a PDF is reported rather than panicked on.
    #[test]
    fn rubbish_in_is_an_error_not_a_panic() {
        let error =
            set_metadata(b"this is not a PDF", &Metadata::default()).expect_err("should refuse");
        assert!(!error.to_string().is_empty());
    }
}
