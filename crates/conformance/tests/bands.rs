//! Headers and footers, drawn into the margins as sheets stamped onto the pages
//! (D38).
//!
//! # What was measured to write these
//!
//! A band is anchored to the **paper edge** and reaches towards the content,
//! in a box at least as tall as the margin on its side; `marginTop` decides the
//! box, not where the band starts. Everything below follows from that, and
//! every number in `band.rs`'s module documentation came from running these.
//!
//! The rule is the instrument. Text position is not something the inspector
//! exposes, but `--header-line` paints a rectangle 0.75pt tall across the page,
//! and where *that* lands says where the band is.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::{POINTS_PER_INCH, Pdf, Rect};
use rchtmltopdf_conformance::require_chromium;
use std::path::Path;

/// A block filling the content area, so where it lands measures the margin.
const BODY: &str = "<div style=\"background:#000;width:100%;height:200pt\"></div>";

fn convert(page: &Path, options: &[&str]) -> Pdf {
    let outcome = Run::new()
        .args(options.iter().copied())
        .arg(page.display().to_string())
        .arg("-")
        .output();
    outcome.succeeded();
    Pdf::from_bytes(&outcome.stdout)
}

fn mm(value: f64) -> f64 {
    value / 25.4 * POINTS_PER_INCH
}

/// The content area: the widest painted box that is not the full page.
///
/// The banded page carries one extra box, the template's own container, which
/// spans the whole sheet. Everything else is the document.
fn content(pdf: &Pdf) -> Rect {
    let paper = pdf.media_box(1);
    let mut inside: Vec<Rect> = pdf
        .painted_boxes(1)
        .into_iter()
        .filter(|painted| painted.width() < paper.width() * 0.99)
        .collect();
    inside.sort_by(|a, b| b.height().partial_cmp(&a.height()).unwrap());
    *inside
        .first()
        .unwrap_or_else(|| panic!("nothing was painted: {}", pdf.describe()))
}

/// The rule a `--header-line` or `--footer-line` paints: full width, hairline.
fn rule(pdf: &Pdf) -> Rect {
    let paper = pdf.media_box(1);
    let mut rules: Vec<Rect> = pdf
        .painted_boxes(1)
        .into_iter()
        .filter(|painted| painted.height() < 2.0 && painted.width() > paper.width() * 0.9)
        .collect();
    rules.sort_by(|a, b| b.top.partial_cmp(&a.top).unwrap());
    *rules
        .first()
        .unwrap_or_else(|| panic!("no rule was painted: {}", pdf.describe()))
}

#[test]
fn the_three_cells_of_both_bands_are_drawn() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("bands-cells");
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let pdf = convert(
        &page,
        &[
            "--header-left",
            "HL",
            "--header-center",
            "HC",
            "--header-right",
            "HR",
            "--footer-left",
            "FL",
            "--footer-center",
            "FC",
            "--footer-right",
            "FR",
        ],
    );

    let text = pdf.text();
    for mark in ["HL", "HC", "HR", "FL", "FC", "FR"] {
        assert!(text.contains(mark), "{mark} missing from {text:?}");
    }
}

/// **The gotcha that costs an afternoon.** Turning `displayHeaderFooter` on with
/// only a header template leaves Chromium drawing its *own* footer, which is a
/// page number nobody asked for. The empty band has to be sent explicitly.
#[test]
fn asking_for_a_header_does_not_get_chromiums_footer() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("bands-only-header");
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let pdf = convert(&page, &["--header-center", "HEADERMARK"]);
    let text = pdf.text();
    assert!(text.contains("HEADERMARK"), "{text:?}");
    assert!(
        !text.contains('1'),
        "a page number appeared that nobody asked for: {text:?}"
    );
}

/// A band is drawn inside the margin and does not push the document down.
/// Adding a header to a migrated command line must not silently repaginate it.
#[test]
fn a_band_does_not_move_the_content() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("bands-content");
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let plain = content(&convert(&page, &[]));
    let banded = content(&convert(
        &page,
        &["--header-center", "H", "--footer-center", "F"],
    ));

    assert!(
        (plain.top - banded.top).abs() < 0.01 && (plain.bottom - banded.bottom).abs() < 0.01,
        "the content moved: {plain:?} became {banded:?}"
    );
}

/// Where the band actually is. The header's rule sits at the bottom of the top
/// margin, between the band and the content — and the default 10mm margin only
/// just fits a 12pt band, which is worth knowing before choosing a font size.
#[test]
fn the_header_rule_sits_between_the_band_and_the_content() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("bands-rule");
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let pdf = convert(&page, &["--header-center", "H", "--header-line"]);
    let paper = pdf.media_box(1);
    let rule = rule(&pdf);
    let content = content(&pdf);

    // Below the top of the paper and above the bottom of the content: the rule
    // is in the margin, not floating in the middle of the page.
    assert!(rule.top < paper.top, "the rule is off the top: {rule:?}");
    assert!(
        rule.top > content.bottom,
        "the rule is below the content: {rule:?}"
    );
    // A 12pt band is about 28.5pt tall and a 10mm margin is 28.3pt, so the two
    // land within a point of each other. That is the whole margin used up.
    assert!(
        (paper.top - rule.top - mm(10.0)).abs() < 2.0,
        "the band should fill the default margin: rule at {:.2}, margin is {:.2}",
        paper.top - rule.top,
        mm(10.0)
    );
}

/// Spacing is the one thing that can open a gap, because the band cannot: it is
/// anchored to the paper edge, so only the print margin decides where the
/// content starts.
#[test]
fn spacing_opens_a_gap_between_the_band_and_the_content() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("bands-spacing");
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let tight = convert(&page, &["--header-center", "H", "--header-line"]);
    let spaced = convert(
        &page,
        &[
            "--header-center",
            "H",
            "--header-line",
            "--header-spacing",
            "5",
        ],
    );

    // The band does not move: it is anchored to the paper.
    assert!(
        (rule(&tight).top - rule(&spaced).top).abs() < 0.5,
        "the band moved: {:.2} became {:.2}",
        rule(&tight).top,
        rule(&spaced).top
    );
    // The content does, by exactly the spacing asked for.
    let moved = content(&tight).top - content(&spaced).top;
    assert!(
        (moved - mm(5.0)).abs() < 1.0,
        "wanted {:.2} pt of gap, measured {moved:.2}",
        mm(5.0)
    );
}

/// A bigger font makes a taller band, and the band grows towards the content
/// rather than away from the paper. A 40pt line is about 46pt tall against a
/// 28pt margin, so the rule lands well below where the 12pt one does.
#[test]
fn the_font_size_decides_how_much_room_the_band_needs() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("bands-font");
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let small = convert(&page, &["--header-center", "H", "--header-line"]);
    let large = convert(
        &page,
        &[
            "--header-center",
            "H",
            "--header-line",
            "--header-font-size",
            "40",
        ],
    );

    assert!(
        rule(&large).top < rule(&small).top - 15.0,
        "a 40pt band should reach much further down than a 12pt one: {:.2} vs {:.2}",
        rule(&large).top,
        rule(&small).top
    );
    // And it reaches past the content, which is what a margin too small for a
    // band looks like. Said here so nobody reports it as a surprise.
    assert!(
        rule(&large).top < content(&large).top,
        "a 40pt band in a 10mm margin runs into the content, and this records it"
    );
}

/// Band options are per object, and in a single-document command line they
/// almost always arrive before the input rather than after it — as defaults that
/// every object inherits. Both the brief's example and the KnpSnappy one in
/// `grammar.rs` are written that way.
#[test]
fn band_options_arrive_as_inherited_defaults() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("bands-inherited");
    let page = fixture::write(scratch.path(), "p.html", BODY);

    // `--footer-center` before the input: a default, not an object's own option.
    let outcome = Run::new()
        .arg("--footer-center")
        .arg("INHERITED")
        .arg(page.display().to_string())
        .arg("-")
        .output();
    outcome.succeeded();
    assert!(
        Pdf::from_bytes(&outcome.stdout)
            .text()
            .contains("INHERITED"),
        "the inherited default should have been drawn"
    );
}

/// wkhtmltopdf's shorthand. The page number half of it is `[page]`, which is
/// #22's business; what this asserts is that the band is drawn at all and that
/// the rule it asks for is there.
#[test]
fn default_header_draws_a_band_with_a_rule() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("bands-default-header");
    let page = fixture::write(scratch.path(), "p.html", BODY);

    let pdf = convert(&page, &["--default-header", "--margin-top", "20mm"]);
    let paper = pdf.media_box(1);
    assert!(
        rule(&pdf).top < paper.top,
        "the default header draws a rule: {}",
        pdf.describe()
    );
}

/// The sheet stamped onto each page is a stream this program writes, and it
/// was going out plain where everything the browser hands over is deflated:
/// 2.7 kB a page, near a tenth of a measured eighteen-page file (D67). What
/// stays plain is the two-byte `q` and `Q` around the page's own drawing,
/// which a zlib header would make bigger.
#[test]
fn the_sheet_stamped_onto_a_page_is_not_carried_plain() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("deflate");
    let header = fixture::write(
        scratch.path(),
        "header.html",
        "<div style=\"border-bottom:1px solid #333\">A header, with a rule under it</div>",
    );
    let page = fixture::write(scratch.path(), "body.html", BODY);

    let pdf = convert(
        &page,
        &[
            "--header-html",
            &header.display().to_string(),
            "--enable-local-file-access",
        ],
    );

    // Not nought: Chromium writes glyph procedures and one-line form contents
    // of about sixty bytes, which a zlib header would make bigger. The bound is
    // what nothing worth deflating can hide under.
    let plain = pdf.plain_streams();
    assert!(
        plain.iter().all(|size| *size < 128),
        "a stream went out plain: {plain:?} in {}",
        pdf.describe()
    );
}
