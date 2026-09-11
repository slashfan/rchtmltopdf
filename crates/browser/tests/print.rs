//! Printing, against a real browser.

mod support;

use rchtmltopdf_browser::render::Progress;
use rchtmltopdf_core::Orientation;
use rchtmltopdf_core::page_size;
use rchtmltopdf_core::settings::{LoadSettings, Margins, MediaType, PageSetup, WebSettings};
use rchtmltopdf_core::units::Length;
use support::{TestServer, launch, one_at_a_time};

/// The media box of the first page, in points, read straight out of the bytes.
///
/// Crude on purpose. Proper inspection arrives with the PDF crate; this only has
/// to tell whether the paper came out the size that was asked for.
fn first_media_box(pdf: &[u8]) -> Option<(f64, f64)> {
    let text = String::from_utf8_lossy(pdf);
    let at = text.find("/MediaBox")?;
    let open = text[at..].find('[')? + at;
    let close = text[open..].find(']')? + open;
    let numbers: Vec<f64> = text[open + 1..close]
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    match numbers.as_slice() {
        [x0, y0, x1, y1] => Some((x1 - x0, y1 - y0)),
        _ => None,
    }
}

/// How many times a PDF object type is named in the file.
///
/// Crude, like `first_media_box`, and for the same reason: proper inspection
/// arrives with the PDF crate. It is enough to tell whether a kind of object is
/// present at all.
fn objects_named(kind: &str, pdf: &[u8]) -> usize {
    String::from_utf8_lossy(pdf).matches(kind).count()
}

fn approx(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: {actual} is not within {tolerance} of {expected}"
    );
}

async fn print(page_setup: PageSetup, web: WebSettings, path: &str) -> Option<Vec<u8>> {
    let browser = launch().await?;
    let server = TestServer::start().await;
    let page = browser.new_page().await.unwrap();
    page.prepare(&web).await.unwrap();
    page.load(
        &server.url(path),
        &LoadSettings::default(),
        &Progress::new(),
    )
    .await
    .unwrap();
    let pdf = page.print_to_pdf(&page_setup, &web).await.unwrap();
    browser.close().await.unwrap();
    Some(pdf)
}

#[tokio::test]
async fn a4_portrait_comes_out_a4_portrait() {
    let _turn = one_at_a_time().await;
    let Some(pdf) = print(PageSetup::default(), WebSettings::default(), "/plain").await else {
        return;
    };

    assert!(pdf.starts_with(b"%PDF-"), "not a PDF");
    assert!(pdf.len() > 500, "suspiciously small: {} bytes", pdf.len());

    let (width, height) = first_media_box(&pdf).expect("a media box");
    // A4 is 595.28 by 841.89 points. Chromium rounds, so allow a little.
    approx(width, 595.28, 1.5, "A4 width");
    approx(height, 841.89, 1.5, "A4 height");
}

/// Orientation is resolved into the paper before the call, so this proves the
/// swap happens exactly once rather than not at all or twice.
#[tokio::test]
async fn landscape_swaps_the_paper_exactly_once() {
    let _turn = one_at_a_time().await;
    let setup = PageSetup {
        orientation: Orientation::Landscape,
        ..PageSetup::default()
    };
    let Some(pdf) = print(setup, WebSettings::default(), "/plain").await else {
        return;
    };

    let (width, height) = first_media_box(&pdf).expect("a media box");
    approx(width, 841.89, 1.5, "landscape width");
    approx(height, 595.28, 1.5, "landscape height");
    assert!(width > height, "landscape should be wider than it is tall");
}

#[tokio::test]
async fn other_paper_sizes_come_out_right() {
    let _turn = one_at_a_time().await;
    let setup = PageSetup {
        size: page_size::lookup("Letter").unwrap(),
        ..PageSetup::default()
    };
    let Some(pdf) = print(setup, WebSettings::default(), "/plain").await else {
        return;
    };

    let (width, height) = first_media_box(&pdf).expect("a media box");
    // 8.5 by 11 inches, at 72 points to the inch.
    approx(width, 612.0, 1.5, "Letter width");
    approx(height, 792.0, 1.5, "Letter height");
}

#[tokio::test]
async fn an_explicit_page_size_is_honoured() {
    let _turn = one_at_a_time().await;
    let setup = PageSetup {
        size: rchtmltopdf_core::PageDimensions::mm(100.0, 150.0),
        margins: Margins::uniform(Length::mm(0.0)),
        ..PageSetup::default()
    };
    let Some(pdf) = print(setup, WebSettings::default(), "/plain").await else {
        return;
    };

    let (width, height) = first_media_box(&pdf).expect("a media box");
    approx(width, 100.0 / 25.4 * 72.0, 1.5, "custom width");
    approx(height, 150.0 / 25.4 * 72.0, 1.5, "custom height");
}

/// The D03 trap: Chromium prints with print stylesheets unless told otherwise,
/// and wkhtmltopdf renders with screen ones. A document whose two stylesheets
/// differ must follow the screen one by default.
#[tokio::test]
async fn screen_stylesheets_win_by_default_and_print_media_flips_it() {
    let _turn = one_at_a_time().await;
    let Some(browser) = launch().await else {
        return;
    };
    let server = TestServer::start().await;

    for (media, expected) in [(MediaType::Screen, "screen"), (MediaType::Print, "print")] {
        let web = WebSettings {
            media_type: media,
            ..WebSettings::default()
        };
        let page = browser.new_page().await.unwrap();
        page.prepare(&web).await.unwrap();
        page.load(
            &server.url("/media"),
            &LoadSettings::default(),
            &Progress::new(),
        )
        .await
        .unwrap();

        let seen = page
            .session()
            .send(
                "Runtime.evaluate",
                serde_json::json!({
                    "expression": "getComputedStyle(document.body).getPropertyValue('--which').trim()",
                    "returnByValue": true,
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            seen["result"]["value"], expected,
            "the wrong stylesheet applied for {media:?}"
        );
    }

    browser.close().await.unwrap();
}

#[tokio::test]
async fn backgrounds_can_be_turned_off() {
    let _turn = one_at_a_time().await;
    let with = print(PageSetup::default(), WebSettings::default(), "/painted").await;
    let Some(with) = with else { return };

    let without = print(
        PageSetup::default(),
        WebSettings {
            background: false,
            ..WebSettings::default()
        },
        "/painted",
    )
    .await
    .unwrap();

    assert!(with.starts_with(b"%PDF-") && without.starts_with(b"%PDF-"));

    // Structural, not a size comparison. The fixture's background is a repeating
    // gradient, which Chromium writes as shading and pattern objects. Turning
    // backgrounds off removes them outright, so their presence is the behaviour
    // rather than a proxy for it. Comparing file sizes would be a compression
    // proxy, and the two happen to sit within a factor of two of each other, so
    // a threshold on that would be guesswork.
    assert!(
        objects_named("/Shading", &with) > 0,
        "the painted background should produce shading objects"
    );
    assert_eq!(
        objects_named("/Shading", &without),
        0,
        "backgrounds off should leave no shading behind"
    );
    assert!(objects_named("/Pattern", &with) > 0);
    assert_eq!(objects_named("/Pattern", &without), 0);
}

/// Writes a PDF out so it can be checked by something other than this test
/// suite. Ignored by default; run with `--ignored` when you want to look at one.
#[tokio::test]
#[ignore]
async fn write_a_sample_pdf() {
    let _turn = one_at_a_time().await;
    let Some(pdf) = print(PageSetup::default(), WebSettings::default(), "/plain").await else {
        return;
    };
    let path = std::env::temp_dir().join("rchtmltopdf-sample.pdf");
    std::fs::write(&path, &pdf).unwrap();
    println!("wrote {} ({} bytes)", path.display(), pdf.len());
}
