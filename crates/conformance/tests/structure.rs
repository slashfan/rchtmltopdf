//! The geometry of what came out, measured against the model in `core`.
//!
//! These are the assertions D15 asks for: page count, paper size, orientation
//! and margins, read out of the file rather than looked at. Nothing here
//! compares an image, and nothing here depends on the fonts installed — every
//! fixture brings its own (see `fixture`), and the page counts are driven by
//! explicit page breaks rather than by where text happens to wrap.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::{POINTS_PER_INCH, Pdf, Rect, TOLERANCE};
use rchtmltopdf_conformance::require_chromium;
use rchtmltopdf_core::page_size;

/// Convert, and read the result back.
fn convert(scratch: &Scratch, name: &str, body: &str, options: &[&str]) -> Pdf {
    let page = fixture::write(scratch.path(), &format!("{name}.html"), body);
    let output = scratch.join(&format!("{name}.pdf"));

    Run::new()
        .args(options.iter().copied())
        .arg(page.display().to_string())
        .arg(output.display().to_string())
        .output()
        .succeeded();

    Pdf::from_bytes(&std::fs::read(&output).expect("a converted document should be on disk"))
}

/// What `core` says a named size is, in points.
fn expected(name: &str) -> (f64, f64) {
    let size = page_size::lookup(name).unwrap_or_else(|| panic!("{name} should be a known size"));
    (
        size.width.to_inches() * POINTS_PER_INCH,
        size.height.to_inches() * POINTS_PER_INCH,
    )
}

// --- paper -------------------------------------------------------------------

/// Cross-checks the rendered file against the table in `core`, so the two cannot
/// drift apart. A number typed into this test would only ever agree with itself.
#[test]
fn named_page_sizes_come_out_the_size_the_model_says() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("page-sizes");

    for name in ["A4", "A3", "Letter"] {
        let pdf = convert(&scratch, name, "<p>paper</p>", &["--page-size", name]);
        let (width, height) = expected(name);
        let box_ = pdf.media_box(1);
        assert!(
            box_.is_about(width, height),
            "{name}: wanted {width:.2} x {height:.2} pt, got {}",
            pdf.describe()
        );
    }
}

#[test]
fn landscape_swaps_the_media_box() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("landscape");

    let portrait = convert(&scratch, "portrait", "<p>x</p>", &["--page-size", "A4"]).media_box(1);
    let landscape = convert(
        &scratch,
        "landscape",
        "<p>x</p>",
        &["--page-size", "A4", "--orientation", "Landscape"],
    )
    .media_box(1);

    assert!(
        landscape.is_about(portrait.height(), portrait.width()),
        "landscape should be portrait turned over: {:.2} x {:.2} against {:.2} x {:.2}",
        landscape.width(),
        landscape.height(),
        portrait.width(),
        portrait.height()
    );
    // Stated separately, because "swapped" is also true of a square page, where
    // the assertion above would pass without orientation doing anything.
    assert!(
        landscape.width() > landscape.height(),
        "landscape should be wider than it is tall"
    );
}

#[test]
fn an_explicit_width_and_height_are_honoured() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("explicit-size");

    let pdf = convert(
        &scratch,
        "custom",
        "<p>x</p>",
        &["--page-width", "100mm", "--page-height", "150mm"],
    );
    let (width, height) = (
        100.0 / 25.4 * POINTS_PER_INCH,
        150.0 / 25.4 * POINTS_PER_INCH,
    );
    assert!(
        pdf.media_box(1).is_about(width, height),
        "wanted {width:.2} x {height:.2} pt, got {}",
        pdf.describe()
    );
}

/// A length is a length however it is written (D03). wkhtmltopdf takes a bare
/// number as millimetres, and Snappy applications write sizes every way there
/// is, so a unit that quietly loses precision on the way to the browser would
/// show up as a page a millimetre out and nothing else.
#[test]
fn every_spelling_of_a_size_produces_the_same_page() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("spellings");

    // A4, four ways.
    let spellings = [
        ("mm", "210mm", "297mm"),
        ("bare", "210", "297"),
        ("inches", "8.2677in", "11.6929in"),
        ("points", "595.28pt", "841.89pt"),
    ];

    let mut seen: Vec<(&str, Rect)> = Vec::new();
    for (label, width, height) in spellings {
        let pdf = convert(
            &scratch,
            label,
            "<p>x</p>",
            &["--page-width", width, "--page-height", height],
        );
        seen.push((label, pdf.media_box(1)));
    }

    let (first_label, first) = seen[0];
    for (label, box_) in &seen[1..] {
        assert!(
            box_.is_about(first.width(), first.height()),
            "{label} gave {:.2} x {:.2}, {first_label} gave {:.2} x {:.2}",
            box_.width(),
            box_.height(),
            first.width(),
            first.height()
        );
    }
}

// --- pages -------------------------------------------------------------------

/// Driven by explicit page breaks, not by text flow.
///
/// A count that came from wrapping would be a property of the font metrics as
/// much as of the renderer, and this suite goes to some trouble to keep those
/// out of its assertions.
#[test]
fn explicit_page_breaks_produce_the_pages_they_ask_for() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("page-breaks");

    for wanted in [1usize, 2, 3] {
        let body: String = (1..=wanted)
            .map(|n| {
                if n == wanted {
                    format!("<div>page {n}</div>")
                } else {
                    format!("<div style=\"break-after: page\">page {n}</div>")
                }
            })
            .collect();

        let pdf = convert(&scratch, &format!("breaks-{wanted}"), &body, &[]);
        assert_eq!(pdf.page_count(), wanted, "{}", pdf.describe());
    }
}

// --- margins -----------------------------------------------------------------

/// **A margin has no entry in a PDF.** It is an offset applied to content, and
/// nothing in the file records it, so it can only be measured indirectly.
///
/// The technique settled on here: the fixture paints a block that fills the
/// content area, and where that block lands is the margin. The alternative —
/// the bounding box of extracted text — depends on font metrics and on the line
/// box, which is exactly what this suite keeps out of its assertions.
#[test]
fn margins_put_the_content_where_it_was_asked_for() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("margins");

    let (top_mm, left_mm, right_mm) = (20.0, 15.0, 15.0);
    let pdf = convert(
        &scratch,
        "margins",
        // Fills the content area horizontally, with a height small enough that
        // it cannot spill onto a second page.
        "<div style=\"background:#000;width:100%;height:200pt\"></div>",
        &[
            "--page-size",
            "A4",
            "--margin-top",
            "20mm",
            "--margin-bottom",
            "20mm",
            "--margin-left",
            "15mm",
            "--margin-right",
            "15mm",
        ],
    );

    assert_eq!(pdf.page_count(), 1, "{}", pdf.describe());
    let paper = pdf.media_box(1);
    let painted = pdf.largest_painted_box(1);

    let mm = |value: f64| value / 25.4 * POINTS_PER_INCH;
    let close = |actual: f64, wanted: f64, what: &str| {
        assert!(
            (actual - wanted).abs() <= TOLERANCE,
            "{what}: wanted {wanted:.2} pt, measured {actual:.2} pt \
             (paper {:.2} x {:.2}, content at {:.2},{:.2} to {:.2},{:.2})",
            paper.width(),
            paper.height(),
            painted.left,
            painted.bottom,
            painted.right,
            painted.top
        );
    };

    close(painted.left, mm(left_mm), "left margin");
    close(paper.right - painted.right, mm(right_mm), "right margin");
    close(paper.top - painted.top, mm(top_mm), "top margin");
}
