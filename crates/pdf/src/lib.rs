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
//! The outlines, joined end to end under one root, so a reader's sidebar lists
//! every document's headings in order (#40). Everything else that hung off a
//! catalog — named destinations, link destinations that named a page — is left
//! behind, and rebuilt by the milestone that owns it (#41).
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
    let mut outlines: Vec<PartOutline> = Vec::new();

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
        outlines.extend(absorbed.outline);
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
    let mut catalog = dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    };
    if let Some(root) = join_outlines(&mut merged, &outlines)? {
        catalog.set("Outlines", Object::Reference(root));
    }
    let catalog = merged.add_object(catalog);
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
    /// Its outline, when it had one with something in it.
    outline: Option<PartOutline>,
}

/// The top level of one document's outline: the ends of the chain of items
/// that hung directly under its root.
struct PartOutline {
    first: ObjectId,
    last: ObjectId,
}

/// The ends of the top-level chain under a document's outline root.
fn part_outline(document: &Document) -> Option<PartOutline> {
    let root = outline_root(document)?;
    let root = document.get_dictionary(root).ok()?;
    let first = root.get(b"First").and_then(Object::as_reference).ok()?;
    let last = root.get(b"Last").and_then(Object::as_reference).ok()?;
    Some(PartOutline { first, last })
}

/// The catalog's outline root, when it names one that resolves.
fn outline_root(document: &Document) -> Option<ObjectId> {
    let id = document
        .catalog()
        .ok()?
        .get(b"Outlines")
        .and_then(Object::as_reference)
        .ok()?;
    document.get_dictionary(id).is_ok().then_some(id)
}

/// One root over every document's top-level items, chained in order.
///
/// Each document's chain is already consistent inside itself: `Prev` and
/// `Next` between siblings, `Parent` pointing at the old root. What is left is
/// to point every top-level `Parent` at the new root, tie the last item of one
/// document to the first of the next, and count.
fn join_outlines(
    document: &mut Document,
    parts: &[PartOutline],
) -> Result<Option<ObjectId>, Error> {
    let Some(first) = parts.first() else {
        return Ok(None);
    };
    let root = document.new_object_id();
    for (index, part) in parts.iter().enumerate() {
        for item in siblings(document, Some(part.first)) {
            let dictionary = document.get_dictionary_mut(item).map_err(fail)?;
            dictionary.set("Parent", Object::Reference(root));
        }
        if let Some(previous) = index.checked_sub(1).map(|i| &parts[i]) {
            document
                .get_dictionary_mut(previous.last)
                .map_err(fail)?
                .set("Next", Object::Reference(part.first));
            document
                .get_dictionary_mut(part.first)
                .map_err(fail)?
                .set("Prev", Object::Reference(previous.last));
        }
    }
    let last = parts.last().expect("checked above").last;
    document.objects.insert(
        root,
        Object::Dictionary(dictionary! {
            "Type" => "Outlines",
            "First" => first.first,
            "Last" => last,
        }),
    );
    recount(document, root)?;
    Ok(Some(root))
}

/// A chain of siblings, `Next` after `Next`, starting at `first`.
///
/// Bounded, because a malformed chain that loops would otherwise never end,
/// and no real outline has this many entries at one level.
fn siblings(document: &Document, first: Option<ObjectId>) -> Vec<ObjectId> {
    let mut chain = Vec::new();
    let mut next = first;
    while let Some(id) = next {
        if chain.len() >= 100_000 || chain.contains(&id) {
            break;
        }
        chain.push(id);
        next = document
            .get_dictionary(id)
            .ok()
            .and_then(|dictionary| dictionary.get(b"Next").and_then(Object::as_reference).ok());
    }
    chain
}

/// Set `Count` on a node to the number of entries visible under it, at every
/// level, and return that number.
///
/// Every entry is open — Chromium writes them so, and nothing here closes one
/// — so the count of a node is all of its descendants. A node with nothing
/// under it carries no `Count` at all, which is what the specification wants.
fn recount(document: &mut Document, node: ObjectId) -> Result<i64, Error> {
    let first = document
        .get_dictionary(node)
        .map_err(fail)?
        .get(b"First")
        .and_then(Object::as_reference)
        .ok();
    let children = siblings(document, first);
    let mut total = 0;
    for child in &children {
        total += 1 + recount(document, *child)?;
    }
    let dictionary = document.get_dictionary_mut(node).map_err(fail)?;
    if total > 0 {
        dictionary.set("Count", Object::Integer(total));
    } else {
        dictionary.remove(b"Count");
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// The outline, after printing.

/// One entry of the outline, with what hangs under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    pub title: String,
    /// The 1-based page in the file the entry points at. Nought when it points
    /// at nothing that could be resolved, which nothing Chromium writes does.
    pub page: usize,
    pub children: Vec<OutlineItem>,
}

/// What to do with the outline the browser wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineTreatment {
    /// Leave it in the file. Off is `--no-outline`.
    pub keep: bool,
    /// Entries deeper than this are cut, `--outline-depth`. Nought cuts them
    /// all.
    pub depth: u32,
}

/// Bound the outline to a depth, read what is left, and take it out of the
/// file when it was only wanted for reading.
///
/// The print call can be asked for the outline or not, and nothing in between:
/// the depth wkhtmltopdf lets a user set is cut here, after the fact, by
/// unhooking the children of every entry at the boundary. What is read back is
/// what a reader will show, so `--dump-outline` describes the outline the file
/// actually carries.
pub fn outline(
    pdf: &[u8],
    treatment: &OutlineTreatment,
) -> Result<(Vec<u8>, Vec<OutlineItem>), Error> {
    let mut document = Document::load_mem(pdf).map_err(fail)?;
    let Some(root) = outline_root(&document) else {
        return Ok((pdf.to_vec(), Vec::new()));
    };

    let page_numbers: BTreeMap<ObjectId, usize> = document
        .get_pages()
        .into_iter()
        .map(|(number, id)| (id, number as usize))
        .collect();
    let items = if treatment.depth == 0 {
        Vec::new()
    } else {
        let first = document
            .get_dictionary(root)
            .map_err(fail)?
            .get(b"First")
            .and_then(Object::as_reference)
            .ok();
        read_and_cut(&mut document, first, 1, treatment.depth, &page_numbers)?
    };

    if treatment.keep && !items.is_empty() {
        recount(&mut document, root)?;
    } else {
        document.catalog_mut().map_err(fail)?.remove(b"Outlines");
    }
    // Whatever was unhooked is unreachable now, and this is what removes it
    // rather than leaving a title in the bytes that no reader shows.
    document.prune_objects();

    let mut out = Vec::with_capacity(pdf.len());
    document.save_to(&mut out).map_err(fail)?;
    Ok((out, items))
}

/// Read a chain of entries at `level`, descending while the depth allows and
/// unhooking the children of entries at the boundary.
fn read_and_cut(
    document: &mut Document,
    first: Option<ObjectId>,
    level: u32,
    depth: u32,
    page_numbers: &BTreeMap<ObjectId, usize>,
) -> Result<Vec<OutlineItem>, Error> {
    let mut items = Vec::new();
    for id in siblings(document, first) {
        let (title, page, child) = {
            let dictionary = document.get_dictionary(id).map_err(fail)?;
            (
                dictionary
                    .get(b"Title")
                    .ok()
                    .and_then(|title| title.as_str().ok())
                    .map(decode_text)
                    .unwrap_or_default(),
                page_of(document, dictionary, page_numbers),
                dictionary.get(b"First").and_then(Object::as_reference).ok(),
            )
        };
        let children = if level < depth {
            read_and_cut(document, child, level + 1, depth, page_numbers)?
        } else {
            let dictionary = document.get_dictionary_mut(id).map_err(fail)?;
            dictionary.remove(b"First");
            dictionary.remove(b"Last");
            dictionary.remove(b"Count");
            Vec::new()
        };
        items.push(OutlineItem {
            title,
            page,
            children,
        });
    }
    Ok(items)
}

/// The page an entry's destination names, as a 1-based number.
///
/// Chromium writes `Dest [page /XYZ x y 0]`, page as a reference. The array
/// may itself be indirect, so it is dereferenced first.
fn page_of(
    document: &Document,
    entry: &lopdf::Dictionary,
    page_numbers: &BTreeMap<ObjectId, usize>,
) -> usize {
    entry
        .get(b"Dest")
        .ok()
        .and_then(|dest| document.dereference(dest).ok())
        .and_then(|(_, dest)| dest.as_array().ok())
        .and_then(|array| array.first())
        .and_then(|page| page.as_reference().ok())
        .and_then(|id| page_numbers.get(&id).copied())
        .unwrap_or(0)
}

/// A PDF text string as a reader shows it: Latin-1, or UTF-16 big endian when
/// the byte order mark says so.
fn decode_text(bytes: &[u8]) -> String {
    if let Some(units) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = units
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    bytes.iter().map(|byte| *byte as char).collect()
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
    let outline = part_outline(&source);

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
        outline: outline.map(|part| PartOutline {
            first: map[&part.first],
            last: map[&part.last],
        }),
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

    fn decode(bytes: &[u8]) -> String {
        decode_text(bytes)
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

    // --- the outline ----------------------------------------------------------

    /// A heading and what hangs under it, for building an outline by hand.
    struct Node(&'static str, usize, Vec<Node>);

    fn leaf(title: &'static str, page: usize) -> Node {
        Node(title, page, Vec::new())
    }

    /// The page document with an outline over it, pages given 0-based.
    fn outlined(labels: &[&str], tree: Vec<Node>) -> Vec<u8> {
        let mut document = Document::load_mem(&pages(labels, None)).expect("should parse");
        let page_ids: Vec<ObjectId> = document.page_iter().collect();
        let root = document.new_object_id();
        let (first, last, count) = add_items(&mut document, &tree, root, &page_ids);
        let mut dictionary = dictionary! { "Type" => "Outlines" };
        if let (Some(first), Some(last)) = (first, last) {
            dictionary.set("First", first);
            dictionary.set("Last", last);
            dictionary.set("Count", count);
        }
        document
            .objects
            .insert(root, Object::Dictionary(dictionary));
        document
            .catalog_mut()
            .expect("a catalog")
            .set("Outlines", Object::Reference(root));
        let mut out = Vec::new();
        document.save_to(&mut out).expect("should save");
        out
    }

    fn add_items(
        document: &mut Document,
        nodes: &[Node],
        parent: ObjectId,
        page_ids: &[ObjectId],
    ) -> (Option<ObjectId>, Option<ObjectId>, i64) {
        let ids: Vec<ObjectId> = nodes.iter().map(|_| document.new_object_id()).collect();
        let mut total = 0;
        for (index, node) in nodes.iter().enumerate() {
            let (first, last, count) = add_items(document, &node.2, ids[index], page_ids);
            let mut dictionary = dictionary! {
                "Title" => text_string(node.0),
                "Dest" => vec![
                    Object::Reference(page_ids[node.1]),
                    Object::Name(b"XYZ".to_vec()),
                    0.into(), 0.into(), 0.into(),
                ],
                "Parent" => parent,
            };
            if index > 0 {
                dictionary.set("Prev", ids[index - 1]);
            }
            if index + 1 < ids.len() {
                dictionary.set("Next", ids[index + 1]);
            }
            if let (Some(first), Some(last)) = (first, last) {
                dictionary.set("First", first);
                dictionary.set("Last", last);
                dictionary.set("Count", count);
            }
            document
                .objects
                .insert(ids[index], Object::Dictionary(dictionary));
            total += 1 + count;
        }
        (ids.first().copied(), ids.last().copied(), total)
    }

    fn item(title: &str, page: usize, children: Vec<OutlineItem>) -> OutlineItem {
        OutlineItem {
            title: title.into(),
            page,
            children,
        }
    }

    fn keep(depth: u32) -> OutlineTreatment {
        OutlineTreatment { keep: true, depth }
    }

    /// Two chapters over two pages, the first with a section and a subsection.
    fn chapters() -> Vec<u8> {
        outlined(
            &["P1", "P2"],
            vec![
                Node("One", 0, vec![Node("One A", 0, vec![leaf("Deep", 0)])]),
                leaf("Two", 1),
            ],
        )
    }

    #[test]
    fn the_outline_reads_back_with_its_nesting_and_pages() {
        let (_, items) = outline(&chapters(), &keep(4)).expect("should read");
        assert_eq!(
            items,
            [
                item(
                    "One",
                    1,
                    vec![item("One A", 1, vec![item("Deep", 1, vec![])])]
                ),
                item("Two", 2, vec![]),
            ]
        );
    }

    /// **The point of carrying outlines through a merge.** The second document's
    /// entries follow the first's, and point at the pages they now sit on.
    #[test]
    fn merging_joins_the_outlines_and_moves_their_pages() {
        let appendix = outlined(&["A1"], vec![leaf("Appendix", 0)]);
        let out = merge(&[&chapters(), &appendix]).expect("should merge");

        let (out, items) = outline(&out, &keep(4)).expect("should read");
        let titles: Vec<&str> = items.iter().map(|item| item.title.as_str()).collect();
        assert_eq!(titles, ["One", "Two", "Appendix"]);
        assert_eq!(items[2].page, 3, "the appendix is on the third page now");

        // One root, counted over every level, with every top-level entry
        // hanging from it rather than from a root that is gone.
        let document = Document::load_mem(&out).expect("should parse");
        let root = outline_root(&document).expect("an outline");
        let dictionary = document.get_dictionary(root).expect("a root");
        assert_eq!(
            dictionary.get(b"Count").and_then(Object::as_i64).ok(),
            Some(5)
        );
        for id in siblings(
            &document,
            Some(
                dictionary
                    .get(b"First")
                    .and_then(Object::as_reference)
                    .unwrap(),
            ),
        ) {
            let parent = document
                .get_dictionary(id)
                .and_then(|d| d.get(b"Parent"))
                .and_then(Object::as_reference)
                .expect("a parent");
            assert_eq!(parent, root);
        }
    }

    /// A document with no outline contributes nothing, and does not stop the
    /// others from being joined.
    #[test]
    fn a_document_without_an_outline_merges_between_two_that_have_one() {
        let out = merge(&[
            &outlined(&["A"], vec![leaf("First", 0)]),
            &pages(&["B"], None),
            &outlined(&["C"], vec![leaf("Third", 0)]),
        ])
        .expect("should merge");
        let (_, items) = outline(&out, &keep(4)).expect("should read");
        let titles: Vec<&str> = items.iter().map(|item| item.title.as_str()).collect();
        assert_eq!(titles, ["First", "Third"]);
        assert_eq!(items[1].page, 3);
    }

    /// Cut at a depth, the entries below it are gone from the file, not only
    /// from what was read back.
    #[test]
    fn the_depth_cuts_the_tree_and_prunes_what_was_cut() {
        let (out, items) = outline(&chapters(), &keep(2)).expect("should read");
        assert_eq!(
            items,
            [
                item("One", 1, vec![item("One A", 1, vec![])]),
                item("Two", 2, vec![]),
            ]
        );
        assert!(
            !String::from_utf8_lossy(&out).contains("Deep"),
            "the cut entry was left in the bytes"
        );

        let document = Document::load_mem(&out).expect("should parse");
        let root = outline_root(&document).expect("an outline");
        let count = document
            .get_dictionary(root)
            .and_then(|d| d.get(b"Count"))
            .and_then(Object::as_i64)
            .expect("a count");
        assert_eq!(count, 3, "the count follows the cut");
    }

    /// `--no-outline --dump-outline x`: read, then taken out of the file.
    #[test]
    fn not_keeping_the_outline_still_reads_it_first() {
        let treatment = OutlineTreatment {
            keep: false,
            depth: 4,
        };
        let (out, items) = outline(&chapters(), &treatment).expect("should read");
        assert_eq!(items.len(), 2);

        let document = Document::load_mem(&out).expect("should parse");
        assert!(
            outline_root(&document).is_none(),
            "the outline is still there"
        );
        assert!(
            !String::from_utf8_lossy(&out).contains("One A"),
            "an unhooked entry was left in the bytes"
        );
        assert_eq!(document.get_pages().len(), 2, "the pages are untouched");
    }

    #[test]
    fn a_depth_of_nought_leaves_no_outline() {
        let (out, items) = outline(&chapters(), &keep(0)).expect("should read");
        assert!(items.is_empty());
        let document = Document::load_mem(&out).expect("should parse");
        assert!(outline_root(&document).is_none());
    }

    #[test]
    fn a_document_without_an_outline_is_returned_untouched() {
        let plain = two_pages();
        let (out, items) = outline(&plain, &keep(4)).expect("should read");
        assert_eq!(out, plain);
        assert!(items.is_empty());
    }

    /// Chromium writes a title with an accent as UTF-16, and it has to read back
    /// as the heading said.
    #[test]
    fn a_title_outside_latin_1_reads_back() {
        let (_, items) = outline(&outlined(&["A"], vec![leaf("Résumé — plan", 0)]), &keep(4))
            .expect("should read");
        assert_eq!(items[0].title, "Résumé — plan");
    }
}
