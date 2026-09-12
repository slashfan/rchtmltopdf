//! Links, and what the command line does with them (#41).
//!
//! Chromium writes a link annotation for every `<a href>` it prints: an anchor
//! in the document becomes a *named* destination, resolved through a table in
//! the catalog, and everything else a URI, resolved absolute against the
//! document's URL. Three things follow that are asserted here: the named
//! destination has to survive a merge, which drops the catalog it lived in; a
//! link to another document of the same conversion is a link into the file,
//! as wkhtmltopdf made it; and the options drop one kind or the other.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::{Link, Pdf};
use rchtmltopdf_conformance::require_chromium;

/// Three links on page one, an anchor on page two.
const LINKS: &str = "<p><a href=\"#target\">to the target</a></p>\
                     <p><a href=\"https://example.com/x?y=1\">out</a></p>\
                     <p><a href=\"b.html#sec\">to the other document</a></p>\
                     <div style=\"page-break-before:always\"></div>\
                     <p id=\"target\">the target</p>";
const OTHER: &str = "<p>before</p><p id=\"sec\">the section</p>";

fn internal(page: usize) -> Link {
    Link::Internal { page }
}

fn external(uri: &str) -> Link {
    Link::External { uri: uri.into() }
}

fn fixtures(scratch: &Scratch) -> (String, String) {
    let a = fixture::write(scratch.path(), "a.html", LINKS);
    let b = fixture::write(scratch.path(), "b.html", OTHER);
    (a.display().to_string(), b.display().to_string())
}

fn convert(args: &[&str]) -> Pdf {
    let outcome = Run::new().args(args.iter().copied()).arg("-").output();
    outcome.succeeded();
    Pdf::from_bytes(&outcome.stdout)
}

/// Alone, the anchor resolves through the catalog and the rest are URIs. The
/// relative link is absolute, as the browser resolved it.
#[test]
fn every_link_is_clickable_by_default() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("links-default");
    let (a, _) = fixtures(&scratch);

    let pdf = convert(&[&a]);
    let links = pdf.links(1);
    assert_eq!(links.len(), 3, "{links:?}");
    assert_eq!(links[0], internal(2));
    assert_eq!(links[1], external("https://example.com/x?y=1"));
    let Link::External { uri } = &links[2] else {
        panic!(
            "the other document is not in this conversion: {:?}",
            links[2]
        );
    };
    assert!(
        uri.starts_with("file://") && uri.ends_with("/b.html#sec"),
        "{uri}"
    );
}

/// **The merge drops the catalog the anchor resolved through**, so the anchor
/// has to be resolved first. And the other document *is* in this conversion
/// now, so the link to it is a link into the file.
#[test]
fn an_anchor_and_a_link_between_documents_survive_a_merge() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("links-merge");
    let (a, b) = fixtures(&scratch);

    // b first, so a's pages move: the anchor is on page 3 now.
    let pdf = convert(&[&b, &a]);
    assert_eq!(pdf.page_count(), 3);
    assert_eq!(
        pdf.links(2),
        [
            internal(3),
            external("https://example.com/x?y=1"),
            internal(1),
        ]
    );

    // a first: the section is in b, which is page 3.
    let pdf = convert(&[&a, &b]);
    assert_eq!(pdf.links(1)[2], internal(3));
}

#[test]
fn each_kind_of_link_can_be_switched_off() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("links-off");
    let (a, b) = fixtures(&scratch);

    // Internal covers the anchor and the link into the other document.
    let pdf = convert(&["--disable-internal-links", &a, &b]);
    assert_eq!(pdf.links(1), [external("https://example.com/x?y=1")]);

    let pdf = convert(&["--disable-external-links", &a, &b]);
    assert_eq!(pdf.links(1), [internal(2), internal(3)]);

    // Per object: only the document it was written after.
    let pdf = convert(&[&a, "--disable-external-links", &b]);
    assert_eq!(pdf.links(1).len(), 2);
    let pdf = convert(&[&a, &b, "--disable-external-links"]);
    assert_eq!(pdf.links(1).len(), 3);
}

/// The browser resolved the relative link; asked to, the conversion writes it
/// back relative to the document's directory.
#[test]
fn keep_relative_links_writes_them_relative_again() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("links-relative");
    let (a, _) = fixtures(&scratch);

    let pdf = convert(&["--keep-relative-links", &a]);
    assert_eq!(pdf.links(1)[2], external("b.html#sec"));
    // And an absolute one is left alone.
    assert_eq!(pdf.links(1)[1], external("https://example.com/x?y=1"));
}
