//! The binary, run as a subprocess.
//!
//! Everything the project promises lives in `main`: exit codes, what goes to
//! which stream, quiet mode, and the warnings on options that are recognised but
//! do nothing. None of it was tested before, because `run` is private to a
//! binary target and there was no harness. That is how `--log-level` came to be
//! marked implemented in the option table while doing nothing at all.
//!
//! No new dependency and no browser: cargo hands the built binary's path to the
//! test at compile time, so these are fast and deterministic.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rchtmltopdf"))
        .args(args)
        .output()
        .expect("the binary should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// KnpSnappy raises when the exit code is non-zero **and** stderr is non-empty.
/// It is the pair that matters, so every case asserts both (D14).
fn assert_outcome(output: &Output, code: i32, stderr_empty: bool) {
    assert_eq!(output.status.code(), Some(code), "exit code");
    assert_eq!(
        stderr(output).trim().is_empty(),
        stderr_empty,
        "stderr was {:?}",
        stderr(output)
    );
}

// --- meta options ------------------------------------------------------------

#[test]
fn no_arguments_prints_usage_and_fails() {
    let output = run(&[]);
    assert_outcome(&output, 1, false);
    assert!(stderr(&output).contains("Usage:"));
    assert!(stdout(&output).is_empty(), "usage belongs on stderr");
}

#[test]
fn help_succeeds_and_writes_only_to_stdout() {
    for flag in ["-h", "--help", "-H", "--extended-help"] {
        let output = run(&[flag]);
        assert_outcome(&output, 0, true);
        assert!(stdout(&output).contains("Usage:"), "{flag}");
    }
}

/// The extended help lists everything the parser accepts, including the options
/// that are recognised and ignored. The short help does not.
#[test]
fn extended_help_lists_more_than_short_help() {
    let short = stdout(&run(&["--help"])).lines().count();
    let extended = stdout(&run(&["--extended-help"])).lines().count();
    assert!(extended > short, "short {short}, extended {extended}");
    assert!(stdout(&run(&["--extended-help"])).contains("[accepted, ignored]"));
}

/// The help describes what the program does. It used to carry a note saying
/// conversion was not implemented; that note came out when it was.
#[test]
fn help_describes_what_the_program_does() {
    let help = stdout(&run(&["--help"]));
    assert!(help.contains("Chromium"), "{help}");
    assert!(
        !help.contains("not implemented yet"),
        "the note should be gone"
    );
}

#[test]
fn version_succeeds_and_names_the_program() {
    let output = run(&["--version"]);
    assert_outcome(&output, 0, true);
    assert!(stdout(&output).starts_with("rchtmltopdf "));
}

#[test]
fn the_licence_is_reported() {
    let output = run(&["--license"]);
    assert_outcome(&output, 0, true);
    assert!(stdout(&output).contains("MIT OR Apache-2.0"));
}

/// Documentation generators wkhtmltopdf had and this does not. Exiting zero
/// having produced nothing that was asked for would be worse than saying so.
#[test]
fn unbuilt_meta_options_say_so_and_fail() {
    for flag in ["--manpage", "--readme", "--htmldoc"] {
        let output = run(&[flag]);
        assert_outcome(&output, 1, false);
        assert!(stderr(&output).contains("not implemented"), "{flag}");
    }
}

/// The bug this file exists because of: a plain scan of the arguments mistook an
/// option's *value* for a request for help.
#[test]
fn a_meta_option_as_a_value_is_not_a_request() {
    // Quiet only because a footer is not drawn yet and says so (#19). The case
    // is kept as it was written, because `-h` as a footer is a real thing to
    // want and an invented option would not have found the bug.
    let output = run(&[
        "-q",
        "--footer-center",
        "-h",
        "page.html",
        "out.pdf",
        "--dump-parse",
    ]);
    assert_outcome(&output, 0, true);
    assert!(stdout(&output).contains(r#"--footer-center "-h""#));
}

/// Answers on any machine, with or without a browser, which is the whole point:
/// it is what somebody runs when a conversion said it could not find one.
///
/// Deliberately tolerant about *which* outcome, because both are legitimate and
/// a test that demanded a browser would be untestable on a machine that has
/// none — the same reasoning as `require_chromium`.
#[test]
fn dump_chromium_either_names_a_browser_or_says_how_to_get_one() {
    let output = run(&["--dump-chromium"]);

    match output.status.code() {
        Some(0) => {
            let found = stdout(&output);
            assert!(found.contains("path:"), "{found}");
            assert!(found.contains("source:"), "{found}");
            assert!(found.contains("flavour:"), "{found}");
        }
        _ => {
            let why = stderr(&output);
            assert!(why.contains("could not find Chromium"), "{why}");
            assert!(why.contains("apt install chromium"), "{why}");
            assert!(!why.contains("fetch-chromium"), "no such command: {why}");
        }
    }
}

/// It is a question rather than a conversion, so it needs no document — and the
/// grammar wants one. `--chromium-path` still has to be read through the parser
/// on a line that has neither input nor output.
#[test]
fn dump_chromium_needs_no_document_and_still_reads_the_path() {
    let output = run(&[
        "--chromium-path",
        "/definitely-not-a-browser",
        "--dump-chromium",
    ]);
    let said = format!("{}{}", stdout(&output), stderr(&output));

    assert!(
        !said.contains("at least one input file"),
        "a question should not be answered with the grammar: {said}"
    );
    assert!(
        said.contains("/definitely-not-a-browser"),
        "should say what happened to the path it was given: {said}"
    );
}

// --- errors ------------------------------------------------------------------

#[test]
fn an_unknown_option_fails_with_wkhtmltopdfs_wording() {
    let output = run(&["--invent-a-flag", "a.html", "out.pdf"]);
    assert_outcome(&output, 1, false);
    assert!(stderr(&output).contains("Unknown long argument --invent-a-flag"));
}

#[test]
fn too_few_arguments_fails_with_wkhtmltopdfs_wording() {
    let output = run(&["out.pdf"]);
    assert_outcome(&output, 1, false);
    assert!(
        stderr(&output)
            .contains("You need to specify at least one input file, and exactly one output file")
    );
}

#[test]
fn an_unknown_log_level_is_reported_rather_than_ignored() {
    let output = run(&["--log-level", "shout", "a.html", "out.pdf"]);
    assert_outcome(&output, 1, false);
    // Names the option as written, and says what was expected.
    let message = stderr(&output);
    assert!(message.contains("--log-level"), "{message}");
    assert!(message.contains("unknown log level `shout`"), "{message}");
}

// --- warnings, and how much is said ------------------------------------------

fn warnings(args: &[&str]) -> usize {
    stderr(&run(args))
        .lines()
        .filter(|line| line.contains("warning:"))
        .count()
}

/// D02: an option with no Chromium equivalent is accepted and warned about, so a
/// wrapper emitting it cannot break a production application.
#[test]
fn an_ignored_option_warns_but_does_not_fail() {
    let output = run(&["--grayscale", "a.html", "out.pdf", "--dump-parse"]);
    assert_outcome(&output, 0, false);
    assert!(stderr(&output).contains("--grayscale"));
    assert!(stderr(&output).contains("no Chromium equivalent"));
}

/// `--log-level` was marked implemented in the option table and did nothing.
/// Only `quiet` was ever checked.
#[test]
fn log_level_decides_whether_warnings_are_shown() {
    let noisy = |level: &str| -> usize {
        warnings(&[
            "--log-level",
            level,
            "--grayscale",
            "a.html",
            "out.pdf",
            "--dump-parse",
        ])
    };

    assert_eq!(
        warnings(&["--grayscale", "a.html", "out.pdf", "--dump-parse"]),
        1,
        "the default should warn"
    );
    assert_eq!(noisy("info"), 1);
    assert_eq!(noisy("warn"), 1);
    assert_eq!(noisy("error"), 0);
    assert_eq!(noisy("none"), 0);
}

/// wkhtmltopdf documents `-q` as shorthand for `--log-level none`.
#[test]
fn quiet_is_shorthand_for_log_level_none() {
    assert_eq!(
        warnings(&["-q", "--grayscale", "a.html", "out.pdf", "--dump-parse"]),
        0
    );
    // Written together, the last one wins.
    assert_eq!(
        warnings(&[
            "-q",
            "--log-level",
            "info",
            "--grayscale",
            "a.html",
            "out.pdf",
            "--dump-parse"
        ]),
        1
    );
    assert_eq!(
        warnings(&[
            "--log-level",
            "info",
            "-q",
            "--grayscale",
            "a.html",
            "out.pdf",
            "--dump-parse"
        ]),
        0
    );
}

#[test]
fn a_repeated_option_warns_once_not_once_per_occurrence() {
    assert_eq!(
        warnings(&[
            "--grayscale",
            "--grayscale",
            "--grayscale",
            "a.html",
            "out.pdf",
            "--dump-parse"
        ]),
        1
    );
    // Different options still get a line each.
    assert_eq!(
        warnings(&[
            "--grayscale",
            "--lowquality",
            "a.html",
            "out.pdf",
            "--dump-parse"
        ]),
        2
    );
}

// --- streams -----------------------------------------------------------------

/// Nothing but the result may reach stdout, because the result can *be* stdout.
/// Diagnostics, warnings and progress all belong on stderr.
#[test]
fn diagnostics_never_reach_stdout() {
    let output = run(&["--grayscale", "a.html", "out.pdf", "--dump-parse"]);
    assert!(!stdout(&output).contains("warning"));
    assert!(stderr(&output).contains("warning"));
}

/// Fails before a browser is involved at all, which is why this test needs none.
/// Starting one to discover a file is missing costs a second and produces a PDF
/// of an error page rather than an error.
#[test]
fn a_missing_document_fails_by_name_before_anything_starts() {
    let output = run(&["definitely-not-here.html", "out.pdf"]);
    assert_outcome(&output, 1, false);
    assert!(
        stderr(&output).contains("definitely-not-here.html"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("no such file"),
        "{}",
        stderr(&output)
    );
    assert!(stdout(&output).is_empty());
    assert!(
        !std::path::Path::new("out.pdf").exists(),
        "a failure must leave no document behind"
    );
}

/// Also browser-free: what is supported is settled before one is started.
/// Converting the pages around a cover and saying nothing would produce a
/// document that looks right and is missing part of itself.
#[test]
fn what_is_not_supported_yet_is_refused_by_name() {
    let cover = run(&["cover", "a.html", "out.pdf"]);
    assert_outcome(&cover, 1, false);
    assert!(stderr(&cover).contains("cover"), "{}", stderr(&cover));

    let toc = run(&["toc", "out.pdf"]);
    assert_outcome(&toc, 1, false);
    assert!(
        stderr(&toc).contains("table of contents"),
        "{}",
        stderr(&toc)
    );
}

/// Every document is resolved before a browser starts, so the last of several
/// missing fails as fast as the first, and the failure names it.
#[test]
fn a_missing_document_anywhere_on_the_line_is_refused_before_any_browser() {
    // The first input exists, so the failure can only be the second's.
    let output = run(&[
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/wkhtmltopdf-0.12.6.1-extended-help.txt"
        ),
        "definitely-not-here.html",
        "out.pdf",
    ]);
    assert_outcome(&output, 1, false);
    assert!(
        stderr(&output).contains("definitely-not-here.html"),
        "{}",
        stderr(&output)
    );
    assert!(
        !std::path::Path::new("out.pdf").exists(),
        "a failure must leave no document behind"
    );
}

/// Standard input is a stream, and the second read of it gets nothing.
#[test]
fn standard_input_twice_is_refused_by_name() {
    let output = run(&["-", "-", "out.pdf"]);
    assert_outcome(&output, 1, false);
    assert!(
        stderr(&output).contains("standard input"),
        "{}",
        stderr(&output)
    );
}
