//! The option table, held to a real wkhtmltopdf.
//!
//! `table.rs` was written from documented help with no binary on the machine,
//! and that was recorded as risk R01: the short aliases and the exact boundary
//! between global and per-object scope were the two things most likely to be
//! wrong. This test is what closes it. The fixture beside it is the verbatim
//! `--extended-help` of wkhtmltopdf 0.12.6.1; see `fixtures/README.md`.
//!
//! The parser here is deliberately literal about the layout, because the layout
//! is the only thing that distinguishes an option from prose. Two things in the
//! help will fool a looser reader:
//!
//! - a description can begin **one** space after the value placeholder, as in
//!   `--load-error-handling <handler> Specify how to handle...`, so a rule that
//!   looks for a run of spaces loses the option;
//! - descriptions mention other options, indented to column 38, as in
//!   `--header-left='[webpage]'`, so a rule that searches for `--` anywhere
//!   invents options that do not exist.
//!
//! A real option line always begins its long name at column 6: either six
//! spaces, or two spaces, the short alias, a comma and a space. That is the
//! anchor used below.

use rchtmltopdf::table::{self, OptionSpec, SECTIONS, Scope, all, lookup_long};

const HELP: &str = include_str!("fixtures/wkhtmltopdf-0.12.6.1-extended-help.txt");

/// How many options the fixture contains.
///
/// A guard on the parser, not on wkhtmltopdf. Without it a parser that silently
/// matched nothing would make every per-option assertion below pass by having
/// nothing to check.
const REFERENCE_COUNT: usize = 122;

/// The sections that list options, and the scope each one implies.
///
/// Verified against the binary rather than inferred from the heading: an option
/// from `Global Options` or `Outline Options` is refused after the first input,
/// one from `TOC Options` is refused anywhere but after a `toc` object, and the
/// page and header sections are accepted in either place.
const SECTION_SCOPE: &[(&str, Scope)] = &[
    ("Global Options", Scope::Global),
    ("Outline Options", Scope::Global),
    ("Page Options", Scope::Object),
    ("Headers And Footer Options", Scope::Object),
    ("TOC Options", Scope::Toc),
];

#[derive(Debug)]
struct Reference {
    section: &'static str,
    scope: Scope,
    long: &'static str,
    short: Option<char>,
    arity: usize,
}

fn parse() -> Vec<Reference> {
    let mut found = Vec::new();
    let mut section: Option<(&str, Scope)> = None;

    for line in HELP.lines() {
        let trimmed = line.trim_end();
        if !line.starts_with(' ') && trimmed.ends_with(':') {
            let name = trimmed.trim_end_matches(':');
            section = SECTION_SCOPE
                .iter()
                .find(|(heading, _)| *heading == name)
                .map(|(heading, scope)| (*heading, *scope));
            continue;
        }
        let Some((heading, scope)) = section else {
            continue;
        };

        let bytes = line.as_bytes();
        if bytes.len() < 9 || &bytes[6..8] != b"--" {
            continue;
        }
        // "  -d, --dpi": the alias sits at column 3, its comma at column 4.
        let short = if bytes[2] == b'-' && bytes[4] == b',' {
            Some(bytes[3] as char)
        } else {
            None
        };

        let rest = &line[8..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_'))
            .unwrap_or(rest.len());
        let (long, mut rest) = rest.split_at(end);

        // Placeholders follow the name immediately, one space apart. `<>` is a
        // real spelling: --viewport-size has an empty placeholder.
        let mut arity = 0;
        while let Some(after) = rest.strip_prefix(" <") {
            let Some(close) = after.find('>') else { break };
            arity += 1;
            rest = &after[close + 1..];
        }

        found.push(Reference {
            section: heading,
            scope,
            long,
            short,
            arity,
        });
    }
    found
}

#[test]
fn the_fixture_parses_into_the_options_it_contains() {
    let reference = parse();
    assert_eq!(
        reference.len(),
        REFERENCE_COUNT,
        "the parser found {} options; if the fixture was replaced, update REFERENCE_COUNT",
        reference.len()
    );

    // Spot checks on the shapes that broke earlier attempts at this parser.
    let by_name = |name: &str| {
        parse()
            .into_iter()
            .find(|option| option.long == name)
            .unwrap_or_else(|| panic!("--{name} should be in the reference"))
    };
    // Description one space after the placeholder.
    assert_eq!(by_name("load-error-handling").arity, 1);
    // An empty placeholder.
    assert_eq!(by_name("viewport-size").arity, 1);
    // Two placeholders.
    assert_eq!(by_name("cookie").arity, 2);
    // A short alias.
    assert_eq!(by_name("dpi").short, Some('d'));
    // Mentioned in prose at column 38, and a real option in its own right.
    assert_eq!(by_name("header-left").arity, 1);

    let mut names: Vec<&str> = reference.iter().map(|option| option.long).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(
        before,
        names.len(),
        "the parser produced a duplicate option"
    );
}

/// Every option the program has, this table has, spelled the same way.
#[test]
fn every_reference_option_is_in_the_table() {
    let mut wrong = Vec::new();
    for option in parse() {
        let Some(ours) = lookup_long(option.long) else {
            wrong.push(format!("--{} is missing from the table", option.long));
            continue;
        };
        if ours.short != option.short {
            wrong.push(format!(
                "--{}: short alias is {:?}, wkhtmltopdf says {:?}",
                option.long, ours.short, option.short
            ));
        }
        if ours.arity() != option.arity {
            wrong.push(format!(
                "--{}: takes {} value(s), wkhtmltopdf takes {}",
                option.long,
                ours.arity(),
                option.arity
            ));
        }
        if ours.scope != option.scope {
            wrong.push(format!(
                "--{}: scope is {:?}, wkhtmltopdf lists it under {} ({:?})",
                option.long, ours.scope, option.section, option.scope
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Whether an option is ours rather than wkhtmltopdf's.
///
/// The section it is filed under, not the support marker. Those are different
/// axes and used to be conflated: `--dump-chromium` is ours *and* answered
/// before any conversion, so it is `Support::Meta`, and a check that asked about
/// the marker called it an invented wkhtmltopdf option.
fn is_ours(spec: &OptionSpec) -> bool {
    table::EXTENSION_OPTIONS
        .iter()
        .any(|ours| ours.long == spec.long)
}

/// And nothing else, apart from our own.
///
/// An option the real program does not have is worse than a missing one: it
/// accepts a command line that wkhtmltopdf rejects, so the difference surfaces
/// only when someone migrates back, or compares behaviour and finds we are the
/// lenient one.
#[test]
fn the_table_invents_nothing() {
    let invented: Vec<&str> = all()
        .filter(|spec| !is_ours(spec))
        .map(|spec| spec.long)
        .filter(|long| !parse().iter().any(|option| option.long == *long))
        .collect();

    assert!(
        invented.is_empty(),
        "not in wkhtmltopdf 0.12.6.1: {}",
        invented
            .iter()
            .map(|long| format!("--{long}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// Our extensions are ours, and must not collide with a real option.
#[test]
fn our_own_options_do_not_shadow_wkhtmltopdf_ones() {
    for spec in all().filter(|spec| is_ours(spec)) {
        assert!(
            !parse().iter().any(|option| option.long == spec.long),
            "--{} is an extension, but wkhtmltopdf has an option by that name",
            spec.long
        );
    }
}

/// The section an option is filed under, not just the scope it claims.
///
/// `--cookie-jar` had the right name, the right alias and the right arity, and
/// sat in `PAGE_OPTIONS` while the real program lists it as global. Scope alone
/// would not have caught it once the field was corrected, and `--extended-help`
/// prints the table section by section, so a misfiled option is also a wrong
/// help page.
#[test]
fn every_option_is_filed_under_the_section_the_real_help_uses() {
    // Ours on the left, wkhtmltopdf's on the right. Only the first differs, and
    // it differs because "General" is what our help has always called it.
    const HEADINGS: &[(&str, &str)] = &[
        ("General Options", "Global Options"),
        ("Outline Options", "Outline Options"),
        ("Page Options", "Page Options"),
        ("Headers And Footer Options", "Headers And Footer Options"),
        ("TOC Options", "TOC Options"),
    ];

    let ours = |long: &str| -> Option<&'static str> {
        SECTIONS.iter().find_map(|(section, options)| {
            options
                .iter()
                .any(|spec| spec.long == long)
                .then_some(*section)
        })
    };

    let mut wrong = Vec::new();
    for option in parse() {
        let Some(section) = ours(option.long) else {
            continue; // absent from the table; another test says so
        };
        let expected = HEADINGS
            .iter()
            .find(|(mine, _)| *mine == section)
            .map(|(_, theirs)| *theirs);
        if expected != Some(option.section) {
            wrong.push(format!(
                "--{}: filed under {section}, wkhtmltopdf lists it under {}",
                option.long, option.section
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
