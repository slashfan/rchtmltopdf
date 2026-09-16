//! The vendored font is the one the glyphs actually came from.
//!
//! Everything in `fixture` exists to make page count a function of the pinned
//! Chromium rather than of whatever fonts the machine happens to have. That is a
//! claim, and an unverified one is worth nothing: if the `@font-face` were ever
//! broken — a bad `data:` URI, a family name typo, a `format()` Chromium stopped
//! accepting — the page would quietly render in a system font and every page
//! count in the suite would move without a single test going red.
//!
//! So this asks Chromium which fonts it used, rather than which ones it was
//! offered. `CSS.getPlatformFontsForNode` reports the faces that produced the
//! glyphs on screen, which is the only answer that settles it.
//!
//! `document.fonts.check()` was tried first and taken back out. CSS font loading
//! is lazy, so a declared face that nothing draws with is simply never loaded and
//! `check` returns false — it answers the same question as the platform fonts, or
//! a weaker version of it, and an assertion that cannot fail on its own is not
//! worth the lines.

use rchtmltopdf_browser::render::Progress;
use rchtmltopdf_browser::{Browser, LaunchOptions};
use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::inspect::Pdf;
use rchtmltopdf_conformance::{fixture, require_chromium, sandbox_unavailable, server};
use rchtmltopdf_core::settings::LoadSettings;
use serde_json::json;

/// The name inside the vendored file, which is not the family the fixture asks
/// for. The fixture asks for [`fixture::FAMILY`], a name no system font answers
/// to; what comes back out is the font's own name, and that round trip is the
/// proof that the file was loaded rather than substituted.
const REAL_NAME: &str = "Noto Sans";

#[tokio::test]
async fn glyphs_come_from_the_vendored_font_and_not_a_system_one() {
    let Some(executable) = require_chromium() else {
        return;
    };

    let scratch = fixture::Scratch::new("fonts");
    let page_path = fixture::write(
        scratch.path(),
        "page.html",
        "<p id=sample>Conformance sample text</p>",
    );

    let browser = Browser::launch(
        &executable,
        &LaunchOptions {
            no_sandbox: sandbox_unavailable(),
            ..LaunchOptions::default()
        },
    )
    .await
    .expect("the browser should start");

    let page = browser.new_page().await.expect("a page should open");
    page.prepare(&rchtmltopdf_browser::plan::prepare(
        &rchtmltopdf_core::settings::ObjectSettings::page(rchtmltopdf_core::Input::Stdin),
        "about:blank",
    ))
    .await
    .expect("the page should be preparable");
    page.load(
        &format!("file://{}", page_path.display()),
        &LoadSettings::default(),
        &Progress::new(),
    )
    .await
    .expect("the fixture should load");

    let session = page.session();

    session
        .send("DOM.enable", json!({}))
        .await
        .expect("DOM should enable");
    session
        .send("CSS.enable", json!({}))
        .await
        .expect("CSS should enable");

    let document = session
        .send("DOM.getDocument", json!({}))
        .await
        .expect("the document should be readable");
    let root = document["root"]["nodeId"]
        .as_i64()
        .expect("a document has a root node");

    let node = session
        .send(
            "DOM.querySelector",
            json!({ "nodeId": root, "selector": "#sample" }),
        )
        .await
        .expect("the sample should be findable");
    let node_id = node["nodeId"].as_i64().expect("the sample has a node id");

    let used = session
        .send("CSS.getPlatformFontsForNode", json!({ "nodeId": node_id }))
        .await
        .expect("Chromium should report the fonts it used");

    let families: Vec<String> = used["fonts"]
        .as_array()
        .expect("a rendered node has fonts")
        .iter()
        .filter_map(|font| font["familyName"].as_str().map(str::to_owned))
        .collect();

    assert!(
        families.iter().any(|family| family.contains(REAL_NAME)),
        "the text was drawn with {families:?}, not with the vendored {REAL_NAME}. \
         Page counts measured this way are a property of this machine, not of the \
         pinned Chromium."
    );

    browser.close().await.expect("the browser should close");
}

/// **A local document uses a web font served from another origin** (D58).
///
/// Measured on wkhtmltopdf 0.12.6.1 in `debian:bookworm-slim`, a local file
/// whose `@font-face` points at a loopback server that sends no
/// `Access-Control-Allow-Origin`:
///
/// | Format | wkhtmltopdf | exit |
/// | --- | --- | --- |
/// | `woff` | draws with the served face | 0 |
/// | `truetype` | draws with the served face | 0 |
/// | `woff2` | falls back, the format is beyond its Qt | 0 |
///
/// Chromium fetches a font in CORS mode, and a document loaded from disk has
/// the null origin, so without help the face is refused, the text is drawn in
/// whatever the machine falls back to, and the conversion exits 1 over it.
/// Every application that lets Snappy write its HTML to a temporary file and
/// serves its own fonts over HTTP lands on exactly this.
///
/// The page is written by hand rather than through `fixture::document`,
/// because that one carries the same font inside it as a `data:` URI: the
/// served face has to be the only way to get those glyphs, or the assertion
/// passes without the fetch ever succeeding.
#[test]
fn a_local_document_uses_a_web_font_from_another_origin() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let server = server::Server::start();
    let scratch = fixture::Scratch::new("fonts-cross-origin");
    let path = scratch.path().join("page.html");
    std::fs::write(
        &path,
        format!(
            "<!doctype html>\n<html><head><meta charset=\"utf-8\"><style>\n\
             @font-face {{ font-family: \"Remote Sans\";\n  \
               src: url(\"{}\") format(\"woff2\"); font-display: block; }}\n\
             html, body {{ font-family: \"Remote Sans\"; }}\n\
             </style></head><body><p>Grumpy wizards make toxic brew.</p></body></html>\n",
            server.url(server::FONT)
        ),
    )
    .expect("a fixture should be writable");

    let outcome = Run::new().arg(path.display().to_string()).arg("-").output();
    outcome.succeeded();

    let pdf = Pdf::from_bytes(&outcome.stdout);
    let fonts = pdf.fonts(1);
    assert!(
        fonts.iter().any(|name| name.contains("NotoSans")),
        "the page was drawn with {fonts:?} rather than with the served face"
    );
}
