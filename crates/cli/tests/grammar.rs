//! The wkhtmltopdf command line grammar, as a contract.
//!
//! Each test is a command line someone actually writes, or an error someone
//! actually hits. New compatibility findings belong here first.

use rchtmltopdf::tokenizer::{
    Input, ObjectKind, Occurrence, Output, ParseError, find_meta_option, tokenize,
};
use rchtmltopdf::{Scope, Tokenized};

fn parse(line: &str) -> Tokenized {
    tokenize(split(line)).unwrap_or_else(|error| panic!("`{line}` failed to parse: {error}"))
}

fn parse_err(line: &str) -> ParseError {
    tokenize(split(line)).expect_err(&format!("`{line}` was expected to fail"))
}

/// Split on spaces, honouring single quotes so tests can contain arguments with
/// spaces without building a Vec by hand.
fn split(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for character in line.chars() {
        match character {
            '\'' => {
                quoted = !quoted;
                started = true;
            }
            ' ' if !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            other => {
                current.push(other);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

fn names(occurrences: &[Occurrence]) -> Vec<&str> {
    occurrences.iter().map(|o| o.spec.long).collect()
}

fn page_input(parsed: &Tokenized, index: usize) -> &Input {
    match &parsed.objects[index].kind {
        ObjectKind::Page(input) | ObjectKind::Cover(input) => input,
        ObjectKind::Toc => panic!("object {index} is a toc, not a page"),
    }
}

// --- the shapes from the brief ----------------------------------------------

#[test]
fn local_file_to_file() {
    let parsed = parse("page.html page.pdf");
    assert_eq!(parsed.objects.len(), 1);
    assert_eq!(page_input(&parsed, 0), &Input::Path("page.html".into()));
    assert_eq!(parsed.output, Output::Path("page.pdf".into()));
}

#[test]
fn url_to_file() {
    let parsed = parse("https://example.com page.pdf");
    assert_eq!(
        page_input(&parsed, 0),
        &Input::Url("https://example.com".into())
    );
}

#[test]
fn stdin_to_file() {
    let parsed = parse("- page.pdf");
    assert_eq!(page_input(&parsed, 0), &Input::Stdin);
    assert_eq!(parsed.output, Output::Path("page.pdf".into()));
}

#[test]
fn file_to_stdout() {
    let parsed = parse("page.html -");
    assert_eq!(parsed.output, Output::Stdout);
}

#[test]
fn stdin_to_stdout() {
    let parsed = parse("- -");
    assert_eq!(page_input(&parsed, 0), &Input::Stdin);
    assert_eq!(parsed.output, Output::Stdout);
}

/// The command line from the brief's "target user experience" section.
#[test]
fn the_invoice_command_from_the_brief() {
    let parsed = parse(
        "--page-size A4 --margin-top 15mm --footer-center 'Page [page] / [topage]' \
         https://example.com/invoice/42 invoice.pdf",
    );

    assert_eq!(names(&parsed.globals), ["page-size", "margin-top"]);
    assert_eq!(parsed.globals[0].values, ["A4"]);
    assert_eq!(parsed.globals[1].values, ["15mm"]);

    // The footer is an object option written before any object, so it becomes a
    // default that every object inherits.
    assert_eq!(names(&parsed.defaults), ["footer-center"]);
    assert_eq!(parsed.defaults[0].values, ["Page [page] / [topage]"]);

    assert_eq!(
        page_input(&parsed, 0),
        &Input::Url("https://example.com/invoice/42".into())
    );
    assert_eq!(parsed.output, Output::Path("invoice.pdf".into()));
}

// --- scope ------------------------------------------------------------------

/// Global options collect in one bucket, whichever object follows them.
///
/// This test used to be called `global_options_are_global_wherever_they_sit`
/// and wrote them *after* the first input to prove the point. That is a command
/// line 0.12.6.1 refuses — see `a_global_option_belongs_before_the_first_input`
/// — so the premise was wrong, and only the half that is true survives.
#[test]
fn global_options_collect_in_one_bucket() {
    let parsed = parse("--page-size Letter --orientation Landscape a.html b.html out.pdf");
    assert_eq!(names(&parsed.globals), ["page-size", "orientation"]);
    assert!(
        parsed
            .objects
            .iter()
            .all(|object| object.options.is_empty())
    );
}

#[test]
fn object_options_attach_to_the_object_they_follow() {
    let parsed = parse("a.html --footer-center A b.html --footer-center B out.pdf");
    assert_eq!(parsed.objects.len(), 2);
    assert_eq!(parsed.defaults, vec![]);
    assert_eq!(parsed.objects[0].options[0].values, ["A"]);
    assert_eq!(parsed.objects[1].options[0].values, ["B"]);
}

#[test]
fn object_options_before_the_first_object_are_defaults() {
    let parsed = parse("--zoom 1.3 a.html b.html out.pdf");
    assert_eq!(names(&parsed.defaults), ["zoom"]);
    assert!(
        parsed
            .objects
            .iter()
            .all(|object| object.options.is_empty())
    );
}

#[test]
fn an_option_after_the_output_still_attaches_to_the_last_object() {
    let parsed = parse("a.html out.pdf --no-background");
    assert_eq!(parsed.output, Output::Path("out.pdf".into()));
    assert_eq!(names(&parsed.objects[0].options), ["no-background"]);
}

#[test]
fn scopes_come_from_the_table() {
    let parsed = parse("--page-size A4 --zoom 2 a.html out.pdf");
    assert_eq!(parsed.globals[0].spec.scope, Scope::Global);
    assert_eq!(parsed.defaults[0].spec.scope, Scope::Object);
}

// --- objects ----------------------------------------------------------------

#[test]
fn explicit_page_keyword() {
    let parsed = parse("page a.html out.pdf");
    assert_eq!(parsed.objects.len(), 1);
    assert_eq!(page_input(&parsed, 0), &Input::Path("a.html".into()));
}

#[test]
fn cover_page_and_toc_in_one_document() {
    let parsed = parse("cover cover.html toc page body.html out.pdf");
    assert_eq!(parsed.objects.len(), 3);
    assert!(matches!(parsed.objects[0].kind, ObjectKind::Cover(_)));
    assert!(matches!(parsed.objects[1].kind, ObjectKind::Toc));
    assert!(matches!(parsed.objects[2].kind, ObjectKind::Page(_)));
}

#[test]
fn toc_options_attach_to_the_toc_object() {
    let parsed = parse("toc --toc-header-text Sommaire a.html out.pdf");
    assert_eq!(names(&parsed.objects[0].options), ["toc-header-text"]);
    assert_eq!(parsed.objects[0].options[0].values, ["Sommaire"]);
}

#[test]
fn several_bare_inputs_become_several_pages() {
    let parsed = parse("a.html b.html c.html out.pdf");
    assert_eq!(parsed.objects.len(), 3);
    assert_eq!(parsed.output, Output::Path("out.pdf".into()));
}

// --- option values ----------------------------------------------------------

#[test]
fn two_value_options_take_both() {
    let parsed = parse("--cookie session abc123 --custom-header X-Trace 42 a.html out.pdf");
    assert_eq!(parsed.defaults[0].values, ["session", "abc123"]);
    assert_eq!(parsed.defaults[1].values, ["X-Trace", "42"]);
}

#[test]
fn repeatable_options_accumulate() {
    let parsed = parse("--cookie a 1 --cookie b 2 a.html out.pdf");
    assert_eq!(parsed.defaults.len(), 2);
    assert!(parsed.defaults[0].spec.repeatable);
}

#[test]
fn short_flags_work() {
    let parsed = parse("-s A4 -O Landscape -T 15mm -q a.html out.pdf");
    assert_eq!(
        names(&parsed.globals),
        ["page-size", "orientation", "margin-top", "quiet"]
    );
    assert_eq!(parsed.globals[0].values, ["A4"]);
    assert_eq!(parsed.globals[0].as_written, "-s");
}

#[test]
fn a_value_may_look_like_an_option() {
    // Negative margins are legal, and the value is taken positionally.
    let parsed = parse("--margin-top -5mm a.html out.pdf");
    assert_eq!(parsed.globals[0].values, ["-5mm"]);
}

#[test]
fn inline_values_are_accepted_as_an_extension() {
    let parsed = parse("--page-size=A4 a.html out.pdf");
    assert_eq!(parsed.globals[0].values, ["A4"]);
    assert_eq!(parsed.globals[0].as_written, "--page-size");
}

#[test]
fn double_dash_ends_option_parsing() {
    let parsed = parse("-- --weird-name.html out.pdf");
    assert_eq!(
        page_input(&parsed, 0),
        &Input::Path("--weird-name.html".into())
    );
}

/// `Occurrence.index` is documented as the position of the option in `argv`.
/// It is the anchor every future error message will quote, so it has to point at
/// the option the user typed, not at whatever the option happened to consume.
#[test]
fn occurrence_index_points_at_the_option_not_its_value() {
    // argv: 0 --margin-top  1 15mm  2 --cookie  3 session  4 abc  5 a.html  6 out.pdf
    let parsed = parse("--margin-top 15mm --cookie session abc a.html out.pdf");
    assert_eq!(parsed.globals[0].as_written, "--margin-top");
    assert_eq!(parsed.globals[0].index, 0);
    assert_eq!(parsed.defaults[0].as_written, "--cookie");
    assert_eq!(parsed.defaults[0].index, 2);
}

#[test]
fn occurrence_index_is_right_for_flags_and_inline_values() {
    // argv: 0 --quiet  1 --page-size=A4  2 a.html  3 out.pdf
    //
    // Both are global options, so they go before the input. Written after it,
    // as this test once had them, 0.12.6.1 refuses the line.
    let parsed = parse("--quiet --page-size=A4 a.html out.pdf");
    assert_eq!(parsed.globals[0].as_written, "--quiet");
    assert_eq!(parsed.globals[0].index, 0);
    assert_eq!(parsed.globals[1].as_written, "--page-size");
    assert_eq!(parsed.globals[1].index, 1);
}

#[test]
fn missing_value_error_points_at_the_option() {
    // argv: 0 a.html  1 out.pdf  2 --margin-top   <- the option is at 2
    match parse_err("a.html out.pdf --margin-top") {
        ParseError::MissingValues { index, .. } => assert_eq!(index, 2),
        other => panic!("expected MissingValues, got {other:?}"),
    }
    // argv: 0 a.html  1 out.pdf  2 --cookie  3 name   <- still the option at 2
    match parse_err("a.html out.pdf --cookie name") {
        ParseError::MissingValues { index, .. } => assert_eq!(index, 2),
        other => panic!("expected MissingValues, got {other:?}"),
    }
}

#[test]
fn positional_index_survives_a_preceding_two_value_option() {
    // A value cursor that leaked into the outer loop would misplace or skip the
    // arguments that follow a multi-value option.
    let parsed = parse("--cookie a 1 --custom-header X 2 in.html out.pdf");
    assert_eq!(names(&parsed.defaults), ["cookie", "custom-header"]);
    assert_eq!(parsed.defaults[0].index, 0);
    assert_eq!(parsed.defaults[1].index, 3);
    assert_eq!(page_input(&parsed, 0), &Input::Path("in.html".into()));
    assert_eq!(parsed.output, Output::Path("out.pdf".into()));
}

#[test]
fn as_written_records_how_the_user_typed_it() {
    let parsed = parse("-T 1mm --margin-left 2mm a.html out.pdf");
    assert_eq!(parsed.globals[0].as_written, "-T");
    assert_eq!(parsed.globals[1].as_written, "--margin-left");
}

// --- errors -----------------------------------------------------------------

#[test]
fn unknown_option_is_an_error() {
    let error = parse_err("--invent-a-flag a.html out.pdf");
    assert!(matches!(error, ParseError::UnknownOption { .. }));
    assert_eq!(error.to_string(), "Unknown long argument --invent-a-flag");
}

#[test]
fn clustered_short_flags_are_rejected_rather_than_guessed() {
    assert!(matches!(
        parse_err("-qg a.html out.pdf"),
        ParseError::UnknownOption { .. }
    ));
}

#[test]
fn a_trailing_option_with_no_value_is_an_error() {
    assert!(matches!(
        parse_err("a.html out.pdf --margin-top"),
        ParseError::MissingValues {
            wanted: 1,
            got: 0,
            ..
        }
    ));
}

#[test]
fn a_two_value_option_missing_its_second_value_is_an_error() {
    assert!(matches!(
        parse_err("a.html out.pdf --cookie name"),
        ParseError::MissingValues {
            wanted: 2,
            got: 1,
            ..
        }
    ));
}

#[test]
fn an_inline_value_on_a_flag_is_an_error() {
    assert!(matches!(
        parse_err("--quiet=yes a.html out.pdf"),
        ParseError::UnexpectedValue { .. }
    ));
}

#[test]
fn an_inline_value_on_a_two_value_option_is_an_error() {
    assert!(matches!(
        parse_err("--cookie=session a.html out.pdf"),
        ParseError::InlineValueNotAllowed { wanted: 2, .. }
    ));
}

#[test]
fn an_output_with_no_input_is_an_error() {
    let error = parse_err("out.pdf");
    assert_eq!(
        error.to_string(),
        "You need to specify at least one input file, and exactly one output file"
    );
}

#[test]
fn no_arguments_at_all_is_an_error() {
    assert!(matches!(parse_err(""), ParseError::NotEnoughArguments));
}

#[test]
fn a_page_keyword_with_nothing_left_for_its_input_is_an_error() {
    // `a.html` is claimed as the output, so `page` has nothing to take. The
    // error names the real mistake rather than complaining about argument count.
    let error = parse_err("page a.html");
    assert!(matches!(error, ParseError::ObjectWithoutInput { .. }));
    assert_eq!(error.to_string(), "`page` must be followed by an input");
}

#[test]
fn a_page_keyword_followed_only_by_the_output_is_an_error() {
    assert!(matches!(
        parse_err("b.html page out.pdf"),
        ParseError::ObjectWithoutInput { .. }
    ));
}

// --- real wrapper output ----------------------------------------------------

/// The shape KnpSnappy generates: every option long, before a temp file input.
#[test]
fn a_knp_snappy_style_command_line() {
    let parsed = parse(
        "--lowquality --margin-bottom 10mm --margin-left 10mm --margin-right 10mm \
         --margin-top 10mm --orientation Portrait --page-size A4 --encoding UTF-8 \
         --footer-font-size 8 --footer-right '[page]/[topage]' --enable-local-file-access \
         /tmp/knp_snappy_abc.html /tmp/knp_snappy_def.pdf",
    );
    assert_eq!(parsed.objects.len(), 1);
    assert_eq!(
        parsed.output,
        Output::Path("/tmp/knp_snappy_def.pdf".into())
    );
    assert!(names(&parsed.globals).contains(&"page-size"));
    assert!(names(&parsed.defaults).contains(&"footer-right"));
    assert!(names(&parsed.defaults).contains(&"enable-local-file-access"));
}

// --- meta options ------------------------------------------------------------

fn meta(line: &str) -> Option<&'static str> {
    find_meta_option(&split(line)).map(|spec| spec.long)
}

/// The bug this guards: option values are consumed positionally on purpose, so a
/// plain scan of argv mistakes a *value* of `-h` for a request for help. Both of
/// these used to print help or the version instead of converting.
#[test]
fn an_option_value_is_never_mistaken_for_a_meta_option() {
    assert_eq!(meta("--footer-center -h page.html out.pdf"), None);
    assert_eq!(meta("--custom-header X-Trace -V a.html b.pdf"), None);
    assert_eq!(meta("--header-left --version a.html out.pdf"), None);
    assert_eq!(meta("--cookie -h -V a.html out.pdf"), None);
    // Even at the end of the line, and even though nothing else follows it,
    // `--version` here is the value of `--margin-top`, not a request.
    assert_eq!(meta("--margin-top --version"), None);

    // And the values still arrive intact.
    let parsed = parse("--footer-center -h page.html out.pdf");
    assert_eq!(parsed.defaults[0].values, ["-h"]);
    assert_eq!(page_input(&parsed, 0), &Input::Path("page.html".into()));
}

#[test]
fn meta_options_are_found_when_they_really_are_options() {
    assert_eq!(meta("--help"), Some("help"));
    assert_eq!(meta("-h"), Some("help"));
    assert_eq!(meta("--extended-help"), Some("extended-help"));
    assert_eq!(meta("-H"), Some("extended-help"));
    assert_eq!(meta("--version"), Some("version"));
    assert_eq!(meta("-V"), Some("version"));
    assert_eq!(meta("--license"), Some("license"));
}

#[test]
fn a_meta_option_is_found_alongside_a_full_command_line() {
    assert_eq!(
        meta("--page-size A4 a.html out.pdf --version"),
        Some("version")
    );
}

#[test]
fn an_ordinary_command_line_has_no_meta_option() {
    assert_eq!(meta("--page-size A4 a.html out.pdf"), None);
    assert_eq!(meta(""), None);
}

/// A command line too broken to tokenize still answers a request for help,
/// rather than reporting the unknown option and leaving the user stuck.
#[test]
fn help_still_works_on_an_otherwise_broken_command_line() {
    assert_eq!(meta("--bogus-option --help"), Some("help"));
    assert_eq!(meta("--help --bogus-option"), Some("help"));
    // A genuinely truncated line: `--cookie` needs two values and gets none.
    assert_eq!(meta("--help --cookie"), Some("help"));
}

/// Positionals are never options, however they are spelled.
#[test]
fn a_file_named_like_a_meta_option_is_not_one() {
    assert_eq!(meta("-- --help out.pdf"), None);
}

// --- where an option may be written (verified against 0.12.6.1) --------------
//
// wkhtmltopdf's grammar has three placement rules, not one, and they were found
// by running the real binary rather than by reading the help:
//
//   global options   only before the first input, bare or keyword-introduced
//   page options     anywhere
//   toc options      only after a `toc` object
//
// The real binary answers all three with `<option> specified in incorrect
// location` and exit 1. Before this, all three were accepted, and a misplaced
// `--toc-header-text` silently attached itself to whichever page preceded it.

#[test]
fn a_global_option_belongs_before_the_first_input() {
    // Where it belongs.
    assert!(tokenize(split("--copies 2 in.html out.pdf")).is_ok());

    // The global area ends at the first input, whether or not `page` was
    // written: `in.html --copies 2` is refused by the real binary.
    for line in [
        "in.html --copies 2 out.pdf",
        "page in.html --copies 2 out.pdf",
        "in.html --copies 2 in.html out.pdf",
        "cover in.html --copies 2 out.pdf",
    ] {
        assert!(
            matches!(parse_err(line), ParseError::WrongLocation { .. }),
            "`{line}` should be refused"
        );
    }
}

/// Outline options are global too, which the section heading does not say.
#[test]
fn an_outline_option_is_global() {
    assert!(tokenize(split("--outline-depth 3 in.html out.pdf")).is_ok());
    assert!(matches!(
        parse_err("in.html --outline-depth 3 out.pdf"),
        ParseError::WrongLocation { .. }
    ));
}

/// Page options are the permissive ones: before any object they apply to all of
/// them, after one they apply to it. Including after a `toc` object, which the
/// real binary also allows.
#[test]
fn a_page_option_may_be_written_anywhere() {
    for line in [
        "--disable-javascript in.html out.pdf",
        "in.html --disable-javascript out.pdf",
        "page in.html --disable-javascript out.pdf",
        "toc --disable-javascript out.pdf",
        "--header-left Hi in.html out.pdf",
        "in.html --header-left Hi out.pdf",
    ] {
        assert!(tokenize(split(line)).is_ok(), "`{line}` should be accepted");
    }
}

/// The finding this test was written for. A TOC option is not an object option
/// with a different name: it is refused in the global area as well as after a
/// page, and only a `toc` object accepts it.
#[test]
fn a_toc_option_belongs_only_to_a_toc_object() {
    assert!(tokenize(split("toc --toc-header-text Hi out.pdf")).is_ok());
    assert!(tokenize(split("toc --toc-header-text Hi in.html out.pdf")).is_ok());

    for line in [
        "--toc-header-text Hi in.html out.pdf",
        "in.html --toc-header-text Hi out.pdf",
        "page in.html --toc-header-text Hi out.pdf",
        // After a toc object, but a page object has since taken over.
        "toc in.html --toc-header-text Hi out.pdf",
    ] {
        assert!(
            matches!(parse_err(line), ParseError::WrongLocation { .. }),
            "`{line}` should be refused"
        );
    }
}

/// Our own options are ours, and carry no compatibility contract, so they are
/// accepted wherever they are written. `--dump-parse` in particular is a
/// debugging aid that people append to a line they have already typed.
#[test]
fn our_own_options_are_not_position_sensitive() {
    for line in [
        "--timeout 5 in.html out.pdf",
        "in.html --timeout 5 out.pdf",
        "in.html out.pdf --dump-parse",
        "page in.html --no-sandbox out.pdf",
    ] {
        assert!(tokenize(split(line)).is_ok(), "`{line}` should be accepted");
    }
}

/// The message has to name the option as written and say where it belongs,
/// because "incorrect location" on its own sends people to the manual.
#[test]
fn a_misplaced_option_says_where_it_belongs() {
    let error = parse_err("in.html --copies 2 out.pdf");
    let message = error.to_string();
    assert!(message.contains("--copies"), "{message}");
    assert!(
        message.contains("before the first"),
        "should say where it belongs: {message}"
    );

    let error = parse_err("in.html --toc-header-text Hi out.pdf");
    let message = error.to_string();
    assert!(message.contains("--toc-header-text"), "{message}");
    assert!(message.contains("toc"), "{message}");
}
