//! The `rchtmltopdf` binary.
//!
//! Everything the program promises about how it behaves rather than what it
//! renders lives here: which stream each thing is written to, what the exit code
//! says, how much is said at each log level, and the warnings on options that
//! are recognised and not acted on (D02, D14).

use rchtmltopdf::apply::apply;
use rchtmltopdf::convert::convert;
use rchtmltopdf::table::{OptionSpec, SECTIONS, Support};
use rchtmltopdf::tokenizer::{Input, ObjectKind, Output, find_meta_option, tokenize};
use rchtmltopdf::{PROGRAM, VERSION};
use rchtmltopdf_browser::locate::{Flavour, Origin, SystemEnvironment, locate};
use rchtmltopdf_core::ExitCode;
use std::io::Write;
use std::process;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Returns rather than exiting, so every destructor runs: a browser to stop, a
/// profile directory to remove, a document read from standard input to delete.
/// Exiting from inside would skip all of it.
#[tokio::main(flavor = "current_thread")]
async fn main() -> process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = run(&args).await;
    process::ExitCode::from(u8::try_from(outcome.as_i32()).unwrap_or(1))
}

async fn run(args: &[String]) -> ExitCode {
    if args.is_empty() {
        print_usage(&mut std::io::stderr());
        return ExitCode::Failure;
    }

    // Meta options short-circuit: `--help` on its own has neither an input nor an
    // output, so it can never satisfy the grammar and has to be answered first.
    //
    // Recognising them goes through the tokenizer rather than scanning argv,
    // because option values are consumed positionally and a scan would mistake a
    // value for a request. See `find_meta_option`.
    if let Some(spec) = find_meta_option(args) {
        return run_meta(spec, args);
    }

    let parsed = match tokenize(args.to_vec()) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("{PROGRAM}: {error}");
            return ExitCode::Failure;
        }
    };

    // Translate first, but hold any failure: a command line that will not
    // translate is exactly when seeing how it was read is most useful, so
    // --dump-parse still answers below. Warnings come out either way, at the
    // level the command line asked for, falling back to the default when the
    // level itself is what could not be read.
    let translated = apply(&parsed);
    let log_level = translated
        .as_ref()
        .map(|settings| settings.global.log_level)
        .unwrap_or_default();

    if log_level.shows_warnings() {
        // One line per option, not per occurrence: repeating an option should
        // not repeat the warning.
        let mut already_said = Vec::new();
        for occurrence in parsed.occurrences() {
            if already_said.contains(&occurrence.spec.long) {
                continue;
            }
            if let Some(warning) = occurrence.spec.support.warning(&occurrence.as_written) {
                eprintln!("{PROGRAM}: warning: {warning}");
                already_said.push(occurrence.spec.long);
            }
        }
    }

    let dump = parsed
        .globals
        .iter()
        .any(|occurrence| occurrence.spec.long == "dump-parse");
    if dump {
        print_parse(&parsed);
        return ExitCode::Success;
    }

    let settings = match translated {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("{PROGRAM}: {error}");
            return ExitCode::Failure;
        }
    };

    match convert(&settings).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{PROGRAM}: {error}");
            // A run that ended because something would not load ends with the
            // line applications grep for, wherever the failure was noticed —
            // a document missing from the disk reads the same as one missing
            // from the network, because wkhtmltopdf fetched both the same way
            // (D48). Written at every log level: it is a contract, like the
            // exit code it announces.
            if let Some(failure) = error.network_error() {
                eprintln!("Exit with code 1 due to network error: {}", failure.name());
            }
            ExitCode::Failure
        }
    }
}

/// Answer a meta option: the ones that report something and exit.
///
/// Dispatching on the table rather than on literal strings means every option
/// marked `Support::Meta` arrives here, so a new one cannot silently fall through
/// to "you need to specify at least one input file". One that has no arm yet is
/// reported as not implemented.
fn run_meta(spec: &'static OptionSpec, args: &[String]) -> ExitCode {
    match spec.long {
        "help" => {
            print_help(&mut std::io::stdout(), false);
            ExitCode::Success
        }
        "extended-help" => {
            print_help(&mut std::io::stdout(), true);
            ExitCode::Success
        }
        "version" => {
            println!("{PROGRAM} {VERSION}");
            println!("wkhtmltopdf-compatible CLI, modern Chromium rendering");
            ExitCode::Success
        }
        "license" => {
            println!("{PROGRAM} {VERSION}");
            println!();
            println!("Licensed under either of Apache License, Version 2.0 or the MIT license,");
            println!("at your option. SPDX-License-Identifier: MIT OR Apache-2.0");
            println!();
            println!("{REPOSITORY}");
            ExitCode::Success
        }
        "dump-chromium" => report_chromium(args),
        // wkhtmltopdf could print its own manual, readme and HTML help. Those are
        // documentation generators we have not built. Say so plainly rather than
        // exiting zero having produced nothing that was asked for.
        other => {
            eprintln!("{PROGRAM}: --{other} is not implemented; see {REPOSITORY}");
            ExitCode::Failure
        }
    }
}

/// Say which browser would be used, and where it came from.
///
/// The question this answers is "did my path take", which is worth answering
/// because what counts as a path is wider than it looks: a macOS `.app` bundle
/// and an unpacked directory both work, and both used to be refused (D31).
///
/// `--chromium-path` is read through the real parser, never by scanning the
/// arguments: a scan mistakes an option's *value* for a request, which is the
/// bug `find_meta_option` exists because of.
///
/// The grammar wants an input and an output, and
/// `rchtmltopdf --chromium-path X --dump-chromium` has neither — it is a
/// question, not a conversion. So when the line will not parse as it stands, the
/// parser is asked the same question about a line that would: the two
/// placeholders below are never converted and never reach anything. Answering
/// with the wrong browser because the line was missing a filename would be a bad
/// answer to "did my path take".
fn report_chromium(args: &[String]) -> ExitCode {
    let read = |args: Vec<String>| {
        tokenize(args)
            .ok()
            .and_then(|parsed| apply(&parsed).ok())
            .and_then(|settings| settings.global.browser.path.clone())
    };

    let flag = read(args.to_vec()).or_else(|| {
        let mut complete = args.to_vec();
        complete.push("-".to_string());
        complete.push("-".to_string());
        read(complete)
    });

    match locate(flag.as_deref(), &SystemEnvironment) {
        Ok(executable) => {
            println!("path:    {}", executable.path.display());
            println!("source:  {}", executable.origin);
            // The ladder falls through a rung that does not answer (D09), so a
            // path that is not a browser is quietly overtaken by one that is.
            // Sensible for a conversion, and useless as a reply to "did my path
            // take", so the one case where they differ is said out loud.
            if let Some(given) = flag.as_ref().filter(|_| executable.origin != Origin::Flag) {
                println!(
                    "note:    --chromium-path {} is not a browser, so the search carried on",
                    given.display()
                );
            }
            println!(
                "flavour: {}",
                match executable.flavour {
                    Flavour::HeadlessShell => "chrome-headless-shell",
                    Flavour::FullBrowser => "full browser, run with --headless",
                }
            );
            ExitCode::Success
        }
        Err(error) => {
            eprintln!("{PROGRAM}: {error}");
            ExitCode::Failure
        }
    }
}

/// Print the parsed command line in a stable, greppable shape.
fn print_parse(parsed: &rchtmltopdf::Tokenized) {
    let render = |occurrences: &[rchtmltopdf::tokenizer::Occurrence]| {
        for occurrence in occurrences {
            let values = occurrence
                .values
                .iter()
                .map(|value| format!(" {value:?}"))
                .collect::<String>();
            println!("    {}{}", occurrence.as_written, values);
        }
    };

    println!("global options:");
    render(&parsed.globals);
    println!("object option defaults:");
    render(&parsed.defaults);

    for (position, object) in parsed.objects.iter().enumerate() {
        let description = match &object.kind {
            ObjectKind::Page(input) => format!("page {}", describe_input(input)),
            ObjectKind::Cover(input) => format!("cover {}", describe_input(input)),
            ObjectKind::Toc => "toc".to_string(),
        };
        println!("object {position}: {description}");
        render(&object.options);
    }

    match &parsed.output {
        Output::Stdout => println!("output: <stdout>"),
        Output::Path(path) => println!("output: {}", path.display()),
    }
}

fn describe_input(input: &Input) -> String {
    match input {
        Input::Stdin => "<stdin>".to_string(),
        Input::Url(url) => format!("url {url}"),
        Input::Path(path) => format!("file {}", path.display()),
    }
}

fn print_usage(out: &mut impl Write) {
    let _ = writeln!(out, "{PROGRAM} {VERSION}");
    let _ = writeln!(out);
    let _ = writeln!(out, "Usage:");
    let _ = writeln!(
        out,
        "  {PROGRAM} [GLOBAL OPTION]... [OBJECT]... <output file>"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Run `{PROGRAM} --help` for the common options,");
    let _ = writeln!(out, "or `{PROGRAM} --extended-help` for all of them.");
}

/// Render the option table, in wkhtmltopdf's section order.
///
/// The short help lists only what is implemented, which is the honest answer to
/// "what can this do". The extended help lists everything the parser accepts.
fn print_help(out: &mut impl Write, extended: bool) {
    print_usage(out);
    let _ = writeln!(out);
    let _ = writeln!(out, "Description:");
    let _ = writeln!(
        out,
        "  Converts one or more HTML documents into a PDF, driving a headless"
    );
    let _ = writeln!(
        out,
        "  Chromium. The command line follows wkhtmltopdf; the rendering does not."
    );

    for (section, options) in SECTIONS {
        let shown: Vec<_> = options
            .iter()
            .filter(|spec| {
                extended
                    || matches!(
                        spec.support,
                        Support::Implemented | Support::Extension | Support::Meta
                    )
            })
            .collect();
        if shown.is_empty() {
            continue;
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "{section}:");
        for spec in shown {
            let mut left = String::new();
            match spec.short {
                Some(letter) => left.push_str(&format!("  -{letter}, --{}", spec.long)),
                None => left.push_str(&format!("      --{}", spec.long)),
            }
            for value_name in spec.value_names {
                left.push_str(&format!(" <{value_name}>"));
            }
            let note = match spec.support {
                Support::Planned(_) => " [not implemented yet]",
                Support::NoEquivalent(_) => " [accepted, ignored]",
                _ => "",
            };
            if left.len() >= 38 {
                let _ = writeln!(out, "{left}");
                let _ = writeln!(out, "{:38}{}{}", "", spec.help, note);
            } else {
                let _ = writeln!(out, "{left:38}{}{}", spec.help, note);
            }
        }
    }
}
