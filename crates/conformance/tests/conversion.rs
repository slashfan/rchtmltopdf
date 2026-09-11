//! The harness, proven end to end against a real browser.
//!
//! One conversion and one negative case: enough to show that the binary is
//! found, that a browser is resolved the way the product resolves it, and that
//! what lands on disk is a PDF. The full matrix — every input and output shape,
//! page sizes, landscape, margins, extracted text and a timeout that leaks no
//! process — is #17, and it builds on exactly this.

use rchtmltopdf_conformance::binary::{Run, is_pdf};
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::require_chromium;

#[test]
fn a_local_file_becomes_a_pdf_on_disk() {
    let Some(browser) = require_chromium() else {
        return;
    };
    eprintln!("using {} from {}", browser.path.display(), browser.origin);

    let scratch = Scratch::new("local-file");
    let page = fixture::write(scratch.path(), "page.html", "<h1>hello</h1>");
    let output = scratch.join("out.pdf");

    Run::new()
        .arg(page.display().to_string())
        .arg(output.display().to_string())
        .output()
        .succeeded();

    let bytes = std::fs::read(&output).expect("a converted document should be on disk");
    assert!(
        is_pdf(&bytes),
        "not a PDF, first bytes were {:?}",
        &bytes[..bytes.len().min(16)]
    );
}

/// The pair that matters: no PDF **and** a non-zero exit with something on
/// stderr (D14). A conversion that fails and leaves a plausible-looking file
/// behind is worse than one that fails loudly, because a billing pipeline will
/// happily post it.
#[test]
fn a_document_that_cannot_be_read_writes_no_pdf() {
    let Some(_browser) = require_chromium() else {
        return;
    };

    let scratch = Scratch::new("missing-input");
    let output = scratch.join("out.pdf");

    Run::new()
        .arg(scratch.join("no-such-page.html").display().to_string())
        .arg(output.display().to_string())
        .output()
        .failed();

    assert!(
        !output.exists(),
        "a failed conversion must not leave a file behind"
    );
}
