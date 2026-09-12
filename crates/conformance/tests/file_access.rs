//! What a document is allowed to read off the disk it is rendered on (D10).
//!
//! # Why this needs a real browser
//!
//! Chromium's own default lets a `file://` page read `file://` subresources, and
//! there is no flag that turns that into wkhtmltopdf's rule. The rule is applied
//! per request, and a unit test of the policy proves only that the right answer
//! is computed — not that anything asks. These ask.
//!
//! # The signal
//!
//! A stylesheet that, if it loads, makes the document two pages instead of one.
//! Page count is structural, comes out of the file rather than off a screen, and
//! does not depend on the fonts installed. Comparing file sizes, or looking for
//! text, would not distinguish a stylesheet that was refused from one that was
//! empty.

use rchtmltopdf_conformance::binary::{Outcome, Run};
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::{require_chromium, server};
use std::path::{Path, PathBuf};

/// Taller than A4's content area, so the paragraph after it cannot share a page.
const STYLESHEET: &str = "#pad { height: 400mm; }";

struct Documents {
    /// The document to convert, inside `pages/`.
    page: PathBuf,
    /// The directory holding the stylesheet, which is *not* the document's own.
    assets: PathBuf,
    /// The stylesheet's URL, as the document asks for it.
    stylesheet: String,
}

fn build(scratch: &Scratch) -> Documents {
    let pages = scratch.join("pages");
    let assets = scratch.join("assets");
    std::fs::create_dir_all(&pages).expect("a scratch directory should be writable");
    std::fs::create_dir_all(&assets).expect("a scratch directory should be writable");

    let stylesheet_path = assets.join("tall.css");
    std::fs::write(&stylesheet_path, STYLESHEET).expect("the stylesheet should be writable");
    let stylesheet = file_url(&stylesheet_path);

    // The stylesheet sits beside the document rather than in it, which is the
    // whole question: a document may always read itself.
    let page = pages.join("doc.html");
    std::fs::write(
        &page,
        fixture::document(&format!(
            "<link rel=\"stylesheet\" href=\"{stylesheet}\"><div id=\"pad\"></div><p>after</p>"
        )),
    )
    .expect("the document should be writable");

    Documents {
        page,
        assets,
        stylesheet,
    }
}

fn file_url(path: &Path) -> String {
    format!(
        "file://{}",
        path.canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
    )
}

/// Convert a local document and read back what came out.
fn convert(documents: &Documents, options: &[&str]) -> (Pdf, Outcome) {
    let outcome = Run::new()
        .args(options.iter().copied())
        .arg(documents.page.display().to_string())
        // Straight to stdout, so nothing on disk can be mistaken for the result
        // of an earlier run.
        .arg("-")
        .output();
    outcome.succeeded();
    (Pdf::from_bytes(&outcome.stdout), outcome)
}

/// The default, and the reason the default is what it is.
#[test]
fn a_local_document_cannot_read_the_file_beside_it_by_default() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("file-access-default");
    let documents = build(&scratch);

    let (pdf, outcome) = convert(&documents, &[]);
    assert_eq!(
        pdf.page_count(),
        1,
        "the stylesheet should not have loaded: {}",
        pdf.describe()
    );

    // Silently dropping it would leave a document that renders and is wrong,
    // which is the hardest kind of failure to find.
    assert!(
        outcome.stderr.contains("tall.css"),
        "the refusal should name the file:\n{}",
        outcome.stderr
    );
    assert!(
        outcome.stderr.contains("--enable-local-file-access"),
        "the refusal should say what to do about it:\n{}",
        outcome.stderr
    );
}

#[test]
fn the_flag_lets_it_through() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("file-access-enabled");
    let documents = build(&scratch);

    let (pdf, _) = convert(&documents, &["--enable-local-file-access"]);
    assert_eq!(
        pdf.page_count(),
        2,
        "the stylesheet should have loaded: {}",
        pdf.describe()
    );
}

/// `--allow` opens one directory, and only the one named.
#[test]
fn allow_opens_the_directory_it_names_and_no_other() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("file-access-allow");
    let documents = build(&scratch);

    let allowed = documents.assets.display().to_string();
    let (opened, _) = convert(&documents, &["--allow", &allowed]);
    assert_eq!(opened.page_count(), 2, "{}", opened.describe());

    // The document's own directory is not the stylesheet's, so naming it changes
    // nothing. A whitelist that let a sibling through would not be one.
    let elsewhere = documents
        .page
        .parent()
        .expect("the document is in a directory")
        .display()
        .to_string();
    let (refused, _) = convert(&documents, &["--allow", &elsewhere]);
    assert_eq!(refused.page_count(), 1, "{}", refused.describe());
}

/// The case the whole policy exists for. An invoice template fetched over http
/// must not be able to read this machine's disk, and **no option relaxes it** —
/// so the flag that opens everything for a local document is passed here, and it
/// still may not.
///
/// This asserts the outcome and not our own message, because there is no message
/// to assert. Chromium's renderer refuses the subresource before a request is
/// made, so it never pauses and the policy is never asked. That is measured
/// rather than assumed, and it is why the rule is kept anyway: the guarantee is
/// ours to make rather than the browser's to keep.
#[test]
fn a_remote_document_may_not_read_a_local_file_even_with_the_flag() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("file-access-remote");
    let documents = build(&scratch);
    let server = server::Server::start();

    let url = server.url(&format!(
        "{}?href={}",
        server::REFERENCING,
        documents.stylesheet
    ));
    let outcome = Run::new()
        .arg("--enable-local-file-access")
        .arg(url)
        .arg("-")
        .output();
    outcome.succeeded();

    let pdf = Pdf::from_bytes(&outcome.stdout);
    assert_eq!(
        pdf.page_count(),
        1,
        "a page from the network read a local file: {}",
        pdf.describe()
    );
}

/// The sanity check under all of the above: with no stylesheet involved at all,
/// the document is one page. Without this, every assertion of "one page" above
/// would also pass if the fixture simply never worked.
#[test]
fn the_second_page_comes_from_the_stylesheet_and_nothing_else() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("file-access-control");

    let page = fixture::write(
        scratch.path(),
        "plain.html",
        "<div id=\"pad\"></div><p>after</p>",
    );
    let outcome = Run::new().arg(page.display().to_string()).arg("-").output();
    outcome.succeeded();
    assert_eq!(Pdf::from_bytes(&outcome.stdout).page_count(), 1);

    // And with the rule out of the way it is two, so the fixture measures the
    // rule rather than the browser's mood.
    let scratch = Scratch::new("file-access-control-open");
    let documents = build(&scratch);
    let (pdf, _) = convert(&documents, &["--enable-local-file-access"]);
    assert_eq!(pdf.page_count(), 2, "{}", pdf.describe());
}
