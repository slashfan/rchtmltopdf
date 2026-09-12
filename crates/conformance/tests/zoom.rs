//! `--zoom`, and the options that used to do the same job and no longer can.
//!
//! # Why this one is measured and not counted
//!
//! Every other option in the suite is asserted through the page count, which is
//! a coarse signal: it says something moved. `--zoom` is a scale factor, so the
//! honest assertion is that a block painted at a known size comes out that many
//! times bigger. A page count would pass for a zoom of 1.05 and for one of 3.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::{POINTS_PER_INCH, Pdf, TOLERANCE};
use rchtmltopdf_conformance::require_chromium;

/// A block with a size in millimetres, painted so `inspect` can find it.
const BLOCK: &str = "<div style=\"width:50mm;height:50mm;background:#333\"></div>";

fn convert(scratch: &Scratch, name: &str, options: &[&str]) -> Pdf {
    let page = fixture::write(scratch.path(), &format!("{name}.html"), BLOCK);
    let outcome = Run::new()
        .args(options.iter().copied())
        .arg(page.display().to_string())
        .arg("-")
        .output();
    outcome.succeeded();
    Pdf::from_bytes(&outcome.stdout)
}

/// The marker block, which is **not** the largest thing painted: the page's own
/// background covers the whole content area and is bigger than anything inside
/// it. Everything at least a tenth narrower than the paper is a candidate.
fn marker(pdf: &Pdf) -> rchtmltopdf_conformance::inspect::Rect {
    let paper = pdf.media_box(1);
    let mut candidates: Vec<_> = pdf
        .painted_boxes(1)
        .into_iter()
        .filter(|painted| painted.width() < paper.width() * 0.9)
        .collect();
    candidates.sort_by(|a, b| {
        (b.width() * b.height())
            .partial_cmp(&(a.width() * a.height()))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    *candidates
        .first()
        .unwrap_or_else(|| panic!("the block was not painted: {}", pdf.describe()))
}

fn mm(value: f64) -> f64 {
    value / 25.4 * POINTS_PER_INCH
}

/// The default is 1.0, so a document that names no zoom prints at its own size.
/// Everything below is calibrated against this.
#[test]
fn a_document_with_no_zoom_prints_at_its_own_size() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("zoom-default");
    let painted = marker(&convert(&scratch, "plain", &[]));

    assert!(
        (painted.width() - mm(50.0)).abs() <= TOLERANCE,
        "wanted {:.2} pt, measured {:.2} pt",
        mm(50.0),
        painted.width()
    );
}

/// The assertion the option is actually making: 1.3 means one and three tenths,
/// not "bigger".
#[test]
fn zoom_scales_the_content_by_the_factor_it_was_given() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("zoom-scaled");

    for factor in [1.3, 0.5] {
        let painted = marker(&convert(
            &scratch,
            "scaled",
            &["--zoom", &factor.to_string()],
        ));
        let wanted = mm(50.0) * factor;
        assert!(
            (painted.width() - wanted).abs() <= TOLERANCE,
            "at --zoom {factor}: wanted {wanted:.2} pt, measured {:.2} pt",
            painted.width()
        );
    }
}

/// Zoom scales the content and **not** the paper. A document zoomed to fit
/// different paper is a different request, and `--page-size` is the one that
/// answers it.
#[test]
fn zoom_leaves_the_paper_alone() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("zoom-paper");

    let plain = convert(&scratch, "paper-plain", &[]).media_box(1);
    let zoomed = convert(&scratch, "paper-zoomed", &["--zoom", "1.3"]).media_box(1);
    assert!(
        zoomed.is_about(plain.width(), plain.height()),
        "the paper moved: {:.2} x {:.2} became {:.2} x {:.2}",
        plain.width(),
        plain.height(),
        zoomed.width(),
        zoomed.height()
    );
}

/// The four options that used to do this job in wkhtmltopdf and have no
/// equivalent here (D08). Each is accepted so a wrapper emitting it keeps
/// working, warns so nobody spends an afternoon wondering why it did nothing,
/// and points at the guide that says what to do instead.
#[test]
fn the_shrinking_options_are_accepted_warned_about_and_ignored() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("zoom-shrink");
    let reference = marker(&convert(&scratch, "shrink-plain", &[]));

    for option in [
        "--disable-smart-shrinking",
        "--enable-smart-shrinking",
        "--dpi 300",
        "--image-dpi 300",
    ] {
        let written: Vec<&str> = option.split(' ').collect();
        let page = fixture::write(scratch.path(), "shrink.html", BLOCK);
        let outcome = Run::new()
            .args(written.iter().copied())
            .arg(page.display().to_string())
            .arg("-")
            .output();
        outcome.succeeded();

        assert!(
            outcome.stderr.contains(written[0]),
            "{option} should warn:\n{}",
            outcome.stderr
        );
        assert!(
            outcome.stderr.contains("migration guide"),
            "{option} should point at the guide:\n{}",
            outcome.stderr
        );

        let painted = marker(&Pdf::from_bytes(&outcome.stdout));
        assert!(
            (painted.width() - reference.width()).abs() <= TOLERANCE,
            "{option} changed the rendering: {:.2} pt became {:.2} pt",
            reference.width(),
            painted.width()
        );
    }
}
