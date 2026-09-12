//! What a command line with no options at all produces (D03).
//!
//! # Why this is worth a file of its own
//!
//! Every default here is Chromium's opposite. Chromium prints **Letter** with
//! one-centimetre margins using **print** stylesheets; wkhtmltopdf renders A4
//! with ten-millimetre margins using **screen** ones. A document migrated
//! without changing a single option has to come out the way it did before, so
//! these values are load-bearing for the whole project — and every other
//! assertion in this suite is calibrated against them.
//!
//! `crates/core` asserts the same numbers in the settings model. That proves the
//! model says the right thing, not that a conversion does it, and the two can
//! disagree in either direction: a default that never reaches the print call,
//! or a print call that overrides one.
//!
//! # What is proved elsewhere, and is not repeated here
//!
//! - **Local file access is off**: `file_access.rs`, which already owns the
//!   fixture for it.
//! - **Images load**: `page_tuning.rs`, where three images take three pages.
//! - **A deadline exists at all**: `conversion.rs`, on a page that never settles.

use rchtmltopdf_conformance::binary::Run;
use rchtmltopdf_conformance::fixture::{self, Scratch};
use rchtmltopdf_conformance::inspect::{POINTS_PER_INCH, Pdf, TOLERANCE};
use rchtmltopdf_conformance::require_chromium;

fn convert(scratch: &Scratch, name: &str, body: &str, options: &[&str]) -> Pdf {
    let page = fixture::write(scratch.path(), &format!("{name}.html"), body);
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

/// A block that fills the content area, so where it lands *is* the margin.
/// A margin has no entry of its own in a PDF; it can only be measured
/// indirectly.
const FILLS_THE_PAGE: &str = "<div style=\"background:#000;width:100%;height:200pt\"></div>";

/// **A4 portrait, ten millimetres all round, and not a single option to ask for
/// it.** The numbers are written out rather than read from the model, because a
/// number taken from the model would only ever agree with itself.
#[test]
fn a_bare_command_line_produces_a4_with_ten_millimetre_margins() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("defaults-paper");
    let pdf = convert(&scratch, "bare", FILLS_THE_PAGE, &[]);

    // A4 in points, which is what the issue asks this to be compared against.
    let paper = pdf.media_box(1);
    assert!(
        paper.is_about(595.28, 841.89),
        "wanted A4 (595.28 x 841.89 pt), got {}",
        pdf.describe()
    );
    assert!(
        paper.height() > paper.width(),
        "portrait, not landscape: {}",
        pdf.describe()
    );

    let painted = pdf.largest_painted_box(1);
    let close = |actual: f64, wanted: f64, what: &str| {
        assert!(
            (actual - wanted).abs() <= TOLERANCE,
            "{what}: wanted {wanted:.2} pt, measured {actual:.2} pt ({})",
            pdf.describe()
        );
    };
    close(painted.left, mm(10.0), "left margin");
    close(paper.right - painted.right, mm(10.0), "right margin");
    close(paper.top - painted.top, mm(10.0), "top margin");
}

/// **The single easiest thing in the project to get wrong.** Chromium prints
/// with `print` stylesheets unless told otherwise on every single print call, so
/// this is not a value that is set once and forgotten — it has to be sent every
/// time, and a document with `@media print` rules it never used before changes
/// layout the moment it is not.
#[test]
fn screen_stylesheets_win_until_print_is_asked_for() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("defaults-media");

    // The two rules disagree, so the page count says which one applied.
    let body = "<style>\
                @media screen { #pad { height: 400mm; } }\
                @media print  { #pad { height: 0; } }\
                </style><div id=\"pad\"></div><p>after</p>";

    let screen = convert(&scratch, "media", body, &[]);
    assert_eq!(
        screen.page_count(),
        2,
        "the screen rule should have applied: {}",
        screen.describe()
    );

    let print = convert(&scratch, "media", body, &["--print-media-type"]);
    assert_eq!(
        print.page_count(),
        1,
        "the print rule should have applied: {}",
        print.describe()
    );
}

/// Backgrounds print unless told not to, which is wkhtmltopdf's default and not
/// the one a browser's print dialog offers.
///
/// **Measured by colour, not by geometry.** `--no-background` does not stop
/// Chromium emitting the block's rectangle: it emits the same rectangle in the
/// same place and fills it white. A test that measured the box would pass
/// whether the option worked or not, and the first version of this one did.
#[test]
fn backgrounds_are_printed_unless_refused() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("defaults-background");
    let body = "<div style=\"background:#000;width:100mm;height:100mm\"></div>";

    let block = |pdf: &Pdf| {
        pdf.painted(1)
            .into_iter()
            .find(|painted| (painted.rect.width() - mm(100.0)).abs() <= TOLERANCE)
            .unwrap_or_else(|| panic!("the block was not drawn at all: {}", pdf.describe()))
    };

    let printed = convert(&scratch, "bg", body, &[]);
    assert!(
        block(&printed).is_about(0.0, 0.0, 0.0),
        "the block should be black: {:?}",
        block(&printed).fill
    );

    let refused = convert(&scratch, "bg", body, &["--no-background"]);
    assert!(
        block(&refused).is_about(1.0, 1.0, 1.0),
        "the block should have come out white: {:?}",
        block(&refused).fill
    );
}

/// Scripts run. wkhtmltopdf ran them, so a migrated document that builds a table
/// in JavaScript has to keep working.
#[test]
fn javascript_runs_unless_refused() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("defaults-javascript");
    let body = "<div id=\"pad\"></div><p>after</p>\
                <script>document.getElementById('pad').style.height = '400mm';</script>";

    assert_eq!(convert(&scratch, "js", body, &[]).page_count(), 2);
    assert_eq!(
        convert(&scratch, "js", body, &["--disable-javascript"]).page_count(),
        1
    );
}

/// **The default delay is 200ms, and the number matters.** A script that changes
/// the page a second after it settles is missed; one given two seconds is
/// caught. Both margins are wide — five times the default and half the limit —
/// so this measures the setting rather than how fast the machine is.
#[test]
fn the_javascript_delay_is_two_hundred_milliseconds() {
    let Some(_browser) = require_chromium() else {
        return;
    };
    let scratch = Scratch::new("defaults-delay");
    let body = "<div id=\"pad\"></div><p>after</p>\
                <script>setTimeout(() => { \
                document.getElementById('pad').style.height = '400mm'; }, 1000);</script>";

    assert_eq!(
        convert(&scratch, "delay", body, &[]).page_count(),
        1,
        "a change a second late should be missed by a 200ms default"
    );
    assert_eq!(
        convert(&scratch, "delay", body, &["--javascript-delay", "2000"]).page_count(),
        2,
        "and caught when the delay is long enough"
    );
}
