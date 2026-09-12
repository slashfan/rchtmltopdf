//! Scaffolding shared by the tests that walk the whole option table.
//!
//! Both `apply.rs` and `plan.rs` have to write a command line for an option they
//! know nothing about beyond its entry in the table. That needs two things: a
//! plausible value for every placeholder the table uses, and somewhere on the
//! line the grammar will accept the option (D26).

#![allow(dead_code)]

use rchtmltopdf::table::{OptionSpec, Scope};

/// Split a command line the way a shell would, honouring single quotes.
pub fn split(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut started) = (false, false);
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

/// A plausible value for each placeholder the table uses, so an occurrence can
/// be built for any option without knowing what it does.
///
/// **Every one of these differs from the default it would overwrite**, and that
/// is load-bearing rather than tidy. `plan.rs` asks whether an option changes
/// what a conversion does; `--page-size A4` and `--margin-top 10mm` ask for
/// exactly what they would have got anyway, so a value chosen carelessly here
/// makes an implemented option look inert.
pub fn placeholder(spec: &OptionSpec, name: &str) -> &'static str {
    // Where the placeholder name is ambiguous, the option wins: two options call
    // their value "size" and want different shapes, and "level" is a number for
    // one option and a word for another.
    match spec.long {
        // Not 1024x768: that is the default, and asking for it proves nothing.
        "viewport-size" => return "1280x1024",
        // Not A4, which is the default, and not info, which is the default.
        "page-size" => return "Letter",
        "log-level" => return "error",
        _ => {}
    }

    match name {
        // Not 10mm: that is the default margin, and asking for it proves nothing.
        "unitreal" | "width" => "7mm",
        "orientation" => "Landscape",
        "handler" => "abort",
        "msec" | "int" | "integer" | "number" | "offset" | "seconds" | "dpi" | "size" | "level" => {
            "10"
        }
        "float" | "real" => "1.5",
        "encoding" => "UTF-8",
        "url" | "file" => "https://example.com/a",
        "path" => "/tmp",
        "js" => "void 0",
        "proxy" => "http://127.0.0.1:8080",
        // name, value, text, username, password, windowStatus, arg: a bare word
        // is valid for all of them, and for anything added later.
        _ => "x",
    }
}

/// One option as it would be written, with a value for each placeholder.
pub fn written(spec: &OptionSpec) -> Vec<String> {
    let mut words = vec![format!("--{}", spec.long)];
    words.extend(
        spec.value_names
            .iter()
            .map(|name| placeholder(spec, name).to_string()),
    );
    words
}

/// A whole command line writing the given options, each somewhere the grammar
/// allows it.
///
/// The three placement rules are D26's: a global option before the first input,
/// an object option anywhere, a table-of-contents option only after a `toc`
/// object. A line needing the third cannot also be a page, because a `toc`
/// object has no document of its own.
pub fn line(options: &[&'static OptionSpec]) -> Vec<String> {
    let (mut globals, mut object, mut toc) = (Vec::new(), Vec::new(), Vec::new());
    for spec in options {
        let words = written(spec);
        match spec.scope {
            Scope::Global => globals.extend(words),
            Scope::Object => object.extend(words),
            Scope::Toc => toc.extend(words),
        }
    }

    let mut args = globals;
    args.push(if toc.is_empty() { "in.html" } else { "toc" }.to_string());
    args.extend(object);
    args.extend(toc);
    args.push("out.pdf".to_string());
    args
}
