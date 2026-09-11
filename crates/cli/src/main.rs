//! The `rchtmltopdf` binary.
//!
//! Conversion is not wired up yet. What works today is the command line layer:
//! the grammar is parsed, unimplemented options warn, and `--dump-parse` shows
//! how a command line was understood.

use rchtmltopdf::table::{OptionSpec, SECTIONS, Support};
use rchtmltopdf::tokenizer::{Input, ObjectKind, Output, find_meta_option, tokenize};
use rchtmltopdf_core::ExitCode;
use std::io::Write;
use std::process;

const PROGRAM: &str = "rchtmltopdf";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    process::exit(run(&args).as_i32());
}

fn run(args: &[String]) -> ExitCode {
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
        return run_meta(spec);
    }

    let parsed = match tokenize(args.to_vec()) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("{PROGRAM}: {error}");
            return ExitCode::Failure;
        }
    };

    let quiet = parsed
        .occurrences()
        .any(|occurrence| matches!(occurrence.spec.long, "quiet"));

    if !quiet {
        for occurrence in parsed.occurrences() {
            if let Some(warning) = occurrence.spec.support.warning(&occurrence.as_written) {
                eprintln!("{PROGRAM}: warning: {warning}");
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

    eprintln!(
        "{PROGRAM}: conversion is not implemented yet; only the command line layer is in place."
    );
    eprintln!("{PROGRAM}: run the same command with --dump-parse to see how it was understood.");
    ExitCode::Failure
}

/// Answer a meta option: the ones that report something and exit.
///
/// Dispatching on the table rather than on literal strings means every option
/// marked `Support::Meta` arrives here, so a new one cannot silently fall through
/// to "you need to specify at least one input file". One that has no arm yet is
/// reported as not implemented.
fn run_meta(spec: &'static OptionSpec) -> ExitCode {
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
        // wkhtmltopdf could print its own manual, readme and HTML help. Those are
        // documentation generators we have not built. Say so plainly rather than
        // exiting zero having produced nothing that was asked for.
        other => {
            eprintln!("{PROGRAM}: --{other} is not implemented; see {REPOSITORY}");
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
        Output::Path(path) => println!("output: {path}"),
    }
}

fn describe_input(input: &Input) -> String {
    match input {
        Input::Stdin => "<stdin>".to_string(),
        Input::Url(url) => format!("url {url}"),
        Input::Path(path) => format!("file {path}"),
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
