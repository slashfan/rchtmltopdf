//! Turning a command line into settings.
//!
//! This file can only see as far as the settings model. Whether an option then
//! changes anything a conversion does is a stronger question, and `plan.rs` is
//! where it is asked: thirty-eight options once passed every check here while
//! doing nothing at all (#19).

mod support;

use rchtmltopdf::apply::{Applied, PageOverrides, apply, apply_global, apply_object};
use rchtmltopdf::table::{self, Support};
use rchtmltopdf::tokenizer::{Occurrence, tokenize};
use rchtmltopdf_core::page_size::Orientation;
use rchtmltopdf_core::settings::{
    GlobalSettings, LogLevel, MediaType, ObjectKind, ObjectSettings, Settings,
};
use rchtmltopdf_core::{Input, LoadErrorHandling, Output};
use std::time::Duration;
use support::{placeholder, split};

fn settings(line: &str) -> Settings {
    apply(&tokenize(split(line)).expect("should parse"))
        .unwrap_or_else(|error| panic!("`{line}` failed to apply: {error}"))
}

fn settings_err(line: &str) -> String {
    apply(&tokenize(split(line)).expect("should parse"))
        .expect_err(&format!("`{line}` should have failed"))
        .to_string()
}

fn approx(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

// --- the guard that stops the option table lying -----------------------------

fn synthesise(spec: &'static table::OptionSpec) -> Occurrence {
    Occurrence {
        spec,
        as_written: format!("--{}", spec.long),
        values: spec
            .value_names
            .iter()
            .map(|name| placeholder(spec, name).to_string())
            .collect(),
        index: 0,
    }
}

fn drive(spec: &'static table::OptionSpec) -> Applied {
    let occurrence = synthesise(spec);
    let outcome = if spec.scope == table::Scope::Global {
        let mut global = GlobalSettings::default();
        let mut page = PageOverrides::default();
        apply_global(&occurrence, &mut global, &mut page)
    } else {
        let mut object = ObjectSettings::page(Input::Stdin);
        apply_object(&occurrence, &mut object)
    };
    outcome.unwrap_or_else(|error| panic!("--{} rejected a plausible value: {error}", spec.long))
}

/// `Support::Implemented` is a claim that an option does something. This holds
/// the weaker half of it — the option reaches the settings model — and `plan.rs`
/// holds the half that matters.
#[test]
fn every_option_marked_implemented_is_actually_applied() {
    let unhandled: Vec<&str> = table::all()
        .filter(|spec| spec.support == Support::Implemented)
        .filter(|spec| drive(spec) == Applied::NotYet)
        .map(|spec| spec.long)
        .collect();

    assert!(
        unhandled.is_empty(),
        "marked implemented but nothing acts on them: {unhandled:?}"
    );
}

/// The other direction, for the half of it that is settled. An option with no
/// Chromium equivalent is never going to be built, so an arm for one is a
/// mistake rather than unfinished work.
///
/// `Planned` is deliberately not included. Those may be understood here and
/// ignored by the conversion, which is the normal half-built state and the one
/// the settings model is written in front of. What keeps that honest is that
/// nothing downstream reads the field, and `plan.rs` is what holds it.
#[test]
fn nothing_acts_on_an_option_that_will_never_have_an_equivalent() {
    let acted: Vec<&str> = table::all()
        .filter(|spec| matches!(spec.support, Support::NoEquivalent(_)))
        .filter(|spec| drive(spec) != Applied::NotYet)
        .map(|spec| spec.long)
        .collect();

    assert!(
        acted.is_empty(),
        "acted on, and no browser can honour them: {acted:?}"
    );
}

// --- defaults and inheritance -------------------------------------------------

#[test]
fn a_bare_command_line_gets_the_wkhtmltopdf_defaults() {
    let settings = settings("page.html out.pdf");
    approx(settings.global.page.size.width.to_mm(), 210.0);
    approx(settings.global.page.margins.top.to_mm(), 10.0);
    assert_eq!(settings.global.page.orientation, Orientation::Portrait);
    assert_eq!(settings.global.output, Output::Path("out.pdf".into()));
    assert_eq!(settings.global.timeout, Some(Duration::from_secs(30)));

    let object = settings.single_object().expect("one object");
    assert_eq!(object.kind, ObjectKind::Page);
    assert_eq!(object.input, Some(Input::Path("page.html".into())));
    assert_eq!(object.web.media_type, MediaType::Screen);
    assert!(!object.web.local_file_access);
}

#[test]
fn defaults_are_inherited_and_can_be_overridden_per_object() {
    let settings =
        settings("--footer-center Everywhere --zoom 2 a.html b.html --footer-center JustB out.pdf");
    assert_eq!(settings.objects.len(), 2);
    assert_eq!(
        settings.objects[0].footer.center.as_deref(),
        Some("Everywhere")
    );
    assert_eq!(settings.objects[1].footer.center.as_deref(), Some("JustB"));
    // An override replaces only what it names.
    approx(settings.objects[0].web.zoom, 2.0);
    approx(settings.objects[1].web.zoom, 2.0);
}

#[test]
fn repeatable_options_accumulate_and_others_take_the_last() {
    let settings = settings(
        "--cookie a 1 --cookie b 2 --allow /one --allow /two --zoom 1.5 --zoom 3 in.html out.pdf",
    );
    let object = settings.single_object().unwrap();
    assert_eq!(object.web.cookies.len(), 2);
    assert_eq!(object.web.cookies[1].name, "b");
    assert_eq!(object.web.allowed_paths.len(), 2);
    approx(object.web.zoom, 3.0);
}

// --- paper --------------------------------------------------------------------

#[test]
fn a_named_page_size_is_resolved() {
    approx(
        settings("--page-size Letter a.html out.pdf")
            .global
            .page
            .size
            .width
            .to_inches(),
        8.5,
    );
}

/// An explicit dimension wins over a named size whichever order they appear in,
/// so two equivalent command lines cannot disagree.
#[test]
fn an_explicit_dimension_overrides_a_named_size_in_either_order() {
    let first = settings("--page-size A4 --page-width 100mm a.html out.pdf");
    let second = settings("--page-width 100mm --page-size A4 a.html out.pdf");
    approx(first.global.page.size.width.to_mm(), 100.0);
    approx(first.global.page.size.height.to_mm(), 297.0);
    assert_eq!(first.global.page.size, second.global.page.size);
}

#[test]
fn orientation_swaps_after_the_size_is_resolved() {
    let before = settings("--orientation Landscape --page-size A4 a.html out.pdf");
    let after = settings("--page-size A4 --orientation Landscape a.html out.pdf");
    approx(before.global.page.effective_size().width.to_mm(), 297.0);
    assert_eq!(
        before.global.page.effective_size(),
        after.global.page.effective_size()
    );
}

#[test]
fn margins_accept_every_unit_and_the_short_flags() {
    let margins = settings("-T 1in -B 2cm -L 5 -R 12pt a.html out.pdf")
        .global
        .page
        .margins;
    approx(margins.top.to_inches(), 1.0);
    approx(margins.bottom.to_mm(), 20.0);
    // A bare number is millimetres, as wkhtmltopdf reads it.
    approx(margins.left.to_mm(), 5.0);
    approx(margins.right.to_inches(), 12.0 / 72.0);
}

// --- the rest of the surface ---------------------------------------------------

#[test]
fn quiet_and_log_level_both_reach_the_settings() {
    assert_eq!(
        settings("-q a.html out.pdf").global.log_level,
        LogLevel::None
    );
    assert_eq!(
        settings("--log-level error a.html out.pdf")
            .global
            .log_level,
        LogLevel::Error
    );
}

#[test]
fn a_timeout_of_zero_means_no_limit() {
    assert_eq!(settings("--timeout 0 a.html out.pdf").global.timeout, None);
    assert_eq!(
        settings("--timeout 5 a.html out.pdf").global.timeout,
        Some(Duration::from_secs(5))
    );
}

#[test]
fn the_load_and_web_options_land_where_they_belong() {
    let settings = settings(
        "--javascript-delay 750 --window-status ready --run-script 'a()' --run-script 'b()' \
         --load-error-handling skip --load-media-error-handling abort --print-media-type \
         --no-background --disable-javascript --no-images --enable-local-file-access \
         --encoding UTF-8 --minimum-font-size 9 --viewport-size 1280x1024 \
         --username bob --password hunter2 --custom-header X-A 1 --custom-header-propagation \
         in.html out.pdf",
    );
    let object = settings.single_object().unwrap();

    assert_eq!(object.load.javascript_delay, Duration::from_millis(750));
    assert_eq!(object.load.window_status.as_deref(), Some("ready"));
    assert_eq!(object.load.run_scripts, ["a()", "b()"]);
    assert_eq!(object.load.on_document_error, LoadErrorHandling::Skip);
    assert_eq!(object.load.on_media_error, LoadErrorHandling::Abort);

    assert_eq!(object.web.media_type, MediaType::Print);
    assert!(!object.web.background);
    assert!(!object.web.javascript);
    assert!(!object.web.images);
    assert!(object.web.local_file_access);
    assert_eq!(object.web.encoding.as_deref(), Some("UTF-8"));
    assert_eq!(object.web.minimum_font_size, Some(9));
    assert_eq!(object.web.viewport, (1280, 1024));
    assert_eq!(object.web.username.as_deref(), Some("bob"));
    assert_eq!(object.web.password.as_deref(), Some("hunter2"));
    assert_eq!(object.web.custom_headers[0].name, "X-A");
    assert!(object.web.propagate_custom_headers);
}

#[test]
fn headers_and_footers_land_where_they_belong() {
    let settings = settings(
        "--header-left L --header-center C --header-right R --header-line \
         --footer-center 'Page [page] / [topage]' --footer-font-size 8 --footer-spacing 5 \
         --replace who world in.html out.pdf",
    );
    let object = settings.single_object().unwrap();
    assert_eq!(object.header.left.as_deref(), Some("L"));
    assert_eq!(object.header.center.as_deref(), Some("C"));
    assert!(object.header.line);
    assert_eq!(
        object.footer.center.as_deref(),
        Some("Page [page] / [topage]")
    );
    assert_eq!(object.footer.font_size, Some(8.0));
    assert_eq!(object.footer.spacing, Some(5.0));
    assert_eq!(object.replacements[0].name, "who");
}

/// wkhtmltopdf's shorthand fills both halves of the band.
///
/// It also made room for the band by pushing the top margin down to 20mm, and
/// that half is deliberately gone until the band is drawn. Moving a document's
/// content down a centimetre to leave space for a header nobody prints is worse
/// than ignoring the option, and it is what this program did (#19, #21).
#[test]
fn default_header_fills_the_band_without_moving_the_content() {
    let settings = settings("--default-header a.html out.pdf");
    let object = settings.single_object().unwrap();
    assert_eq!(object.header.left.as_deref(), Some("[webpage]"));
    assert_eq!(object.header.right.as_deref(), Some("[page]/[topage]"));
    assert!(object.header.line);
    approx(settings.global.page.margins.top.to_mm(), 10.0);
}

#[test]
fn objects_keep_their_kind_and_a_toc_has_no_input() {
    let settings = settings("cover c.html toc page b.html out.pdf");
    assert_eq!(settings.objects[0].kind, ObjectKind::Cover);
    assert_eq!(settings.objects[1].kind, ObjectKind::Toc);
    assert_eq!(settings.objects[1].input, None);
    assert_eq!(settings.objects[2].kind, ObjectKind::Page);
}

#[test]
fn options_that_are_not_built_change_nothing() {
    // Each of these is recognised and warned about by the binary. None may
    // quietly alter what the conversion does.
    let plain = settings("a.html out.pdf");
    let noisy =
        settings("--grayscale --lowquality --dpi 96 --disable-smart-shrinking a.html out.pdf");
    assert_eq!(plain.global.page, noisy.global.page);
    assert_eq!(plain.objects, noisy.objects);
}

// --- errors --------------------------------------------------------------------

/// These are read out of a thrown exception by somebody who did not write the
/// command line. They name the option as it was written and say what was wanted.
#[test]
fn errors_echo_the_option_exactly_as_written() {
    let long = settings_err("--margin-top wat a.html out.pdf");
    assert!(long.starts_with("--margin-top:"), "{long}");

    let short = settings_err("-T wat a.html out.pdf");
    assert!(short.starts_with("-T:"), "{short}");
    assert!(
        !short.contains("margin-top"),
        "should not rename it: {short}"
    );
}

#[test]
fn an_unknown_page_size_lists_the_ones_that_exist() {
    let message = settings_err("--page-size A11 a.html out.pdf");
    assert!(message.contains("unknown page size `A11`"), "{message}");
    assert!(
        message.contains("A4") && message.contains("Letter"),
        "{message}"
    );
}

#[test]
fn a_bad_length_reuses_the_parsers_own_message() {
    let message = settings_err("--margin-top 10furlongs a.html out.pdf");
    assert!(message.contains("mm, cm, in, pt or px"), "{message}");
}

#[test]
fn bad_enumerated_values_list_what_was_expected() {
    assert!(
        settings_err("--orientation sideways a.html out.pdf").contains("Portrait or Landscape")
    );
    assert!(
        settings_err("--load-error-handling explode a.html out.pdf")
            .contains("abort, ignore or skip")
    );
    assert!(settings_err("--log-level shout a.html out.pdf").contains("none, error, warn or info"));
}

#[test]
fn a_bad_number_and_a_bad_viewport_are_reported() {
    assert!(settings_err("--zoom loads a.html out.pdf").contains("is not a number"));
    assert!(settings_err("--viewport-size wide a.html out.pdf").contains("expected 1024x768"));
    assert!(settings_err("--viewport-size 1024xtall a.html out.pdf").contains("expected 1024x768"));
}
