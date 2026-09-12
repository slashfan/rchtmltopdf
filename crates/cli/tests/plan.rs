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
use rchtmltopdf::table::{self, OptionSpec, Support};
use rchtmltopdf::tokenizer::tokenize;
use rchtmltopdf_browser::plan::Plan;
use rchtmltopdf_core::settings::Settings;
use support::line;

/// Options honoured before a browser is started, so no plan can see them.
///
/// Both decide whether warnings are printed, which happens while the command
/// line is still being read. Each names the test that holds it instead.
///
/// **This list is meant to stay this short.** An entry is a claim that something
/// else checks the option, so it has to say what. Adding one without naming a
/// test is how the table started lying in the first place.
const HONOURED_BEFORE_THE_BROWSER: &[(&str, &str)] = &[
    ("quiet", "binary.rs: quiet_is_shorthand_for_log_level_none"),
    (
        "log-level",
        "binary.rs: log_level_decides_whether_warnings_are_shown",
    ),
];

fn settings_for(options: &[&'static OptionSpec]) -> Settings {
    let args = line(options);
    let written = args.join(" ");
    let parsed =
        tokenize(args).unwrap_or_else(|error| panic!("`{written}` did not parse: {error}"));
    apply(&parsed).unwrap_or_else(|error| panic!("`{written}` did not apply: {error}"))
}

/// The conversion these settings describe.
fn plan_for(settings: &Settings) -> Plan {
    let object = settings
        .objects
        .first()
        .expect("every line built here has an object");
    Plan::new(&settings.global, object)
}

/// Every starting point this option moves, as the settings before and after.
///
/// Empty means the command line does not understand the option at all, which is
/// the weaker failure `apply.rs` reports.
fn effects(spec: &'static OptionSpec) -> Vec<(Settings, Settings)> {
    let mut found = Vec::new();

    let bare = settings_for(&[]);
    let alone = settings_for(&[spec]);
    if alone != bare {
        found.push((bare, alone));
    }

    for other in table::all() {
        // Meta options report something and exit; putting one on the line would
        // be describing a different program's run.
        if other.long == spec.long || other.support == Support::Meta {
            continue;
        }
        let before = settings_for(&[other]);
        let after = settings_for(&[other, spec]);
        if after != before {
            found.push((before, after));
        }
    }

    found
}

fn changes_the_conversion(spec: &'static OptionSpec) -> bool {
    effects(spec)
        .iter()
        .any(|(before, after)| plan_for(before) != plan_for(after))
}

/// **The point of this file.** Every option the help advertises as working has
/// to change what a conversion does.
#[test]
fn every_option_marked_implemented_changes_the_conversion() {
    let idle: Vec<String> = table::all()
        .filter(|spec| spec.support == Support::Implemented)
        .filter(|spec| {
            !HONOURED_BEFORE_THE_BROWSER
                .iter()
                .any(|(name, _)| *name == spec.long)
        })
        .filter(|spec| !changes_the_conversion(spec))
        .map(|spec| {
            let understood = !effects(spec).is_empty();
            match understood {
                true => format!("--{} (settings change, the conversion does not)", spec.long),
                false => format!("--{} (nothing happens at all)", spec.long),
            }
        })
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
#[test]
fn nothing_the_table_calls_unbuilt_changes_the_conversion() {
    for spec in table::all()
        .filter(|spec| matches!(spec.support, Support::Planned(_) | Support::NoEquivalent(_)))
    {
        for (before, after) in effects(spec) {
            assert_eq!(
                plan_for(&before),
                plan_for(&after),
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
    let header_left = table::lookup_long("header-left").expect("in the table");
    assert!(matches!(header_left.support, Support::Planned(_)));

    let settings = settings_for(&[header_left]);
    let object = settings.single_object().expect("one object");
    assert_eq!(object.header.left.as_deref(), Some("x"));
}

/// The exemption list is a promise about tests elsewhere. An entry naming an
/// option that no longer exists, or one the table no longer advertises, is a
/// promise about nothing.
#[test]
fn every_exemption_names_a_real_option_and_a_test() {
    for (name, held_by) in HONOURED_BEFORE_THE_BROWSER {
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
        implemented, 30,
        "the number of options honoured end to end changed; \
         if that is deliberate, the audit and this number move together"
    );
}
