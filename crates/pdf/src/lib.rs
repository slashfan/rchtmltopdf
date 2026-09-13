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
//! carried the first document's title and nothing else has a better claim; a
//! table of contents this program wrote is passed over, as wkhtmltopdf passed
//! over its own, so `toc` written first does not name the file after itself.
//! The outlines, joined end to end under one root, so a reader's sidebar lists
//! every document's headings in order (#40). The links, with every named
//! destination resolved to the page it meant before the name table it lived in
//! is left behind, and a link to another document of the same conversion
//! pointed into it (#41). Everything else that hung off a catalog — the
//! structure tree above all — is left behind: merging tagged structure is a
//! project of its own, and wkhtmltopdf never wrote any.
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

use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use rchtmltopdf_core::Clock;
use rchtmltopdf_core::settings::LinkSettings;
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

/// The document's own title, as the print call left it.
///
/// Chromium writes the `<title>` element into the Info dictionary of every
/// document it prints, which is the only place a conversion can read it back:
/// the page is gone by the time the bands are drawn, and `[title]` is the
/// current document's own rather than the file's (#110). `None` when the file
/// carries no title at all, which is not the same as an empty one.
pub fn title(pdf: &[u8]) -> Result<Option<String>, Error> {
    let document = Document::load_mem(pdf).map_err(fail)?;
    let Ok(entry) = document.trailer.get(b"Info") else {
        return Ok(None);
    };
    let dictionary = match entry {
        Object::Reference(id) => match document.get_dictionary(*id) {
            Ok(dictionary) => dictionary,
            Err(_) => return Ok(None),
        },
        Object::Dictionary(dictionary) => dictionary,
        _ => return Ok(None),
    };
    Ok(dictionary
        .get(b"Title")
        .ok()
        .and_then(|title| title.as_str().ok())
        .map(decode_text))
}

/// One printed document going into a merge, and what to do with its links.
#[derive(Debug, Clone)]
pub struct Part<'a> {
    pub pdf: &'a [u8],
    /// The URL the document was printed from. A link to it from another part
    /// is pointed into it, and a link below its directory can be made relative
    /// again.
    pub url: &'a str,
    pub links: &'a LinkSettings,
    /// A table of contents this program wrote, rather than a document the user
    /// gave. It has a `<title>` like any other part and is never the one the
    /// merged file is named after: wkhtmltopdf passed over its contents
    /// objects when it picked the title, so a conversion that starts with
    /// `toc` is still named after the first document.
    pub contents: bool,
}

/// What a merge produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub pdf: Vec<u8>,
    /// How many pages each part contributed, in order. What the page numbers
    /// are worked out from (#39).
    pub pages: Vec<usize>,
}

/// Combine several printed documents into one, in the order given.
///
/// One document that asks nothing of its links comes back untouched, bytes for
/// bytes: there is nothing to combine, and rewriting a file that does not need
/// it would only give a single-document conversion a way to differ from what
/// Chromium printed. One that does ask is edited in place, its catalog kept.
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
pub fn merge(parts: &[Part<'_>]) -> Result<Merged, Error> {
    match parts {
        [] => {
            return Err(Error {
                reason: "there is no document to merge".into(),
            });
        }
        [only] => return alone(only),
        _ => {}
    }

    let mut merged = Document::with_version("1.4");
    let mut pages = Vec::new();
    let mut info = None;
    let mut outlines: Vec<PartOutline> = Vec::new();
    let mut absorbed_parts = Vec::with_capacity(parts.len());

    for (index, part) in parts.iter().enumerate() {
        let source = Document::load_mem(part.pdf)
            .map_err(|error| fail(format!("document {}: {error}", index + 1)))?;
        // The highest version any part asks for. A feature a later part used
        // is still used after the merge.
        if source.version > merged.version {
            merged.version = source.version.clone();
        }
        let mut absorbed = absorb(&mut merged, source)
            .map_err(|error| fail(format!("document {}: {}", index + 1, error.reason)))?;
        pages.extend(absorbed.pages.iter().copied());
        if info.is_none() && !part.contents {
            info = absorbed.info;
        }
        outlines.extend(absorbed.outline.take());
        absorbed_parts.push(absorbed);
    }

    // Every document's anchors, known before any link is judged: a link from
    // the first document to the last has to find it.
    let anchors: Vec<Anchor<'_>> = parts
        .iter()
        .zip(&absorbed_parts)
        .map(|(part, absorbed)| Anchor {
            url: part.url,
            names: &absorbed.names,
            first_page: absorbed.pages[0],
        })
        .collect();
    for (index, (part, absorbed)) in parts.iter().zip(&absorbed_parts).enumerate() {
        treat_links(&mut merged, &absorbed.pages, part, &anchors, index)?;
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

    // Now that every page of the finished file has a number, the table of
    // contents can be pointed at the headings it lists (D42).
    point_contents_at_headings(&mut merged)?;

    // The old catalogs, page trees, outlines and object streams are no longer
    // reachable from the trailer, and this is what removes them.
    merged.prune_objects();
    merged.renumber_objects();

    let mut out = Vec::with_capacity(parts.iter().map(|part| part.pdf.len()).sum());
    merged.save_to(&mut out).map_err(fail)?;
    Ok(Merged {
        pdf: out,
        pages: absorbed_parts.iter().map(|part| part.pages.len()).collect(),
    })
}

/// One document alone: nothing to combine, and its catalog kept whole.
fn alone(part: &Part<'_>) -> Result<Merged, Error> {
    let mut document = Document::load_mem(part.pdf).map_err(fail)?;
    let pages: Vec<ObjectId> = document.page_iter().collect();
    if pages.is_empty() {
        return Err(fail("it has no pages"));
    }
    // Before the shortcut below: a table of contents converted on its own is
    // one document, and it still carries a link to its own heading (D42).
    let pointed = point_contents_at_headings(&mut document)?;
    if part.links.leaves_everything() && !pointed {
        return Ok(Merged {
            pdf: part.pdf.to_vec(),
            pages: vec![pages.len()],
        });
    }
    let names = named_destinations(&document);
    let anchors = [Anchor {
        url: part.url,
        names: &names,
        first_page: pages[0],
    }];
    treat_links(&mut document, &pages, part, &anchors, 0)?;

    let mut out = Vec::with_capacity(part.pdf.len());
    document.save_to(&mut out).map_err(fail)?;
    Ok(Merged {
        pdf: out,
        pages: vec![pages.len()],
    })
}

/// What one document contributed to a merge, numbered as the target knows it.
struct Absorbed {
    /// Its pages, in reading order.
    pages: Vec<ObjectId>,
    /// Its Info dictionary, when it had one that resolves.
    info: Option<ObjectId>,
    /// Its outline, when it had one with something in it.
    outline: Option<PartOutline>,
    /// Its named destinations, each resolved to an explicit one.
    names: BTreeMap<Vec<u8>, Object>,
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
// Links.

/// Where the anchors of one document of the conversion are.
struct Anchor<'a> {
    url: &'a str,
    /// Name to explicit destination, numbered as the target document knows it.
    names: &'a BTreeMap<Vec<u8>, Object>,
    /// Where a link to the document with no fragment, or an unknown one, lands.
    first_page: ObjectId,
}

/// A link is one of two kinds, and the options switch each kind off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkKind {
    /// To an anchor: in this document, or in another document of the same
    /// conversion. wkhtmltopdf called these local.
    Internal,
    /// To anywhere else.
    External,
}

/// The catalog's named destinations, each resolved to an explicit one.
///
/// Chromium writes `<a href="#x">` as `/Dest /x` on the annotation and `/x`
/// in the catalog's `Dests` dictionary, so the annotation means nothing
/// without the catalog it came with. A name tree under `Names` is the other
/// place the specification allows and Chromium does not use; it is not read.
fn named_destinations(document: &Document) -> BTreeMap<Vec<u8>, Object> {
    let mut names = BTreeMap::new();
    let Some(dests) = document
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"Dests").ok())
        .and_then(|entry| document.dereference(entry).ok())
        .and_then(|(_, entry)| entry.as_dict().ok())
    else {
        return names;
    };
    for (name, value) in dests.iter() {
        let Ok((_, value)) = document.dereference(value) else {
            continue;
        };
        // Either the array itself, or a dictionary carrying it under `D`.
        let explicit = match value {
            Object::Array(_) => Some(value.clone()),
            Object::Dictionary(dictionary) => dictionary
                .get(b"D")
                .ok()
                .and_then(|d| document.dereference(d).ok())
                .filter(|(_, d)| d.as_array().is_ok())
                .map(|(_, d)| d.clone()),
            _ => None,
        };
        if let Some(explicit) = explicit {
            names.insert(name.clone(), explicit);
        }
    }
    names
}

/// The scheme a generated table of contents points at a heading with.
///
/// **Why a URL and not a destination outright.** The entries are laid out by
/// the browser, and only the browser knows where each line ended up on the
/// page; a link annotation has to be the one it wrote for an `<a href>`. So
/// the table names its target in the href — `rchtmltopdf-contents:PAGE,LEFT,TOP`,
/// the page of the finished file and the place on it — Chromium writes that
/// through verbatim as a `URI` action, and the merge turns each one into the
/// destination it was always naming. Chromium
/// preserves an unknown scheme rather than resolving it against the document,
/// which a relative-looking marker would have been.
pub const CONTENTS_SCHEME: &str = "rchtmltopdf-contents:";

/// Turn every table-of-contents marker into the destination it names.
///
/// Runs once the whole file exists, because the page a marker names is a page
/// of the *finished* file and nothing before the merge has one. A marker
/// naming a page the file does not have loses its annotation rather than
/// keeping a link that goes nowhere.
fn point_contents_at_headings(document: &mut Document) -> Result<bool, Error> {
    let pages: Vec<ObjectId> = document.page_iter().collect();
    let mut any = false;
    for page in &pages {
        let annotations = page_annotations(document, *page);
        if annotations.is_empty() {
            continue;
        }
        let mut kept: Vec<Object> = Vec::with_capacity(annotations.len());
        let mut changed = false;
        for annotation in annotations {
            let Some(target) = contents_target(document, annotation) else {
                kept.push(Object::Reference(annotation));
                continue;
            };
            changed = true;
            let Some(id) = pages.get(target.page.saturating_sub(1)) else {
                // The table names a page that is not there: drop the link
                // rather than leave one that goes nowhere.
                continue;
            };
            let destination = Object::Array(vec![
                Object::Reference(*id),
                Object::Name(b"XYZ".to_vec()),
                Object::Real(target.left as f32),
                Object::Real(target.top as f32),
                Object::Integer(0),
            ]);
            let entry = document.get_dictionary_mut(annotation).map_err(fail)?;
            entry.remove(b"A");
            entry.set("Dest", destination);
            kept.push(Object::Reference(annotation));
        }
        if changed {
            any = true;
            document
                .get_dictionary_mut(*page)
                .map_err(fail)?
                .set("Annots", Object::Array(kept));
        }
    }
    Ok(any)
}

/// Where a table-of-contents marker points, if this annotation is one.
fn contents_target(document: &Document, annotation: ObjectId) -> Option<ContentsTarget> {
    let uri = document
        .get_dictionary(annotation)
        .ok()?
        .get(b"A")
        .ok()
        .and_then(|action| document.dereference(action).ok())
        .and_then(|(_, action)| action.as_dict().ok().cloned())?
        .get(b"URI")
        .ok()
        .and_then(|uri| uri.as_str().ok())
        .map(|uri| String::from_utf8_lossy(uri).into_owned())?;
    let written = uri.strip_prefix(CONTENTS_SCHEME)?;
    let mut parts = written.split(',');
    Some(ContentsTarget {
        page: parts.next()?.parse().ok()?,
        left: parts.next()?.parse().ok()?,
        top: parts.next()?.parse().ok()?,
    })
}

/// A page of the finished file, and the place on it a heading sits.
struct ContentsTarget {
    page: usize,
    left: f64,
    top: f64,
}

/// Judge every link on these pages: resolve what names an anchor, point a
/// link to another document of the conversion into it, make a relative link
/// relative again when asked, and drop the kinds that were switched off.
fn treat_links(
    document: &mut Document,
    pages: &[ObjectId],
    part: &Part<'_>,
    anchors: &[Anchor<'_>],
    own: usize,
) -> Result<(), Error> {
    for page in pages {
        let annotations = page_annotations(document, *page);
        if annotations.is_empty() {
            continue;
        }
        let mut kept = Vec::with_capacity(annotations.len());
        let mut dropped = false;
        for annotation in annotations {
            let keep = match link_kind(document, annotation, part, anchors, own)? {
                Some(LinkKind::Internal) => part.links.internal,
                Some(LinkKind::External) => part.links.external,
                None => true,
            };
            if keep {
                kept.push(Object::Reference(annotation));
            } else {
                dropped = true;
            }
        }
        if dropped {
            // Written straight onto the page, whether or not it was indirect
            // before: the old array is either shared with nobody or pruned.
            document
                .get_dictionary_mut(*page)
                .map_err(fail)?
                .set("Annots", Object::Array(kept));
        }
    }
    Ok(())
}

/// The annotations of a page, by reference.
///
/// Only those written as references, which is how Chromium writes every one.
/// An annotation written inline in the array is left alone rather than
/// judged, because it has no number to keep or drop by.
fn page_annotations(document: &Document, page: ObjectId) -> Vec<ObjectId> {
    document
        .get_dictionary(page)
        .ok()
        .and_then(|dictionary| dictionary.get(b"Annots").ok())
        .and_then(|annots| document.dereference(annots).ok())
        .and_then(|(_, annots)| annots.as_array().ok())
        .map(|array| {
            array
                .iter()
                .filter_map(|item| item.as_reference().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// What kind of link an annotation is, rewriting it on the way where the
/// conversion knows better than the browser did.
///
/// `None` for an annotation that is not a link, or a link that goes nowhere.
fn link_kind(
    document: &mut Document,
    annotation: ObjectId,
    part: &Part<'_>,
    anchors: &[Anchor<'_>],
    own: usize,
) -> Result<Option<LinkKind>, Error> {
    let dictionary = document.get_dictionary(annotation).map_err(fail)?;
    if dictionary.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Link") {
        return Ok(None);
    }

    // A destination, named or explicit. A name is resolved through the table
    // it came with, because that table does not survive a merge.
    if let Ok(destination) = dictionary.get(b"Dest") {
        let resolved = match destination {
            Object::Name(name) | Object::String(name, _) => anchors[own].names.get(name).cloned(),
            _ => None,
        };
        if let Some(explicit) = resolved {
            document
                .get_dictionary_mut(annotation)
                .map_err(fail)?
                .set("Dest", explicit);
        }
        return Ok(Some(LinkKind::Internal));
    }

    let Some(action) = dictionary
        .get(b"A")
        .ok()
        .and_then(|action| document.dereference(action).ok())
        .and_then(|(_, action)| action.as_dict().ok())
    else {
        return Ok(None);
    };
    match action.get(b"S").and_then(Object::as_name).ok() {
        Some(b"GoTo") => Ok(Some(LinkKind::Internal)),
        Some(b"URI") => {
            let Some(uri) = action
                .get(b"URI")
                .ok()
                .and_then(|uri| uri.as_str().ok())
                .map(|uri| String::from_utf8_lossy(uri).into_owned())
            else {
                return Ok(Some(LinkKind::External));
            };

            // A table of contents pointing at a heading. Neither of the user's
            // two kinds: it is this program's own link, written by this
            // program into a document it generated, so `--disable-external-
            // links` is not about it and must not take it away (D42). It is
            // turned into a destination once every page has a number, in
            // [`point_contents_at_headings`].
            if uri.starts_with(CONTENTS_SCHEME) {
                return Ok(None);
            }

            // A link to another document of this conversion is a link into the
            // file being written: wkhtmltopdf made it local, and so does this.
            if let Some(explicit) = destination_in(&uri, anchors) {
                let dictionary = document.get_dictionary_mut(annotation).map_err(fail)?;
                dictionary.remove(b"A");
                dictionary.set("Dest", explicit);
                return Ok(Some(LinkKind::Internal));
            }

            // The browser resolved every link before printing. The one thing
            // it can be asked to undo is a link below the document's own
            // directory, which is what a relative link almost always was.
            if !part.links.resolve_relative
                && let Some(relative) = relative_to(&uri, part.url)
            {
                document.get_dictionary_mut(annotation).map_err(fail)?.set(
                    "A",
                    Object::Dictionary(dictionary! {
                        "Type" => "Action",
                        "S" => "URI",
                        "URI" => Object::string_literal(relative),
                    }),
                );
            }
            Ok(Some(LinkKind::External))
        }
        // Launch, JavaScript, and anything else the browser might one day
        // write: not local, whatever it is.
        _ => Ok(Some(LinkKind::External)),
    }
}

/// Where a URI lands when it names a document of the conversion.
///
/// The document, exactly as the browser resolved it, with an optional
/// fragment. A fragment that names an anchor lands on it; no fragment, or one
/// the document does not have, lands on its first page.
fn destination_in(uri: &str, anchors: &[Anchor<'_>]) -> Option<Object> {
    let (document, fragment) = match uri.split_once('#') {
        Some((document, fragment)) => (document, Some(fragment)),
        None => (uri, None),
    };
    let anchor = anchors.iter().find(|anchor| anchor.url == document)?;
    if let Some(explicit) = fragment.and_then(|fragment| anchor.names.get(fragment.as_bytes())) {
        return Some(explicit.clone());
    }
    Some(Object::Array(vec![
        Object::Reference(anchor.first_page),
        Object::Name(b"Fit".to_vec()),
    ]))
}

/// A URI below the document's directory, written relative to it.
fn relative_to(uri: &str, document_url: &str) -> Option<String> {
    let directory = &document_url[..=document_url.rfind('/')?];
    let rest = uri.strip_prefix(directory)?;
    (!rest.is_empty()).then(|| rest.to_string())
}

// ---------------------------------------------------------------------------
// Stamping: the bands, drawn onto the pages.

/// Draw each page of `overlay` onto the same-numbered page of `document`.
///
/// The overlay is a document Chromium printed with one sheet per page of the
/// target, the bands positioned on each sheet where they belong on its page
/// (D38). The sheet's drawing is appended to the page's own content rather
/// than wrapped in a form XObject, because text inside a form is invisible to
/// every extractor the tests use, and a footer nobody can extract fails the
/// compatibility matrix's own assertions (#38). Appending means the sheet's
/// resources move onto the page under names of their own, and every operator
/// that names a resource is rewritten to match.
///
/// The page's own drawing is wrapped in `q` … `Q` first, so whatever state or
/// transformation it left behind cannot move the band.
pub fn stamp(document: &[u8], overlay: &[u8]) -> Result<Vec<u8>, Error> {
    let mut target = Document::load_mem(document).map_err(fail)?;
    let sheets =
        Document::load_mem(overlay).map_err(|error| fail(format!("the band document: {error}")))?;
    let pages: Vec<ObjectId> = target.page_iter().collect();

    let absorbed = absorb(&mut target, sheets)
        .map_err(|error| fail(format!("the band document: {}", error.reason)))?;
    if absorbed.pages.len() != pages.len() {
        return Err(fail(format!(
            "the band document has {} pages for a document of {}; a sheet did not fit its page",
            absorbed.pages.len(),
            pages.len()
        )));
    }

    for (index, (page, sheet)) in pages.iter().zip(&absorbed.pages).enumerate() {
        stamp_page(&mut target, *page, *sheet, index)?;
    }
    // The overlay's own catalog and page tree, and the sheets themselves now
    // that their drawing and resources have moved on.
    target.prune_objects();

    let mut out = Vec::with_capacity(document.len() + overlay.len());
    target.save_to(&mut out).map_err(fail)?;
    Ok(out)
}

/// The resource categories whose entries are named by operators.
///
/// `ProcSet` is an array rather than names, and nothing reads it; it is left
/// as the page had it.
const RESOURCE_CATEGORIES: [&[u8]; 6] = [
    b"Font",
    b"XObject",
    b"ExtGState",
    b"ColorSpace",
    b"Pattern",
    b"Shading",
];

/// Which operators name a resource, and in which category.
fn category_of(operator: &str) -> Option<&'static [u8]> {
    Some(match operator {
        "Tf" => b"Font",
        "Do" => b"XObject",
        "gs" => b"ExtGState",
        "cs" | "CS" => b"ColorSpace",
        "scn" | "SCN" => b"Pattern",
        "sh" => b"Shading",
        _ => return None,
    })
}

fn stamp_page(
    document: &mut Document,
    page: ObjectId,
    sheet: ObjectId,
    index: usize,
) -> Result<(), Error> {
    // The sheet's resources, by category. `absorb` has copied down anything
    // the sheet inherited, so they are on the sheet itself.
    let sheet_resources = document
        .get_dictionary(sheet)
        .map_err(fail)?
        .get(b"Resources")
        .ok()
        .and_then(|resources| document.dereference(resources).ok())
        .and_then(|(_, resources)| resources.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    let prefix = format!("Ov{index}");
    let mut moved: Vec<(&[u8], Vec<u8>, Object)> = Vec::new();
    for category in RESOURCE_CATEGORIES {
        let Some(entries) = sheet_resources
            .get(category)
            .ok()
            .and_then(|entries| document.dereference(entries).ok())
            .and_then(|(_, entries)| entries.as_dict().ok())
        else {
            continue;
        };
        for (name, value) in entries.iter() {
            let mut renamed = prefix.clone().into_bytes();
            renamed.extend_from_slice(name);
            moved.push((category, renamed, value.clone()));
        }
    }
    let renamed = |category: &[u8], name: &[u8]| -> Option<Vec<u8>> {
        moved
            .iter()
            .find(|(c, r, _)| *c == category && r[prefix.len()..] == *name)
            .map(|(_, r, _)| r.clone())
    };

    // The sheet's drawing, every resource it names renamed to match.
    let raw = document.get_page_content(sheet);
    let mut content = Content::decode(&raw).map_err(|error| {
        fail(format!(
            "the band document's page {} would not decode: {error}",
            index + 1
        ))
    })?;
    for operation in &mut content.operations {
        let Some(category) = category_of(&operation.operator) else {
            continue;
        };
        for operand in &mut operation.operands {
            if let Object::Name(name) = operand
                && let Some(new) = renamed(category, name)
            {
                *name = new;
            }
        }
    }
    let drawing = content.encode().map_err(fail)?;

    // Onto the page: its resources first.
    let resources = own_resources(document, page)?;
    for (category, name, value) in moved {
        let entries = own_dictionary(document, resources, category)?;
        document
            .get_dictionary_mut(entries)
            .map_err(fail)?
            .set(name, value);
    }

    // Then its content: the page's own, wrapped, and the sheet's after it.
    let save = document.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
    let restore = document.add_object(Stream::new(dictionary! {}, b"Q\n".to_vec()));
    let band = document.add_object(Stream::new(dictionary! {}, drawing));
    let dictionary = document.get_dictionary_mut(page).map_err(fail)?;
    let mut contents = vec![Object::Reference(save)];
    match dictionary.get(b"Contents") {
        Ok(Object::Array(existing)) => contents.extend(existing.iter().cloned()),
        Ok(other) => contents.push(other.clone()),
        Err(_) => {}
    }
    contents.push(Object::Reference(restore));
    contents.push(Object::Reference(band));
    dictionary.set("Contents", Object::Array(contents));
    Ok(())
}

/// The page's resource dictionary as an object of its own, made so if it was
/// written inline, so it can be edited by number.
fn own_resources(document: &mut Document, page: ObjectId) -> Result<ObjectId, Error> {
    own_dictionary(document, page, b"Resources")
}

/// The dictionary under `key` of `holder` as an object of its own: moved out
/// if it was inline, created if it was absent, and shared with nothing else
/// afterwards — a page whose resources were a dictionary shared with another
/// page must not gain that page's bands.
fn own_dictionary(
    document: &mut Document,
    holder: ObjectId,
    key: &[u8],
) -> Result<ObjectId, Error> {
    let current = document
        .get_dictionary(holder)
        .map_err(fail)?
        .get(key)
        .ok()
        .cloned();
    let owned = match current {
        Some(Object::Reference(id)) => {
            let copy = document.get_dictionary(id).map_err(fail)?.clone();
            document.add_object(Object::Dictionary(copy))
        }
        Some(Object::Dictionary(inline)) => document.add_object(Object::Dictionary(inline)),
        _ => document.add_object(Object::Dictionary(Dictionary::new())),
    };
    document
        .get_dictionary_mut(holder)
        .map_err(fail)?
        .set(key.to_vec(), Object::Reference(owned));
    Ok(owned)
}

// ---------------------------------------------------------------------------
// The outline, after printing.

/// One entry of the outline, with what hangs under it.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    /// The 1-based page in the file the entry points at. Nought when it points
    /// at nothing that could be resolved, which nothing Chromium writes does.
    pub page: usize,
    /// Where on that page the heading sits, in points from the bottom-left.
    ///
    /// Chromium writes `Dest [page /XYZ left top 0]`, so this is the heading's
    /// own position rather than the corner of the page — which is what lets a
    /// table of contents point *at the heading* and not merely at the page it
    /// is on (D42). Nought when the destination did not say.
    pub left: f64,
    pub top: f64,
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
        let (title, (page, left, top), child) = {
            let dictionary = document.get_dictionary(id).map_err(fail)?;
            (
                dictionary
                    .get(b"Title")
                    .ok()
                    .and_then(|title| title.as_str().ok())
                    .map(decode_text)
                    .unwrap_or_default(),
                destination_of(document, dictionary, page_numbers),
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
            left,
            top,
            children,
        });
    }
    Ok(items)
}

/// Where an entry's destination points: the 1-based page, and the place on it.
///
/// Chromium writes `Dest [page /XYZ left top 0]`, page as a reference. The
/// array may itself be indirect, so it is dereferenced first. A destination
/// that says nothing readable is nought throughout, which no destination
/// Chromium writes does.
fn destination_of(
    document: &Document,
    entry: &lopdf::Dictionary,
    page_numbers: &BTreeMap<ObjectId, usize>,
) -> (usize, f64, f64) {
    let Some(array) = entry
        .get(b"Dest")
        .ok()
        .and_then(|dest| document.dereference(dest).ok())
        .and_then(|(_, dest)| dest.as_array().ok().cloned())
    else {
        return (0, 0.0, 0.0);
    };
    let page = array
        .first()
        .and_then(|page| page.as_reference().ok())
        .and_then(|id| page_numbers.get(&id).copied())
        .unwrap_or(0);
    // `[page /XYZ left top zoom]`: the two numbers after the name.
    let number = |at: usize| {
        array
            .get(at)
            .and_then(|value| value.as_float().ok())
            .unwrap_or(0.0) as f64
    };
    (page, number(2), number(3))
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
    let mut names = named_destinations(&source);

    let objects = std::mem::take(&mut source.objects);
    let map: BTreeMap<ObjectId, ObjectId> = objects
        .keys()
        .map(|old| (*old, into.new_object_id()))
        .collect();
    for (old, mut object) in objects {
        relabel(&mut object, &map);
        into.objects.insert(map[&old], object);
    }
    // The destinations name pages by their old numbers, like everything else.
    for destination in names.values_mut() {
        relabel(destination, &map);
    }

    Ok(Absorbed {
        pages: page_ids.iter().map(|id| map[id]).collect(),
        info: info.map(|id| map[&id]),
        outline: outline.map(|part| PartOutline {
            first: map[&part.first],
            last: map[&part.last],
        }),
        names,
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

    /// **What `[title]` is read from** (#110). Chromium writes each printed
    /// document's `<title>` into its Info dictionary, and that is where a band
    /// finds the title of the document it is being drawn on.
    #[test]
    fn a_parts_own_title_is_read_back() {
        let pdf = titled(&["A"], "The document's own title");
        assert_eq!(
            title(&pdf).expect("should parse"),
            Some("The document's own title".to_string())
        );
    }

    /// A file with no Info dictionary at all has no title, which is not the
    /// same as an empty one: `[doctitle]` falls back to it rather than
    /// printing nothing.
    #[test]
    fn a_part_without_an_info_dictionary_has_no_title() {
        let pdf = pages(&["A"], None);
        assert_eq!(title(&pdf).expect("should parse"), None);
    }

    /// A title outside ASCII travels as UTF-16BE, and comes back as itself.
    #[test]
    fn a_title_is_read_back_whatever_it_is_written_in() {
        let written = set_metadata(
            &pages(&["A"], None),
            &Metadata {
                title: Some("Facturé — 2026 ☕".to_string()),
                producer: "test".into(),
                creator: "test".into(),
                created: Clock::default(),
            },
        )
        .expect("should rewrite");
        assert_eq!(
            title(&written).expect("should parse"),
            Some("Facturé — 2026 ☕".to_string())
        );
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

    /// A part with nothing asked of its links, printed from nowhere in
    /// particular.
    fn part(pdf: &[u8]) -> Part<'_> {
        Part {
            pdf,
            url: "file:///nowhere/document.html",
            links: &DEFAULT_LINKS,
            contents: false,
        }
    }

    const DEFAULT_LINKS: LinkSettings = LinkSettings {
        external: true,
        internal: true,
        resolve_relative: true,
    };

    fn merge_all(parts: &[&[u8]]) -> Result<Vec<u8>, Error> {
        let parts: Vec<Part<'_>> = parts.iter().map(|pdf| part(pdf)).collect();
        merge(&parts).map(|merged| merged.pdf)
    }

    /// The counts the numbering is worked out from.
    #[test]
    fn a_merge_reports_how_many_pages_each_part_gave() {
        let a = pages(&["A1", "A2"], None);
        let b = pages(&["B1"], None);
        let parts = [part(&a), part(&b)];
        assert_eq!(merge(&parts).expect("should merge").pages, [2, 1]);
        assert_eq!(merge(&parts[..1]).expect("alone").pages, [2]);
    }

    #[test]
    fn pages_come_out_in_command_line_order() {
        let a = pages(&["A1", "A2"], None);
        let b = pages(&["B1"], None);
        let c = pages(&["C1", "C2", "C3"], None);

        let out = merge_all(&[&a, &b, &c]).expect("should merge");

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
        let out =
            merge_all(&[&pages(&["A"], None), &pages(&["B", "C"], None)]).expect("should merge");

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
        let out = merge_all(&[&pages(&["A"], None), &pages(&["B"], None)]).expect("should merge");

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
        let out = merge_all(&[&pages(&["A"], Some(inherited)), &pages(&["B"], None)])
            .expect("should merge");

        let document = Document::load_mem(&out).expect("should parse");
        let first = document.page_iter().next().expect("a page");
        let resources = document
            .get_dictionary(first)
            .and_then(|page| page.get(b"Resources"))
            .and_then(Object::as_dict)
            .expect("the page should carry the resources it inherited");
        assert!(resources.has(b"ProcSet"));
    }

    /// A conversion that starts with `toc` is named after the document, not
    /// after the contents page this program wrote: wkhtmltopdf passed over its
    /// contents objects when it picked the title.
    #[test]
    fn a_table_of_contents_does_not_name_the_document() {
        let contents = titled(&["C"], "Table of Contents");
        let document = titled(&["A"], "First");
        let parts = [
            Part {
                contents: true,
                ..part(&contents)
            },
            part(&document),
        ];
        let out = merge(&parts).expect("should merge").pdf;
        assert_eq!(info(&out, "Title").as_deref(), Some("First"));
    }

    /// wkhtmltopdf's output carried the first document's title.
    #[test]
    fn the_first_documents_info_dictionary_is_the_one_kept() {
        let out = merge_all(&[&titled(&["A"], "First"), &titled(&["B"], "Second")])
            .expect("should merge");
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

        let out = merge_all(&[&broken, &pages(&["B"], None)]).expect("should merge");

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
        assert_eq!(merge_all(&[&only]).expect("should pass through"), only);
    }

    #[test]
    fn nothing_to_merge_is_an_error_not_an_empty_file() {
        assert!(merge_all(&[]).is_err());
    }

    /// The failure names which document, because a command line can carry ten.
    #[test]
    fn a_part_that_is_not_a_pdf_is_named_by_position() {
        let error = merge_all(&[&two_pages(), b"not a PDF"]).expect_err("should refuse");
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
            left: 0.0,
            top: 0.0,
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
        let out = merge_all(&[&chapters(), &appendix]).expect("should merge");

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
        let out = merge_all(&[
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

    // --- links ----------------------------------------------------------------

    /// A link annotation to put on the first page of a hand-built document.
    enum LinkSpec {
        Named(&'static str),
        Explicit(usize),
        Uri(&'static str),
    }

    /// The page document with links on its first page and a `Dests` table
    /// naming pages, 0-based.
    fn linked(labels: &[&str], dests: &[(&str, usize)], links: &[LinkSpec]) -> Vec<u8> {
        let mut document = Document::load_mem(&pages(labels, None)).expect("should parse");
        let page_ids: Vec<ObjectId> = document.page_iter().collect();
        let explicit = |page: usize| {
            Object::Array(vec![
                Object::Reference(page_ids[page]),
                Object::Name(b"XYZ".to_vec()),
                0.into(),
                0.into(),
                0.into(),
            ])
        };

        let mut table = lopdf::Dictionary::new();
        for (name, page) in dests {
            table.set(name.as_bytes().to_vec(), explicit(*page));
        }
        let table = document.add_object(Object::Dictionary(table));
        let catalog = document.catalog_mut().expect("a catalog");
        catalog.set("Dests", Object::Reference(table));
        catalog.set("Lang", Object::string_literal("fr-FR"));

        let annotations: Vec<Object> = links
            .iter()
            .map(|link| {
                let mut annotation = dictionary! {
                    "Type" => "Annot",
                    "Subtype" => "Link",
                    "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                };
                match link {
                    LinkSpec::Named(name) => {
                        annotation.set("Dest", Object::Name(name.as_bytes().to_vec()))
                    }
                    LinkSpec::Explicit(page) => annotation.set("Dest", explicit(*page)),
                    LinkSpec::Uri(uri) => annotation.set(
                        "A",
                        Object::Dictionary(dictionary! {
                            "Type" => "Action", "S" => "URI", "URI" => Object::string_literal(*uri),
                        }),
                    ),
                }
                Object::Reference(document.add_object(Object::Dictionary(annotation)))
            })
            .collect();
        let annots = document.add_object(Object::Array(annotations));
        document
            .get_dictionary_mut(page_ids[0])
            .expect("a page")
            .set("Annots", Object::Reference(annots));

        let mut out = Vec::new();
        document.save_to(&mut out).expect("should save");
        out
    }

    /// What a link on a 1-based page does, as a reader would follow it.
    #[derive(Debug, PartialEq, Eq)]
    enum Followed {
        Named(String),
        Page(usize),
        Uri(String),
    }

    fn links_on(pdf: &[u8], page: usize) -> Vec<Followed> {
        let document = Document::load_mem(pdf).expect("should parse");
        let numbers: BTreeMap<ObjectId, usize> = document
            .get_pages()
            .into_iter()
            .map(|(number, id)| (id, number as usize))
            .collect();
        let id = document.get_pages()[&(page as u32)];
        page_annotations(&document, id)
            .into_iter()
            .map(|annotation| {
                let dictionary = document.get_dictionary(annotation).expect("an annotation");
                match dictionary.get(b"Dest") {
                    Ok(Object::Name(name)) => {
                        Followed::Named(String::from_utf8_lossy(name).into_owned())
                    }
                    Ok(Object::Array(array)) => {
                        Followed::Page(numbers[&array[0].as_reference().expect("a page")])
                    }
                    _ => {
                        let action = dictionary
                            .get(b"A")
                            .and_then(Object::as_dict)
                            .expect("an action");
                        let uri = action.get(b"URI").and_then(Object::as_str).expect("a URI");
                        Followed::Uri(String::from_utf8_lossy(uri).into_owned())
                    }
                }
            })
            .collect()
    }

    fn with_links<'a>(pdf: &'a [u8], url: &'a str, links: &'a LinkSettings) -> Part<'a> {
        Part {
            pdf,
            url,
            links,
            contents: false,
        }
    }

    /// **The reason named destinations are resolved.** The table they lived in
    /// is the catalog's, and the catalog does not survive a merge.
    #[test]
    fn a_named_destination_survives_the_merge_as_an_explicit_one() {
        let a = linked(
            &["A1", "A2"],
            &[("target", 1)],
            &[LinkSpec::Named("target")],
        );
        let b = pages(&["B1"], None);

        let out = merge_all(&[&b, &a]).expect("should merge");
        // A's first page is the second page now, and its target the third.
        assert_eq!(links_on(&out, 2), [Followed::Page(3)]);
    }

    /// A link to another document of the conversion is a link into the file:
    /// to the anchor it names, or to the document's first page.
    #[test]
    fn a_link_to_another_document_of_the_conversion_becomes_internal() {
        let a = linked(
            &["A1"],
            &[],
            &[
                LinkSpec::Uri("file:///d/b.html#sec"),
                LinkSpec::Uri("file:///d/b.html"),
                LinkSpec::Uri("file:///d/b.html#nowhere"),
                LinkSpec::Uri("file:///d/elsewhere.html"),
                LinkSpec::Uri("https://example.com/"),
            ],
        );
        let b = linked(&["B1", "B2"], &[("sec", 1)], &[]);

        let out = merge(&[
            with_links(&a, "file:///d/a.html", &DEFAULT_LINKS),
            with_links(&b, "file:///d/b.html", &DEFAULT_LINKS),
        ])
        .expect("should merge")
        .pdf;
        assert_eq!(
            links_on(&out, 1),
            [
                Followed::Page(3),
                Followed::Page(2),
                Followed::Page(2),
                Followed::Uri("file:///d/elsewhere.html".into()),
                Followed::Uri("https://example.com/".into()),
            ]
        );
    }

    /// Each option drops one kind and only that kind, and a document alone
    /// keeps the rest of its catalog while it is edited.
    #[test]
    fn disabling_a_kind_of_link_drops_only_that_kind() {
        let pdf = linked(
            &["P1", "P2"],
            &[("t", 1)],
            &[
                LinkSpec::Named("t"),
                LinkSpec::Uri("https://example.com/"),
                LinkSpec::Explicit(1),
            ],
        );
        let no_internal = LinkSettings {
            internal: false,
            ..DEFAULT_LINKS
        };
        let out = merge(&[with_links(&pdf, "file:///d/p.html", &no_internal)])
            .expect("alone")
            .pdf;
        assert_eq!(
            links_on(&out, 1),
            [Followed::Uri("https://example.com/".into())]
        );
        let document = Document::load_mem(&out).expect("should parse");
        assert!(
            document.catalog().unwrap().has(b"Lang"),
            "a document alone keeps its catalog"
        );

        let no_external = LinkSettings {
            external: false,
            ..DEFAULT_LINKS
        };
        let out = merge(&[with_links(&pdf, "file:///d/p.html", &no_external)])
            .expect("alone")
            .pdf;
        // The named one is resolved on the way, as it would be in a merge.
        assert_eq!(links_on(&out, 1), [Followed::Page(2), Followed::Page(2)]);
    }

    /// The browser resolved every link; below the document's own directory it
    /// can be made relative again.
    #[test]
    fn keeping_relative_links_writes_them_relative_again() {
        let pdf = linked(
            &["P1"],
            &[],
            &[
                LinkSpec::Uri("file:///d/other.html#x"),
                LinkSpec::Uri("file:///d/sub/deep.html"),
                LinkSpec::Uri("file:///elsewhere/x.html"),
                LinkSpec::Uri("https://example.com/"),
            ],
        );
        let keep = LinkSettings {
            resolve_relative: false,
            ..DEFAULT_LINKS
        };
        let out = merge(&[with_links(&pdf, "file:///d/p.html", &keep)])
            .expect("alone")
            .pdf;
        assert_eq!(
            links_on(&out, 1),
            [
                Followed::Uri("other.html#x".into()),
                Followed::Uri("sub/deep.html".into()),
                Followed::Uri("file:///elsewhere/x.html".into()),
                Followed::Uri("https://example.com/".into()),
            ]
        );
    }

    #[test]
    fn a_document_alone_that_asks_nothing_of_its_links_is_untouched() {
        let pdf = linked(&["P1"], &[("t", 0)], &[LinkSpec::Named("t")]);
        assert_eq!(merge(&[part(&pdf)]).expect("alone").pdf, pdf);
    }

    #[test]
    fn a_relative_link_is_only_one_below_the_documents_directory() {
        assert_eq!(
            relative_to("file:///d/x/y.html", "file:///d/a.html").as_deref(),
            Some("x/y.html")
        );
        assert_eq!(relative_to("file:///d/", "file:///d/a.html"), None);
        assert_eq!(relative_to("file:///e/y.html", "file:///d/a.html"), None);
        assert_eq!(
            relative_to("https://h/p/q.html#f", "https://h/p/index.html").as_deref(),
            Some("q.html#f")
        );
    }

    // --- stamping ---------------------------------------------------------------

    /// A document whose pages each draw one word with a font named `F1`, so a
    /// sheet stamped onto it has a resource name to collide with.
    fn worded(words: &[&str]) -> Vec<u8> {
        let mut document = Document::load_mem(&pages(words, None)).expect("should parse");
        let page_ids: Vec<ObjectId> = document.page_iter().collect();
        let font = document.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        for (page, word) in page_ids.iter().zip(words) {
            let contents = document.add_object(Stream::new(
                dictionary! {},
                format!("BT /F1 12 Tf 20 20 Td ({word}) Tj ET").into_bytes(),
            ));
            let dictionary = document.get_dictionary_mut(*page).expect("a page");
            dictionary.set("Contents", Object::Reference(contents));
            dictionary.set(
                "Resources",
                Object::Dictionary(dictionary! {
                    "Font" => dictionary! { "F1" => font },
                }),
            );
        }
        let mut out = Vec::new();
        document.save_to(&mut out).expect("should save");
        out
    }

    /// The font dictionary of a 1-based page, following references on the way.
    fn fonts_of(document: &Document, page: u32) -> Dictionary {
        let id = document.get_pages()[&page];
        let resolve = |object: &Object| document.dereference(object).ok().map(|(_, o)| o.clone());
        document
            .get_dictionary(id)
            .ok()
            .and_then(|page| page.get(b"Resources").ok())
            .and_then(resolve)
            .and_then(|resources| {
                resources
                    .as_dict()
                    .ok()
                    .and_then(|r| r.get(b"Font").ok())
                    .and_then(resolve)
            })
            .and_then(|fonts| fonts.as_dict().cloned().ok())
            .expect("a font dictionary")
    }

    /// Every word drawn on a 1-based page, in order, with the font each used.
    fn drawn(pdf: &[u8], page: usize) -> Vec<(String, String)> {
        let document = Document::load_mem(pdf).expect("should parse");
        let id = document.get_pages()[&(page as u32)];
        let content = Content::decode(&document.get_page_content(id)).expect("should decode");
        let mut font = String::new();
        let mut out = Vec::new();
        for operation in content.operations {
            match operation.operator.as_str() {
                "Tf" => {
                    font = String::from_utf8_lossy(operation.operands[0].as_name().unwrap())
                        .into_owned();
                }
                "Tj" => out.push((
                    String::from_utf8_lossy(operation.operands[0].as_str().unwrap()).into_owned(),
                    font.clone(),
                )),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn a_sheet_is_drawn_after_its_page_under_names_of_its_own() {
        let body = worded(&["one", "two"]);
        let bands = worded(&["FOOT1", "FOOT2"]);

        let out = stamp(&body, &bands).expect("should stamp");

        // Both words, the page's first, and the sheet's font renamed rather
        // than colliding with the page's own `F1`.
        assert_eq!(
            drawn(&out, 1),
            [
                ("one".to_string(), "F1".to_string()),
                ("FOOT1".to_string(), "Ov0F1".to_string())
            ]
        );
        assert_eq!(drawn(&out, 2)[1].0, "FOOT2");

        let document = Document::load_mem(&out).expect("should parse");
        assert_eq!(document.get_pages().len(), 2, "the sheets are not pages");
        let fonts = fonts_of(&document, 1);
        assert!(fonts.has(b"F1") && fonts.has(b"Ov0F1"), "{fonts:?}");
        // And the text is extractable, which is the whole reason for inlining.
        let text = document.extract_text(&[1]).expect("text");
        assert!(text.contains("one") && text.contains("FOOT1"), "{text}");
    }

    /// The page's own drawing is wrapped, so a transformation it left behind
    /// cannot move the band.
    #[test]
    fn the_pages_own_drawing_is_wrapped_before_the_sheet() {
        let out = stamp(&worded(&["one"]), &worded(&["FOOT"])).expect("should stamp");
        let document = Document::load_mem(&out).expect("should parse");
        let id = document.get_pages()[&1];
        let content = Content::decode(&document.get_page_content(id)).expect("should decode");
        let operators: Vec<&str> = content
            .operations
            .iter()
            .map(|o| o.operator.as_str())
            .collect();
        assert_eq!(operators.first(), Some(&"q"));
        let restore = operators.iter().position(|o| *o == "Q").expect("a Q");
        let band = operators
            .iter()
            .rposition(|o| *o == "Tj")
            .expect("the band's text");
        assert!(
            restore < band,
            "the band was drawn inside the page's own state: {operators:?}"
        );
    }

    #[test]
    fn a_band_document_with_the_wrong_number_of_pages_is_refused() {
        let error = stamp(&worded(&["one", "two"]), &worded(&["FOOT"])).expect_err("should refuse");
        assert!(
            error.to_string().contains("1 pages for a document of 2"),
            "{error}"
        );
    }

    /// Two pages sharing one resource dictionary must not gain each other's
    /// bands: the dictionary is copied per page before it is added to.
    #[test]
    fn pages_sharing_resources_are_given_their_own() {
        let mut document = Document::load_mem(&worded(&["one", "two"])).expect("should parse");
        let page_ids: Vec<ObjectId> = document.page_iter().collect();
        let shared = document
            .get_dictionary(page_ids[0])
            .unwrap()
            .get(b"Resources")
            .unwrap()
            .clone();
        let shared = document.add_object(shared);
        for page in &page_ids {
            document
                .get_dictionary_mut(*page)
                .unwrap()
                .set("Resources", Object::Reference(shared));
        }
        let mut body = Vec::new();
        document.save_to(&mut body).expect("should save");

        let out = stamp(&body, &worded(&["FOOT1", "FOOT2"])).expect("should stamp");
        let document = Document::load_mem(&out).expect("should parse");
        for (page, own) in [(1u32, b"Ov0F1".as_slice()), (2, b"Ov1F1".as_slice())] {
            let fonts = fonts_of(&document, page);
            assert!(
                fonts.has(own),
                "page {page} lacks its own band font: {fonts:?}"
            );
            assert_eq!(
                fonts.len(),
                2,
                "page {page} gained another page's band: {fonts:?}"
            );
        }
    }
}
