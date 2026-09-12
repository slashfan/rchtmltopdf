//! PDF post-processing: merging and metadata now, outlines and overlays later.
//!
//! **No lopdf type crosses this boundary (D12).** Everything here takes bytes
//! and gives bytes back, so the library underneath can be replaced by editing
//! one crate. The conformance harness keeps the same promise on its own side of
//! the fence, in `inspect` (D25).
//!
//! # What a merge keeps, and what it drops
//!
//! Every page, in command line order, with whatever it inherited from the page
//! tree above it copied down onto it, because the tree it sat in is not coming
//! along. The first document's Info dictionary, because wkhtmltopdf's output
//! carried the first document's title and nothing else has a better claim.
//! Everything else that hung off a catalog — outlines, named destinations,
//! link destinations that named a page — is left behind, and rebuilt by the
//! milestone that owns it (#40, #41).
//!
//! **Fonts are not deduplicated (D34).** Chromium subsets a font per document,
//! so two documents in the same face carry two different subsets under two
//! different names, and folding them into one is a font-program operation, not
//! a PDF one. A ten document PDF embeds the face ten times, on purpose.
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

use lopdf::{Document, Object, ObjectId, dictionary};
use rchtmltopdf_core::Clock;
use std::collections::BTreeMap;
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

    // The dictionary that is already there, edited in place. Adding a second one
    // and pointing the trailer at it leaves the first in the file: readers
    // follow the trailer and see the right thing, but the bytes still carry
    // Chromium's producer, and **the document's own title goes with it**. The
    // print call derives that title from the `<title>` element and it is the
    // only one most documents will ever have.
    let id = existing_info(&mut document);
    let info = document
        .get_object_mut(id)
        .and_then(lopdf::Object::as_dict_mut)
        .map_err(fail)?;

    // Only when asked. An absent `--title` means "leave the document's own
    // alone", not "clear it".
    if let Some(title) = &metadata.title {
        info.set("Title", text_string(title));
    }
    // These four are ours whatever was there before: Chromium produced the pages
    // and this produced the file, and a dictionary naming two programs is worse
    // than either.
    info.set("Producer", text_string(&metadata.producer));
    info.set("Creator", text_string(&metadata.creator));
    let date = pdf_date(&metadata.created);
    info.set("CreationDate", text_string(&date));
    info.set("ModDate", text_string(&date));

    let mut out = Vec::with_capacity(pdf.len());
    document.save_to(&mut out).map_err(fail)?;
    Ok(out)
}

/// Combine several printed documents into one, in the order given.
///
/// One document comes back untouched, bytes for bytes: there is nothing to
/// combine, and rewriting a file that does not need it would only give a
/// single-document conversion a way to differ from what Chromium printed.
///
/// # How the objects are moved
///
/// Every object in a source document gets a fresh number in the target, and
/// every reference inside it is rewritten to match, so nothing from one
/// document can collide with anything from another. A reference to an object
/// the source never had is rewritten to `null` rather than left pointing at a
/// number that now belongs to something else: that is what the specification
/// says a dangling reference means, and it is the one case where keeping the
/// number would silently corrupt the result.
///
/// The old catalogs and page trees are not copied across. A new tree holds
/// every page, the trailer points at a new catalog, and whatever is no longer
/// reachable from the trailer is pruned before the numbers are compacted.
pub fn merge(parts: &[&[u8]]) -> Result<Vec<u8>, Error> {
    match parts {
        [] => {
            return Err(Error {
                reason: "there is no document to merge".into(),
            });
        }
        [only] => return Ok(only.to_vec()),
        _ => {}
    }

    let mut merged = Document::with_version("1.4");
    let mut pages = Vec::new();
    let mut info = None;

    for (index, part) in parts.iter().enumerate() {
        let source = Document::load_mem(part)
            .map_err(|error| fail(format!("document {}: {error}", index + 1)))?;
        // The highest version any part asks for. A feature a later part used
        // is still used after the merge.
        if source.version > merged.version {
            merged.version = source.version.clone();
        }
        let absorbed = absorb(&mut merged, source)
            .map_err(|error| fail(format!("document {}: {}", index + 1, error.reason)))?;
        pages.extend(absorbed.pages);
        if info.is_none() {
            info = absorbed.info;
        }
    }

    let pages_id = merged.new_object_id();
    for page in &pages {
        let dictionary = merged.get_dictionary_mut(*page).map_err(fail)?;
        dictionary.set("Parent", Object::Reference(pages_id));
    }
    let count = i64::try_from(pages.len()).map_err(fail)?;
    merged.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => pages.into_iter().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => count,
        }),
    );
    let catalog = merged.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    merged.trailer.set("Root", Object::Reference(catalog));
    if let Some(info) = info {
        merged.trailer.set("Info", Object::Reference(info));
    }

    // The old catalogs, page trees, outlines and object streams are no longer
    // reachable from the trailer, and this is what removes them.
    merged.prune_objects();
    merged.renumber_objects();

    let mut out = Vec::with_capacity(parts.iter().map(|part| part.len()).sum());
    merged.save_to(&mut out).map_err(fail)?;
    Ok(out)
}

/// What one document contributed to a merge, numbered as the target knows it.
struct Absorbed {
    /// Its pages, in reading order.
    pages: Vec<ObjectId>,
    /// Its Info dictionary, when it had one that resolves.
    info: Option<ObjectId>,
}

/// Move every object of `source` into `into` under fresh numbers.
fn absorb(into: &mut Document, mut source: Document) -> Result<Absorbed, Error> {
    // Read while the source is still whole: the page order comes from walking
    // its tree, and the inherited attributes from walking `Parent`, and neither
    // exists once the objects have been moved.
    let page_ids: Vec<ObjectId> = source.page_iter().collect();
    if page_ids.is_empty() {
        return Err(fail("it has no pages"));
    }
    for page in &page_ids {
        inherit(&mut source, *page)?;
    }
    let info = source
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|entry| entry.as_reference().ok())
        .filter(|id| source.get_dictionary(*id).is_ok());

    let objects = std::mem::take(&mut source.objects);
    let map: BTreeMap<ObjectId, ObjectId> = objects
        .keys()
        .map(|old| (*old, into.new_object_id()))
        .collect();
    for (old, mut object) in objects {
        relabel(&mut object, &map);
        into.objects.insert(map[&old], object);
    }

    Ok(Absorbed {
        pages: page_ids.iter().map(|id| map[id]).collect(),
        info: info.map(|id| map[&id]),
    })
}

/// The page attributes the specification lets a page leave to its ancestors.
const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// Copy onto a page whatever it was inheriting from the tree above it.
///
/// Chromium writes all four on the page itself today, and a merge that relied
/// on that would work right up to the first PDF from something else. The
/// nearest ancestor wins, which is the specification's rule too.
fn inherit(document: &mut Document, page: ObjectId) -> Result<(), Error> {
    let mut found = Vec::new();
    {
        let dictionary = document.get_dictionary(page).map_err(fail)?;
        let mut missing: Vec<&[u8]> = INHERITABLE
            .iter()
            .copied()
            .filter(|key| !dictionary.has(key))
            .collect();
        let mut node = dictionary
            .get(b"Parent")
            .and_then(Object::as_reference)
            .ok();
        // A tree that loops is malformed; sixty-four levels is more than any
        // real one has, and enough to stop rather than spin.
        let mut depth = 0;
        while let Some(id) = node.filter(|_| !missing.is_empty() && depth < 64) {
            let parent = document.get_dictionary(id).map_err(fail)?;
            missing.retain(|key| match parent.get(key) {
                Ok(value) => {
                    found.push((key.to_vec(), value.clone()));
                    false
                }
                Err(_) => true,
            });
            node = parent.get(b"Parent").and_then(Object::as_reference).ok();
            depth += 1;
        }
    }
    let dictionary = document.get_dictionary_mut(page).map_err(fail)?;
    for (key, value) in found {
        dictionary.set(key, value);
    }
    Ok(())
}

/// Rewrite every reference inside an object through the map.
fn relabel(object: &mut Object, map: &BTreeMap<ObjectId, ObjectId>) {
    match object {
        Object::Reference(id) => match map.get(id) {
            Some(new) => *id = *new,
            // Dangling in the source, so null by the specification's own rule,
            // and never a number that now means something else.
            None => *object = Object::Null,
        },
        Object::Array(items) => items.iter_mut().for_each(|item| relabel(item, map)),
        Object::Dictionary(dictionary) => dictionary
            .iter_mut()
            .for_each(|(_, value)| relabel(value, map)),
        Object::Stream(stream) => stream
            .dict
            .iter_mut()
            .for_each(|(_, value)| relabel(value, map)),
        _ => {}
    }
}

/// The document's Info dictionary, created empty if it has none.
fn existing_info(document: &mut Document) -> lopdf::ObjectId {
    if let Ok(Object::Reference(id)) = document.trailer.get(b"Info") {
        let id = *id;
        // A dangling reference is not a dictionary to edit. Rare, and cheaper to
        // check than to debug.
        if document.get_dictionary(id).is_ok() {
            return id;
        }
    }

    let id = document.add_object(Object::Dictionary(lopdf::Dictionary::new()));
    document.trailer.set("Info", Object::Reference(id));
    id
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
        pages(&["ONE", "TWO"], None)
    }

    /// A document with one page per label, the label written into the page's
    /// content stream so the order can be read back after a merge.
    ///
    /// With `resources`, the dictionary sits on the `Pages` node rather than on
    /// each page: the inherited arrangement the specification allows and
    /// Chromium never produces.
    fn pages(labels: &[&str], resources: Option<lopdf::Dictionary>) -> Vec<u8> {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let page_ids: Vec<Object> = labels
            .iter()
            .map(|label| {
                let contents = document.add_object(lopdf::Stream::new(
                    dictionary! {},
                    format!("BT ({label}) Tj ET").into_bytes(),
                ));
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
        let mut tree = dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids,
            "Count" => count,
        };
        if let Some(resources) = resources {
            tree.set("Resources", Object::Dictionary(resources));
        }
        document.objects.insert(pages_id, Object::Dictionary(tree));
        let catalog = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog);

        let mut out = Vec::new();
        document.save_to(&mut out).expect("a hand-built PDF saves");
        out
    }

    /// The same document with an Info dictionary carrying a title.
    fn titled(labels: &[&str], title: &str) -> Vec<u8> {
        let mut document = Document::load_mem(&pages(labels, None)).expect("should parse");
        let id = document.add_object(Object::Dictionary(dictionary! {
            "Title" => Object::string_literal(title),
        }));
        document.trailer.set("Info", Object::Reference(id));
        let mut out = Vec::new();
        document.save_to(&mut out).expect("should save");
        out
    }

    /// The labels of every page, in reading order.
    fn labels(pdf: &[u8]) -> Vec<String> {
        let document = Document::load_mem(pdf).expect("the result should parse");
        document
            .page_iter()
            .map(|page| {
                let content = String::from_utf8(document.get_page_content(page))
                    .expect("hand-written content is text");
                content
                    .trim()
                    .trim_start_matches("BT (")
                    .trim_end_matches(") Tj ET")
                    .to_string()
            })
            .collect()
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
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes(*pair))
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

    /// Left alone rather than emptied. The print call has already taken the
    /// document's own `<title>`, and for most documents that is the only title
    /// there will ever be.
    #[test]
    fn no_title_leaves_the_one_that_was_there() {
        let mut document = Document::load_mem(&two_pages()).expect("should parse");
        let id = document.add_object(Object::Dictionary(dictionary! {
            "Title" => Object::string_literal("From the document"),
            "Producer" => Object::string_literal("Skia/PDF m153"),
        }));
        document.trailer.set("Info", Object::Reference(id));
        let mut titled = Vec::new();
        document.save_to(&mut titled).expect("should save");

        let out = set_metadata(
            &titled,
            &Metadata {
                producer: "rchtmltopdf 0.0.1".into(),
                ..Metadata::default()
            },
        )
        .expect("should rewrite");

        assert_eq!(info(&out, "Title").as_deref(), Some("From the document"));
        // And the producer is ours, because that one we do own.
        assert_eq!(info(&out, "Producer").as_deref(), Some("rchtmltopdf 0.0.1"));
    }

    /// The old dictionary is edited rather than orphaned, so nothing in the file
    /// still claims the browser made it.
    #[test]
    fn the_previous_producer_does_not_survive_in_the_bytes() {
        let mut document = Document::load_mem(&two_pages()).expect("should parse");
        let id = document.add_object(Object::Dictionary(dictionary! {
            "Producer" => Object::string_literal("Skia/PDF m153"),
        }));
        document.trailer.set("Info", Object::Reference(id));
        let mut before = Vec::new();
        document.save_to(&mut before).expect("should save");

        let out = set_metadata(
            &before,
            &Metadata {
                producer: "rchtmltopdf 0.0.1".into(),
                ..Metadata::default()
            },
        )
        .expect("should rewrite");

        assert!(
            !String::from_utf8_lossy(&out).contains("Skia/PDF"),
            "the old dictionary was left in the file"
        );
    }

    /// A document with no Info dictionary at all still gets one.
    #[test]
    fn a_document_without_metadata_gains_some() {
        let out = set_metadata(
            &two_pages(),
            &Metadata {
                producer: "rchtmltopdf 0.0.1".into(),
                ..Metadata::default()
            },
        )
        .expect("should rewrite");
        assert_eq!(info(&out, "Producer").as_deref(), Some("rchtmltopdf 0.0.1"));
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

    // --- merge ------------------------------------------------------------------

    #[test]
    fn pages_come_out_in_command_line_order() {
        let a = pages(&["A1", "A2"], None);
        let b = pages(&["B1"], None);
        let c = pages(&["C1", "C2", "C3"], None);

        let out = merge(&[&a, &b, &c]).expect("should merge");

        assert_eq!(labels(&out), ["A1", "A2", "B1", "C1", "C2", "C3"]);
        let document = Document::load_mem(&out).expect("should parse");
        assert_eq!(document.get_pages().len(), 6);
    }

    /// **The check that the relabelling was complete.** Every reference in the
    /// result must resolve. A number left behind from a source document either
    /// dangles or, worse, lands on another document's object and the file opens
    /// with the wrong font on the wrong page.
    #[test]
    fn nothing_in_the_result_references_an_object_that_is_not_there() {
        let out = merge(&[&pages(&["A"], None), &pages(&["B", "C"], None)]).expect("should merge");

        let mut document = Document::load_mem(&out).expect("should parse");
        let referenced = document.traverse_objects(|_| {});
        for id in referenced {
            assert!(document.has_object(id), "{id:?} is referenced and missing");
        }
    }

    /// One tree, one catalog. The sources' own are unreachable and pruned,
    /// so a reader cannot pick up a stale `Count` or a second `Root`.
    #[test]
    fn the_sources_own_catalogs_and_trees_are_gone() {
        let out = merge(&[&pages(&["A"], None), &pages(&["B"], None)]).expect("should merge");

        let document = Document::load_mem(&out).expect("should parse");
        let of_type = |name: &[u8]| {
            document
                .objects
                .values()
                .filter(|object| object.as_dict().is_ok_and(|d| d.has_type(name)))
                .count()
        };
        assert_eq!(of_type(b"Catalog"), 1);
        assert_eq!(of_type(b"Pages"), 1);

        let tree = document
            .catalog()
            .and_then(|catalog| catalog.get(b"Pages"))
            .and_then(Object::as_reference)
            .and_then(|id| document.get_dictionary(id))
            .expect("the catalog should name the tree");
        assert_eq!(tree.get(b"Count").and_then(Object::as_i64).ok(), Some(2));
        // And every page points back at that tree, not at the one it came from.
        for page in document.page_iter() {
            let parent = document
                .get_dictionary(page)
                .and_then(|d| d.get(b"Parent"))
                .and_then(Object::as_reference)
                .expect("a page should have a parent");
            assert!(
                document
                    .get_dictionary(parent)
                    .is_ok_and(|d| d.has_type(b"Pages") && d.get(b"Count").is_ok()),
                "page {page:?} still points at an old parent"
            );
        }
    }

    /// The tree a page sat in is not coming along, so what it inherited from
    /// that tree has to come down onto the page first.
    #[test]
    fn what_a_page_inherited_from_its_tree_is_copied_onto_it() {
        let inherited = dictionary! { "ProcSet" => vec![Object::Name(b"PDF".to_vec())] };
        let out =
            merge(&[&pages(&["A"], Some(inherited)), &pages(&["B"], None)]).expect("should merge");

        let document = Document::load_mem(&out).expect("should parse");
        let first = document.page_iter().next().expect("a page");
        let resources = document
            .get_dictionary(first)
            .and_then(|page| page.get(b"Resources"))
            .and_then(Object::as_dict)
            .expect("the page should carry the resources it inherited");
        assert!(resources.has(b"ProcSet"));
    }

    /// wkhtmltopdf's output carried the first document's title.
    #[test]
    fn the_first_documents_info_dictionary_is_the_one_kept() {
        let out =
            merge(&[&titled(&["A"], "First"), &titled(&["B"], "Second")]).expect("should merge");
        assert_eq!(info(&out, "Title").as_deref(), Some("First"));

        // And the metadata pass still finds it to edit, rather than adding one.
        let rewritten = set_metadata(
            &out,
            &Metadata {
                producer: "rchtmltopdf 0.0.1".into(),
                ..Metadata::default()
            },
        )
        .expect("should rewrite");
        assert_eq!(info(&rewritten, "Title").as_deref(), Some("First"));
        assert_eq!(
            info(&rewritten, "Producer").as_deref(),
            Some("rchtmltopdf 0.0.1")
        );
    }

    /// A source that references an object it does not have gets `null`, never a
    /// number that now belongs to another document.
    #[test]
    fn a_dangling_reference_becomes_null_rather_than_somebody_elses_object() {
        let mut document = Document::load_mem(&pages(&["A"], None)).expect("should parse");
        let first = document.page_iter().next().expect("a page");
        document
            .get_dictionary_mut(first)
            .expect("a page")
            .set("Dangling", Object::Reference((900, 0)));
        let mut broken = Vec::new();
        document.save_to(&mut broken).expect("should save");

        let out = merge(&[&broken, &pages(&["B"], None)]).expect("should merge");

        let document = Document::load_mem(&out).expect("should parse");
        let first = document.page_iter().next().expect("a page");
        let page = document.get_dictionary(first).expect("a page");
        assert!(
            !matches!(page.get(b"Dangling"), Ok(Object::Reference(_))),
            "a reference into nothing was kept as a reference"
        );
    }

    /// Bytes for bytes: nothing to combine, and nothing to risk.
    #[test]
    fn one_document_is_returned_untouched() {
        let only = two_pages();
        assert_eq!(merge(&[&only]).expect("should pass through"), only);
    }

    #[test]
    fn nothing_to_merge_is_an_error_not_an_empty_file() {
        assert!(merge(&[]).is_err());
    }

    /// The failure names which document, because a command line can carry ten.
    #[test]
    fn a_part_that_is_not_a_pdf_is_named_by_position() {
        let error = merge(&[&two_pages(), b"not a PDF"]).expect_err("should refuse");
        assert!(error.to_string().contains("document 2"), "{error}");
    }
}
