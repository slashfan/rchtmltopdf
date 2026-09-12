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
use rchtmltopdf_conformance::{fixture, require_chromium, sandbox_unavailable};
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
