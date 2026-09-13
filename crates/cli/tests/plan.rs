//! What the option table claims, held to what a conversion would actually do.
//!
//! # The claim that was not being checked
//!
//! `Support::Implemented` says an option changes what comes out of the program.
//! Nothing held it to that, and it was wrong about thirty-eight of the
//! fifty-eight options it was written on. The command line filled a settings
//! field, nothing downstream ever read the field, and the help said the option
//! worked. `--default-header` managed worse than inert: it moved the top margin
//! down by a centimetre to make room for a band nothing draws (#19).
//!
//! The earlier guard in `apply.rs` cannot see any of this. It asks whether the
//! translation layer has an arm for an option, which is a question about the
//! command line, and every one of those thirty-eight had one.
//!
//! # How this asks a different question
//!
//! `Plan` is everything a conversion would do, worked out without doing it
//! (D27). So an option is implemented if, from some starting point, writing it
//! changes the plan — and is not, whatever the table says, if it cannot change
//! the plan from any starting point at all.
//!
//! **Why "from some starting point" and not "from the defaults".** A flag that
//! restates a default changes nothing when it is the only thing on the line:
//! `--background`, `--enable-javascript` and `--no-print-media-type` all ask for
//! what they would have got anyway. What they change is the effect of their
//! opposite, and the table contains their opposite, so the search below tries
//! every other option as a starting point and stops at the first one the option
//! under test moves.

mod support;

use rchtmltopdf::apply::apply;
use rchtmltopdf::table::Scope;
use rchtmltopdf::table::{self, OptionSpec, Support};
use rchtmltopdf::tokenizer::tokenize;
use rchtmltopdf_browser::plan::Plan;
use rchtmltopdf_core::Clock;
use rchtmltopdf_core::settings::Settings;
use support::{line_shaped, section_of};

/// Options no plan can see, because they are not about what the browser is asked
/// to do.
///
/// Two decide how much is said while the command line is still being read. Two
/// decide what to make of what came back: whether a failed subresource is worth
/// an exit code is a judgement on the result, and the browser was asked exactly
/// the same thing either way.
///
/// **Each entry names the test that holds it instead.** An entry without one is
/// how the table started lying in the first place, and there is a test below
/// that checks every entry names a real option that is still advertised.
const HONOURED_OUTSIDE_THE_PLAN: &[(&str, &str)] = &[
    ("quiet", "binary.rs: quiet_is_shorthand_for_log_level_none"),
    (
        "log-level",
        "binary.rs: log_level_decides_whether_warnings_are_shown",
    ),
    (
        "load-media-error-handling",
        "conformance/failures.rs: a_failed_subresource_is_judged_by_the_media_handler",
    ),
    (
        "load-error-handling",
        "conformance/failures.rs: ignore_prints_the_document_that_did_load",
    ),
];

/// Documents a plan is built against.
///
/// Both, because some options only do anything for one kind. `--encoding` can
/// only be applied to a document we can read ourselves, and `--cookie` needs an
/// origin to scope a cookie to. A single document would call one of them inert.
const DOCUMENTS: &[&str] = &["file:///doc.html", "https://example.com/doc"];

/// A frozen clock, for the same reason there are fixed documents: two plans
/// built a moment apart have to be equal, or a band carrying `[time]` would make
/// every option look as though it changed something.
const FROZEN: Clock = Clock {
    year: 2026,
    month: 9,
    day: 12,
    hour: 12,
    minute: 0,
    second: 0,
    utc_offset_seconds: 0,
};

fn settings_shaped(options: &[&'static OptionSpec], toc_object: bool) -> Settings {
    let args = line_shaped(options, toc_object);
    let written = args.join(" ");
    let parsed =
        tokenize(args).unwrap_or_else(|error| panic!("`{written}` did not parse: {error}"));
    apply(&parsed).unwrap_or_else(|error| panic!("`{written}` did not apply: {error}"))
}

/// The conversion these settings describe, against one of the fixed documents.
fn plan_for(settings: &Settings, document: &str) -> Plan {
    let object = settings
        .objects
        .first()
        .expect("every line built here has an object");
    Plan::new(&settings.global, object, FROZEN, document)
}

/// Whether writing `spec` on top of `written` changes any conversion.
fn moves_anything(written: &[&'static OptionSpec], spec: &'static OptionSpec) -> bool {
    let mut with = written.to_vec();
    with.push(spec);

    // Both halves are built on the same object. A table-of-contents option can
    // only be written after a `toc`, which has no document of its own, so a line
    // carrying one differs from a line carrying a page in more than the option
    // under test.
    let toc = with.iter().any(|option| option.scope == Scope::Toc);
    let before = settings_shaped(written, toc);
    let after = settings_shaped(&with, toc);
    if before == after {
        return false;
    }
    DOCUMENTS
        .iter()
        .any(|document| plan_for(&before, document) != plan_for(&after, document))
}

/// Every starting point of one other option, which is enough for all but a
/// handful.
fn single_baselines(spec: &'static OptionSpec) -> Vec<Vec<&'static OptionSpec>> {
    let mut baselines = vec![Vec::new()];
    for other in table::all() {
        // Meta options report something and exit; putting one on the line would
        // be describing a different program's run.
        if other.long == spec.long || other.support == Support::Meta {
            continue;
        }
        baselines.push(vec![other]);
    }
    baselines
}

fn changes_the_conversion(spec: &'static OptionSpec) -> bool {
    if single_baselines(spec)
        .iter()
        .any(|written| moves_anything(written, spec))
    {
        return true;
    }

    // Some options modulate another rather than doing anything alone.
    // `--no-custom-header-propagation` decides *where* `--custom-header` goes, so
    // one other option on the line is not enough to make it bite and two are:
    // the header, and the propagation it is turning off. Only reached when
    // nothing simpler worked, and bounded to the option's own section of the
    // table, which is wkhtmltopdf's own grouping of things that interact.
    let section = section_of(spec);
    section.iter().any(|first| {
        section.iter().any(|second| {
            first.long != second.long
                && first.long != spec.long
                && second.long != spec.long
                && moves_anything(&[first, second], spec)
        })
    })
}

/// **The point of this file.** Every option the help advertises as working has
/// to change what a conversion does.
#[test]
fn every_option_marked_implemented_changes_the_conversion() {
    let idle: Vec<&str> = table::all()
        .filter(|spec| spec.support == Support::Implemented)
        .filter(|spec| {
            !HONOURED_OUTSIDE_THE_PLAN
                .iter()
                .any(|(name, _)| *name == spec.long)
        })
        .filter(|spec| !changes_the_conversion(spec))
        .map(|spec| spec.long)
        .collect();

    assert!(
        idle.is_empty(),
        "the table calls these implemented and they change nothing that is printed: {idle:#?}"
    );
}

/// The other direction, and the one that caught `--default-header` moving the
/// margin for a band nothing draws. The binary warns that these are ignored, so
/// one that quietly changed the output would make the warning a lie — and a
/// wrapper would be emitting an option that silently alters a document.
///
/// One other option on the line, not two: the escalation above exists to prove a
/// positive, and running it for every unbuilt option would cost far more than it
/// could find.
#[test]
fn nothing_the_table_calls_unbuilt_changes_the_conversion() {
    for spec in table::all()
        .filter(|spec| matches!(spec.support, Support::Planned(_) | Support::NoEquivalent(_)))
    {
        for written in single_baselines(spec) {
            assert!(
                !moves_anything(&written, spec),
                "--{} is warned about as ignored, and it changed the conversion",
                spec.long
            );
        }
    }
}

/// A `Planned` option may still be understood as far as the settings model, and
/// several are. That is a half-built option rather than a broken one, and it is
/// the state the next layer's work lands in — but only while nothing reads the
/// field, which is what the test above holds.
#[test]
fn an_option_that_is_not_built_may_still_be_understood() {
    let copies = table::lookup_long("copies").expect("in the table");
    assert!(matches!(copies.support, Support::Planned(_)));
    // Understood as far as the grammar: it parses, takes its value, and the
    // conversion never asks.
    assert_eq!(copies.arity(), 1);
    assert!(!changes_the_conversion(copies));
}

/// The exemption list is a promise about tests elsewhere. An entry naming an
/// option that no longer exists, or one the table no longer advertises, is a
/// promise about nothing.
#[test]
fn every_exemption_names_a_real_option_and_a_test() {
    for (name, held_by) in HONOURED_OUTSIDE_THE_PLAN {
        let spec = table::lookup_long(name).unwrap_or_else(|| panic!("--{name} is not an option"));
        assert_eq!(
            spec.support,
            Support::Implemented,
            "--{name} is exempt from a check it is not subject to"
        );
        assert!(held_by.contains(".rs: "), "{name} names no test");
    }
}

/// Not a guard, a record. The audit is the deliverable of #19, and a count that
/// silently grows is how the previous claim went unexamined for so long: the
/// table advertised sixty-one working options before anything read a command
/// line at all.
#[test]
fn the_advertised_surface_is_the_audited_one() {
    let implemented = table::all()
        .filter(|spec| spec.support == Support::Implemented)
        .count();
    assert_eq!(
        implemented, 77,
        "the number of options honoured end to end changed; \
         if that is deliberate, the audit and this number move together"
    );
}
