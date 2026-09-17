//! The four options that change how the page is set up before it is printed.
//!
//! Each is asserted through the page count, which is structural and comes out of
//! the file. Three of them are easy to *claim* and hard to see: a font floor, a
//! blocked image and an emulated window all change layout without changing
//! anything a PDF names, so each fixture is built so that the option moves
//! content onto a second page or does not.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::require_chromium;
use std::path::Path;

/// Convert a document already on disk, straight to standard output.
fn convert(page: &Path, options: &[&str]) -> Pdf {
    let outcome = Run::new()
        .args(options.iter().copied())
        .arg(page.display().to_string())
        .arg("-")
        .output();
    outcome.succeeded();
    Pdf::from_bytes(&outcome.stdout)
}

/// Text far below any sensible floor, so raising the floor has somewhere to go.
#[test]
fn minimum_font_size_raises_text_that_is_below_it() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("minimum-font-size");
    let page = fixture::write(
        scratch.path(),
        "tiny.html",
        &format!("<div style=\"font-size:4px\">{}</div>", "tiny ".repeat(400)),
    );

    let untouched = convert(&page, &[]);
    let raised = convert(&page, &["--minimum-font-size", "40"]);
    assert!(
        raised.page_count() > untouched.page_count(),
        "a floor of 40px should not fit in {} pages: {}",
        untouched.page_count(),
        raised.describe()
    );
}

/// An image with an intrinsic size, which takes up room if it loads and none if
/// it does not. `data:` rather than a file, so the local file policy is not
/// quietly what is being measured.
#[test]
fn no_images_stops_them_taking_up_room() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("no-images");

    let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='600' height='900'>\
               <rect width='600' height='900' fill='#333'/></svg>";
    let uri = format!(
        "data:image/svg+xml;base64,{}",
        rchtmltopdf_conformance::fixture::base64(svg.as_bytes())
    );
    let page = fixture::write(
        scratch.path(),
        "images.html",
        &format!("<img src=\"{uri}\"><img src=\"{uri}\"><img src=\"{uri}\">"),
    );

    let with = convert(&page, &[]);
    let without = convert(&page, &["--no-images"]);
    assert!(with.page_count() > 1, "the fixture should need the images");
    assert_eq!(
        without.page_count(),
        1,
        "the images should not have loaded: {}",
        without.describe()
    );
}

/// What `--viewport-size` actually moves, and what it does not.
///
/// **It does not decide the printed layout width**, and it does not decide which
/// media queries match either: a printed page is laid out at the paper's content
/// width, and `@media (min-width: …)` is evaluated against that whatever the
/// window is set to. Driving the same fixture with a media query at 1024 and at
/// 1280 produces the same document, which is why this test uses a script.
///
/// What it does move is the window a script reads, which is wkhtmltopdf's
/// behaviour and what a migrated document relies on. `docs/migration.md` carries
/// both halves.
#[test]
fn the_viewport_is_the_window_a_script_reads() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("viewport");
    let page = fixture::write(
        scratch.path(),
        "window.html",
        "<div id=\"pad\"></div><p>after</p>\
         <script>if (window.innerWidth >= 1200) \
         { document.getElementById('pad').style.height = '400mm'; }</script>",
    );

    // wkhtmltopdf's default window is 1024 wide, so the script does not fire.
    assert_eq!(convert(&page, &[]).page_count(), 1);
    assert_eq!(
        convert(&page, &["--viewport-size", "1024x768"]).page_count(),
        1
    );

    let wide = convert(&page, &["--viewport-size", "1280x1024"]);
    assert_eq!(
        wide.page_count(),
        2,
        "the script should have read the wider window: {}",
        wide.describe()
    );
}

/// A document that declares no charset, read two ways.
///
/// The bytes are `é` written as UTF-8. Read as UTF-8 they are one character;
/// read as Latin-1 they are two, so the same file is twice as long and takes
/// more pages. Both runs name an encoding, so this measures `--encoding` rather
/// than what Chromium guesses when nothing says.
#[test]
fn encoding_decides_how_a_document_that_says_nothing_is_read() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("encoding");

    let mut body = b"<div style=\"overflow-wrap:anywhere;font-size:12px\">".to_vec();
    for _ in 0..20_000 {
        body.extend_from_slice("é".as_bytes());
    }
    body.extend_from_slice(b"</div>");

    let page = scratch.join("undeclared.html");
    std::fs::write(&page, fixture::undeclared(&body)).expect("the fixture should be writable");

    let utf8 = convert(&page, &["--encoding", "UTF-8"]);
    let latin = convert(&page, &["--encoding", "ISO-8859-1"]);
    assert!(
        utf8.page_count() > 1,
        "the fixture should take several pages"
    );
    assert!(
        latin.page_count() > utf8.page_count(),
        "read as Latin-1 the document is twice as long: {} pages as UTF-8, {} as Latin-1",
        utf8.page_count(),
        latin.page_count()
    );
}

/// There is nowhere to put the charset on a document fetched over http, so say
/// so rather than accepting the option and doing nothing with it.
#[test]
fn encoding_says_it_does_not_apply_to_a_document_from_the_network() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = rchtmltopdf_conformance::server::Server::start();

    let outcome = Run::new()
        .arg("--encoding")
        .arg("ISO-8859-1")
        .arg(server.url(rchtmltopdf_conformance::server::PAGE))
        .arg("-")
        .output();
    outcome.succeeded();
    assert!(
        outcome.stderr.contains("--encoding"),
        "should say the option did not apply:\n{}",
        outcome.stderr
    );
}

/// **A document's own `@page` rule does not decide the margins** (D60).
///
/// wkhtmltopdf ignores `@page` altogether — measured on 0.12.6.1, a document
/// saying `margin: 0` still gets the margin from the command line, and one
/// saying `margin: 30mm` with no option still gets wkhtmltopdf's 10mm default.
/// Chromium honours the rule, so a print stylesheet carrying the commonest
/// line there is — `@page { margin: 0 }`, written precisely because it never
/// did anything under wkhtmltopdf — silently took the margins away, and
/// anything the bands draw then landed on the content instead of beside it.
///
/// The instrument is a block filling the content area: where it lands is the
/// margin, as everywhere else in this suite.
#[test]
fn a_documents_own_page_rule_does_not_decide_the_margins() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("page-rule-margins");
    let block = "<div style=\"background:#000;width:100%;height:120pt\"></div>";
    let plain = fixture::write(scratch.path(), "plain.html", block);
    let declaring = fixture::write(
        scratch.path(),
        "declaring.html",
        &format!("<style>@page {{ margin: 0; }}</style>{block}"),
    );

    let options = ["--margin-top", "22mm", "--margin-left", "15mm"];
    let expected = convert(&plain, &options);
    let measured = convert(&declaring, &options);

    let top = |pdf: &Pdf| pdf.media_box(1).top - pdf.largest_painted_box(1).top;
    let left = |pdf: &Pdf| pdf.largest_painted_box(1).left;
    assert!(
        (top(&measured) - top(&expected)).abs() < 1.0,
        "top margin: {:.2} pt with the rule against {:.2} pt without it",
        top(&measured),
        top(&expected)
    );
    assert!(
        (left(&measured) - left(&expected)).abs() < 1.0,
        "left margin: {:.2} pt with the rule against {:.2} pt without it",
        left(&measured),
        left(&expected)
    );
}

/// The other half of the same rule: a document asking for another paper size
/// does not get it, and does not get its *layout* either (D60). The paper was
/// already protected by `preferCSSPageSize: false`; the boxes the content is
/// broken into were not, and a page laid out for A5 on A4 paper breaks in the
/// wrong places.
#[test]
fn a_documents_own_page_size_does_not_decide_the_layout() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("page-rule-size");
    let block = "<div style=\"background:#000;width:100%;height:120pt\"></div>";
    let plain = fixture::write(scratch.path(), "plain.html", block);
    let declaring = fixture::write(
        scratch.path(),
        "declaring.html",
        &format!("<style>@page {{ size: A5; }}</style>{block}"),
    );

    let expected = convert(&plain, &[]);
    let measured = convert(&declaring, &[]);

    assert!(
        measured.media_box(1).is_about(
            expected.media_box(1).width(),
            expected.media_box(1).height()
        ),
        "the paper should be the one asked for: {}",
        measured.describe()
    );
    let width = |pdf: &Pdf| pdf.largest_painted_box(1).width();
    assert!(
        (width(&measured) - width(&expected)).abs() < 1.0,
        "content width: {:.2} pt laid out with the rule against {:.2} pt without it",
        width(&measured),
        width(&expected)
    );
}
