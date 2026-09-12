//! Turning `argv` into objects, options and an output path.
//!
//! # The grammar
//!
//! ```text
//! rchtmltopdf [GLOBAL OPTION]... [OBJECT]... <output file>
//! ```
//!
//! An object is `page <input>`, `cover <input>`, `toc`, or a bare input. The
//! last positional argument is always the output, and everything positional
//! before it is an input. Options may appear anywhere; a global option applies
//! to the whole run wherever it sits, while an object option applies to the
//! object it follows. Object options written before the first object become
//! defaults inherited by every object.
//!
//! # Two deliberate departures from wkhtmltopdf
//!
//! `--option=value` is accepted for options taking exactly one value, and `--`
//! ends option parsing. wkhtmltopdf supports neither, so both can only turn a
//! command line it would reject into one that works.

use crate::table::{OptionSpec, Scope, Support, lookup_long, lookup_short};
// The document model owns these: the browser and PDF layers need them too, and
// nothing downstream should have to know how a command line was written.
pub use rchtmltopdf_core::document::{Input, Output};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectKind {
    Page(Input),
    Cover(Input),
    Toc,
}

/// One object on the command line, with the options written after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub kind: ObjectKind,
    pub options: Vec<Occurrence>,
}

/// One option as it was actually written, with the values it consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub spec: &'static OptionSpec,
    /// Exactly how the user wrote it, such as `--margin-top` or `-T`.
    pub as_written: String,
    pub values: Vec<String>,
    /// Position in `argv`, for error messages.
    pub index: usize,
}

/// A command line, taken apart but not yet interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokenized {
    /// Options applying to the whole run.
    pub globals: Vec<Occurrence>,
    /// Object options written before the first object, inherited by all of them.
    pub defaults: Vec<Occurrence>,
    pub objects: Vec<Object>,
    pub output: Output,
}

impl Tokenized {
    /// Every option occurrence, in the order it was written.
    pub fn occurrences(&self) -> impl Iterator<Item = &Occurrence> {
        self.globals
            .iter()
            .chain(self.defaults.iter())
            .chain(self.objects.iter().flat_map(|object| object.options.iter()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    UnknownOption {
        as_written: String,
        index: usize,
    },
    MissingValues {
        as_written: String,
        wanted: usize,
        got: usize,
        index: usize,
    },
    /// `--flag=value` where the flag takes no value.
    UnexpectedValue {
        as_written: String,
        index: usize,
    },
    /// `--cookie=a` where the option needs two values.
    InlineValueNotAllowed {
        as_written: String,
        wanted: usize,
        index: usize,
    },
    /// `page` or `cover` with nothing after it to use as the input.
    ObjectWithoutInput {
        keyword: String,
        index: usize,
    },
    /// Fewer than one input and one output.
    NotEnoughArguments,
    /// An option written somewhere its scope does not allow.
    WrongLocation {
        as_written: String,
        belongs: Placement,
        index: usize,
    },
}

/// Where a misplaced option should have gone.
///
/// wkhtmltopdf has three placement rules, not one, and they were established by
/// running 0.12.6.1 rather than by reading its help, which does not spell them
/// out. Page options go anywhere; the other two are constrained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Before the first input, bare or introduced by `page`, `cover` or `toc`.
    GlobalArea,
    /// Immediately after a `toc` object, and before any later object.
    TocObject,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnknownOption { as_written, .. } => {
                write!(f, "Unknown long argument {as_written}")
            }
            ParseError::MissingValues {
                as_written,
                wanted,
                got,
                ..
            } => write!(
                f,
                "{as_written} needs {wanted} value(s), but only {got} were given"
            ),
            ParseError::UnexpectedValue { as_written, .. } => {
                write!(f, "{as_written} takes no value")
            }
            ParseError::InlineValueNotAllowed {
                as_written, wanted, ..
            } => write!(
                f,
                "{as_written} needs {wanted} values, so they must be written as separate arguments"
            ),
            ParseError::ObjectWithoutInput { keyword, .. } => {
                write!(f, "`{keyword}` must be followed by an input")
            }
            ParseError::NotEnoughArguments => write!(
                f,
                "You need to specify at least one input file, and exactly one output file"
            ),
            ParseError::WrongLocation {
                as_written,
                belongs: Placement::GlobalArea,
                ..
            } => write!(
                f,
                "{as_written} is a global option, so it must be written before the first input"
            ),
            ParseError::WrongLocation {
                as_written,
                belongs: Placement::TocObject,
                ..
            } => write!(
                f,
                "{as_written} applies to a table of contents, so it must follow a `toc` object"
            ),
        }
    }
}

impl std::error::Error for ParseError {}

/// A single argument, once classified.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Opt(Occurrence),
    Positional { value: String, index: usize },
}

/// Split a command line into objects, options and an output.
///
/// `args` must not include the program name.
pub fn tokenize<I, S>(args: I) -> Result<Tokenized, ParseError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let argv: Vec<String> = args.into_iter().map(Into::into).collect();
    let tokens = classify(&argv)?;
    assemble(tokens)
}

/// Find the meta option on a command line, if there is one.
///
/// Meta options are the ones answered without converting anything: `--help`,
/// `--version` and friends. They have to be recognised before the grammar is
/// enforced, because `--help` on its own has neither an input nor an output and
/// so can never satisfy it.
///
/// The recognition runs the **first tokenizing pass**, which knows how many
/// values each option takes. That is the whole point. Option values are consumed
/// positionally on purpose, so a plain scan of `argv` mistakes a *value* for a
/// request:
///
/// ```text
/// rchtmltopdf --footer-center -h page.html out.pdf   # a footer reading "-h"
/// rchtmltopdf --custom-header X-Trace -V a.html b.pdf
/// ```
///
/// When the first pass fails, the command line is malformed in some other way,
/// and a plain scan is used as a fallback so that asking for help still works on
/// a line that is otherwise broken.
pub fn find_meta_option(args: &[String]) -> Option<&'static OptionSpec> {
    let is_meta = |spec: &&'static OptionSpec| spec.support == Support::Meta;

    match classify(args) {
        Ok(tokens) => tokens.iter().find_map(|token| match token {
            Token::Opt(occurrence) if occurrence.spec.support == Support::Meta => {
                Some(occurrence.spec)
            }
            _ => None,
        }),
        Err(_) => args
            .iter()
            .filter(|arg| looks_like_option(arg))
            .find_map(|arg| {
                let (name, _) = split_inline_value(arg);
                resolve(name, 0).ok().filter(is_meta)
            }),
    }
}

/// First pass: decide what each argument is, and pull option values out.
fn classify(argv: &[String]) -> Result<Vec<Token>, ParseError> {
    let mut tokens = Vec::with_capacity(argv.len());
    let mut index = 0;
    let mut options_ended = false;

    while index < argv.len() {
        let arg = &argv[index];

        if options_ended || !looks_like_option(arg) {
            tokens.push(Token::Positional {
                value: arg.clone(),
                index,
            });
            index += 1;
            continue;
        }

        if arg == "--" {
            options_ended = true;
            index += 1;
            continue;
        }

        let (name_part, inline_value) = split_inline_value(arg);
        let spec = resolve(name_part, index)?;
        let arity = spec.arity();
        let mut values = Vec::with_capacity(arity);

        if let Some(inline) = inline_value {
            match arity {
                0 => {
                    return Err(ParseError::UnexpectedValue {
                        as_written: name_part.to_string(),
                        index,
                    });
                }
                1 => values.push(inline.to_string()),
                wanted => {
                    return Err(ParseError::InlineValueNotAllowed {
                        as_written: name_part.to_string(),
                        wanted,
                        index,
                    });
                }
            }
        }

        // Values are taken positionally, even if they look like options. That is
        // what wkhtmltopdf does, and it is what makes `--margin-top -5mm` work.
        //
        // The cursor that walks the values is separate from the option's own
        // position, so that both the occurrence and any error report where the
        // *option* was written rather than where its last value landed.
        let mut value_index = index;
        while values.len() < arity {
            value_index += 1;
            match argv.get(value_index) {
                Some(value) => values.push(value.clone()),
                None => {
                    return Err(ParseError::MissingValues {
                        as_written: name_part.to_string(),
                        wanted: arity,
                        got: values.len(),
                        index,
                    });
                }
            }
        }

        tokens.push(Token::Opt(Occurrence {
            spec,
            as_written: name_part.to_string(),
            values,
            index,
        }));
        index = value_index + 1;
    }

    Ok(tokens)
}

/// `-` on its own is stdin, not an option.
fn looks_like_option(arg: &str) -> bool {
    arg.starts_with('-') && arg != "-"
}

/// Split `--name=value` into its two halves. Only the first `=` counts.
fn split_inline_value(arg: &str) -> (&str, Option<&str>) {
    match arg.find('=') {
        Some(position) => (&arg[..position], Some(&arg[position + 1..])),
        None => (arg, None),
    }
}

/// Look an argument up in the option table.
fn resolve(as_written: &str, index: usize) -> Result<&'static OptionSpec, ParseError> {
    let unknown = || ParseError::UnknownOption {
        as_written: as_written.to_string(),
        index,
    };

    if let Some(long) = as_written.strip_prefix("--") {
        return lookup_long(long).ok_or_else(unknown);
    }

    let short = as_written.strip_prefix('-').ok_or_else(unknown)?;
    let mut letters = short.chars();
    match (letters.next(), letters.next()) {
        (Some(letter), None) => lookup_short(letter).ok_or_else(unknown),
        // No clustering: `-qg` is reported as unknown rather than guessed at.
        _ => Err(unknown()),
    }
}

/// Second pass: attach options to objects and pick out the output.
fn assemble(tokens: Vec<Token>) -> Result<Tokenized, ParseError> {
    let positional_count = tokens
        .iter()
        .filter(|token| matches!(token, Token::Positional { .. }))
        .count();
    if positional_count < 2 {
        return Err(ParseError::NotEnoughArguments);
    }

    // The output is the last positional. Removing it first means the remaining
    // positionals form a clean sequence of objects.
    let last_positional = tokens
        .iter()
        .rposition(|token| matches!(token, Token::Positional { .. }))
        .expect("checked there are at least two positionals");

    let mut tokens = tokens;
    let Token::Positional {
        value: output_raw, ..
    } = tokens.remove(last_positional)
    else {
        unreachable!("index came from rposition over positionals")
    };
    let output = Output::classify(&output_raw);

    let mut globals = Vec::new();
    let mut defaults = Vec::new();
    let mut objects: Vec<Object> = Vec::new();
    // Set once `page` or `cover` has been seen and is waiting for its input.
    let mut pending_keyword: Option<(String, usize)> = None;

    for token in tokens {
        match token {
            Token::Opt(occurrence) => {
                // The global area ends at the first input. A `page`, `cover` or
                // `toc` keyword ends it too, even before its input has been
                // read: 0.12.6.1 does not diagnose a global option written
                // there, it mangles the line — `page --copies 2 in.html` sends
                // it off to load `http://2` — and refusing is the better of the
                // two ways to differ from that.
                let in_global_area = objects.is_empty() && pending_keyword.is_none();

                // Our own options carry no compatibility contract, so they are
                // accepted wherever they are written. `--dump-parse` in
                // particular is a debugging aid people append to a line they
                // have already typed, and making that an error would be a
                // strictness nobody asked for.
                let ours = occurrence.spec.support == Support::Extension;

                let bucket = match occurrence.spec.scope {
                    Scope::Global if ours || in_global_area => &mut globals,
                    Scope::Global => {
                        return Err(ParseError::WrongLocation {
                            as_written: occurrence.as_written,
                            belongs: Placement::GlobalArea,
                            index: occurrence.index,
                        });
                    }
                    Scope::Toc => match objects.last_mut() {
                        Some(object) if object.kind == ObjectKind::Toc => &mut object.options,
                        // Refused in the global area as well as after a page,
                        // which is what makes a TOC option something other than
                        // an object option with a longer name.
                        _ => {
                            return Err(ParseError::WrongLocation {
                                as_written: occurrence.as_written,
                                belongs: Placement::TocObject,
                                index: occurrence.index,
                            });
                        }
                    },
                    Scope::Object => match objects.last_mut() {
                        Some(object) => &mut object.options,
                        None => &mut defaults,
                    },
                };
                bucket.push(occurrence);
            }
            Token::Positional { value, index } => {
                if let Some((keyword, _)) = pending_keyword.take() {
                    let input = Input::classify(&value);
                    let kind = if keyword == "cover" {
                        ObjectKind::Cover(input)
                    } else {
                        ObjectKind::Page(input)
                    };
                    objects.push(Object {
                        kind,
                        options: Vec::new(),
                    });
                    continue;
                }

                match value.as_str() {
                    "page" | "cover" => pending_keyword = Some((value, index)),
                    "toc" => objects.push(Object {
                        kind: ObjectKind::Toc,
                        options: Vec::new(),
                    }),
                    _ => objects.push(Object {
                        kind: ObjectKind::Page(Input::classify(&value)),
                        options: Vec::new(),
                    }),
                }
            }
        }
    }

    if let Some((keyword, index)) = pending_keyword {
        return Err(ParseError::ObjectWithoutInput { keyword, index });
    }

    if objects.is_empty() {
        return Err(ParseError::NotEnoughArguments);
    }

    Ok(Tokenized {
        globals,
        defaults,
        objects,
        output,
    })
}
