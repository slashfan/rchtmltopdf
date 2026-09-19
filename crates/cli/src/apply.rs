//! Turning a parsed command line into settings.
//!
//! The tokenizer says what was written; this says what it means. Everything
//! downstream reads the settings model and never sees a command line, which is
//! the seam that lets the browser layer be driven from somewhere else entirely.
//!
//! # Understanding an option is not honouring it
//!
//! An option marked `Planned` is still translated here, and that is deliberate.
//! The settings model is where the browser layer's work lands, so filling it
//! ahead of that layer is how the two halves are built without either being
//! written blind. What makes it honest rather than a lie is that nothing
//! downstream reads the field, and that the binary warns the option is being
//! ignored — the warning lives there, once, because saying it twice would mean
//! two places to keep in step (D02).
//!
//! So nothing here is evidence that an option works. `tests/plan.rs` asks that
//! question, against what a conversion would actually do (D27).
//!
//! # Errors echo what the user typed
//!
//! A message naming `--margin-top` when the user wrote `-T` is a message about
//! somebody else's command line. These are read out of a thrown exception by a
//! developer who did not write the invocation by hand, so they quote the option
//! exactly as it appeared.

use crate::table::Support;
use crate::tokenizer::{Object, ObjectKind, Occurrence, Tokenized};
use rchtmltopdf_core::page_size::{self, Orientation};
use rchtmltopdf_core::settings::{
    Band, GlobalSettings, LogLevel, MediaType, ObjectKind as SettingsObjectKind, ObjectSettings,
    Pair, Settings,
};
use rchtmltopdf_core::units::Length;
use rchtmltopdf_core::{Input, LoadErrorHandling};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// A value that could not be understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyError {
    /// The option exactly as the user wrote it, `-T` or `--margin-top`.
    pub option: String,
    pub reason: String,
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.option, self.reason)
    }
}

impl std::error::Error for ApplyError {}

/// Whether this layer is the one that acts on an option.
///
/// Exists so a test can walk the whole option table and prove nothing marked as
/// working quietly falls through. Without it the table could claim anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// Acted on here.
    Here,
    /// Acted on somewhere else, by design: help, version, and the debugging flag.
    Elsewhere,
    /// Recognised and ignored, because it is not built yet or has no equivalent.
    NotYet,
}

/// Read a command line into settings.
pub fn apply(parsed: &Tokenized) -> Result<Settings, ApplyError> {
    let mut global = GlobalSettings {
        output: parsed.output.clone(),
        ..GlobalSettings::default()
    };
    let mut page = PageOverrides::default();

    for occurrence in &parsed.globals {
        apply_global(occurrence, &mut global, &mut page)?;
    }

    // Object options written before the first object are the defaults every
    // object starts from; its own options then override them.
    let mut inherited = ObjectSettings::page(Input::Stdin);
    for occurrence in &parsed.defaults {
        apply_object(occurrence, &mut inherited, &mut global)?;
    }

    let mut objects = Vec::with_capacity(parsed.objects.len());
    for object in &parsed.objects {
        objects.push(build_object(object, &inherited, &mut global)?);
    }

    page.resolve(&mut global);
    Ok(Settings { global, objects })
}

fn build_object(
    object: &Object,
    inherited: &ObjectSettings,
    global: &mut GlobalSettings,
) -> Result<ObjectSettings, ApplyError> {
    let (kind, input) = match &object.kind {
        ObjectKind::Page(input) => (SettingsObjectKind::Page, Some(input.clone())),
        ObjectKind::Cover(input) => (SettingsObjectKind::Cover, Some(input.clone())),
        ObjectKind::Toc => (SettingsObjectKind::Toc, None),
    };

    let mut settings = ObjectSettings {
        kind,
        input,
        ..inherited.clone()
    };
    // A cover "does not have headers and footers", in wkhtmltopdf's words: the
    // bands every object inherits are cleared before its own options are read,
    // so a footer given before the first input decorates the pages and not the
    // cover, while one written after `cover` still applies to it.
    if kind == SettingsObjectKind::Cover {
        settings.header = Band::default();
        settings.footer = Band::default();
        // "The page does not appear in the table of contents", nor in the
        // outline it is built from.
        settings.in_outline = false;
    }
    for occurrence in &object.options {
        apply_object(occurrence, &mut settings, global)?;
    }
    Ok(settings)
}

/// Paper set piecemeal, resolved once everything has been read.
///
/// Public so the exhaustiveness test can drive a single option in isolation.
///
/// An explicit width or height wins over a named size however they were ordered,
/// so `--page-size A4 --page-width 100mm` and the reverse agree.
#[derive(Debug, Default)]
pub struct PageOverrides {
    pub width: Option<Length>,
    pub height: Option<Length>,
}

impl PageOverrides {
    fn resolve(&self, global: &mut GlobalSettings) {
        if let Some(width) = self.width {
            global.page.size.width = width;
        }
        if let Some(height) = self.height {
            global.page.size.height = height;
        }
    }
}

pub fn apply_global(
    occurrence: &Occurrence,
    global: &mut GlobalSettings,
    page: &mut PageOverrides,
) -> Result<Applied, ApplyError> {
    if let Some(applied) = answered_elsewhere(occurrence) {
        return Ok(applied);
    }

    match occurrence.spec.long {
        "page-size" => {
            let name = value(occurrence);
            global.page.size = page_size::lookup(name).ok_or_else(|| {
                fail(
                    occurrence,
                    format!(
                        "unknown page size `{name}`; expected one of: {}",
                        page_size::all_names().collect::<Vec<_>>().join(", ")
                    ),
                )
            })?;
        }
        "page-width" => page.width = Some(length(occurrence)?),
        "page-height" => page.height = Some(length(occurrence)?),
        // Written, and remembered as written: an HTML band replaces a margin
        // that was defaulted and fits into one that was named.
        "margin-top" => {
            global.page.margins.top = length(occurrence)?;
            global.page.named.top = true;
        }
        "margin-right" => global.page.margins.right = length(occurrence)?,
        "margin-bottom" => {
            global.page.margins.bottom = length(occurrence)?;
            global.page.named.bottom = true;
        }
        "margin-left" => global.page.margins.left = length(occurrence)?,
        "orientation" => {
            let name = value(occurrence);
            global.page.orientation = Orientation::parse(name).ok_or_else(|| {
                fail(
                    occurrence,
                    format!("unknown orientation `{name}`; expected Portrait or Landscape"),
                )
            })?;
        }
        "title" => global.title = Some(value(occurrence).to_string()),
        "quiet" => global.log_level = LogLevel::None,
        "log-level" => {
            let name = value(occurrence);
            global.log_level = LogLevel::parse(name).ok_or_else(|| {
                fail(
                    occurrence,
                    format!("unknown log level `{name}`; expected none, error, warn or info"),
                )
            })?;
        }
        "timeout" => {
            let seconds: u64 = number(occurrence)?;
            // Nought asks for no limit at all, which is what wkhtmltopdf had.
            global.timeout = (seconds > 0).then(|| Duration::from_secs(seconds));
        }
        "chromium-path" => global.browser.path = Some(PathBuf::from(value(occurrence))),
        "chromium-arg" => global
            .browser
            .extra_args
            .push(value(occurrence).to_string()),
        "no-sandbox" => global.browser.no_sandbox = true,
        "tagged-pdf" => global.tagged_pdf = true,

        // --- the outline ------------------------------------------------------
        "outline" => global.outline.enabled = true,
        "no-outline" => global.outline.enabled = false,
        "outline-depth" => global.outline.depth = number(occurrence)?,
        "dump-outline" => global.outline.dump = Some(PathBuf::from(value(occurrence))),

        // Answered before any of this runs.
        "dump-parse" => return Ok(Applied::Elsewhere),
        _ => return Ok(Applied::NotYet),
    }

    Ok(Applied::Here)
}

/// Read one option written on an object, or before the first one.
///
/// `global` is here for `--page-offset` alone: wkhtmltopdf lists it among the
/// page options and keeps it in `PdfGlobal`, so it may be written anywhere and
/// the last one written wins for the whole output (D51). Every other option on
/// this line belongs to the object it follows.
pub fn apply_object(
    occurrence: &Occurrence,
    object: &mut ObjectSettings,
    global: &mut GlobalSettings,
) -> Result<Applied, ApplyError> {
    if let Some(applied) = answered_elsewhere(occurrence) {
        return Ok(applied);
    }

    let load = &mut object.load;
    let web = &mut object.web;

    match occurrence.spec.long {
        // --- what the page may do -------------------------------------------
        "enable-javascript" => web.javascript = true,
        "disable-javascript" => web.javascript = false,
        "images" => web.images = true,
        "no-images" => web.images = false,
        "background" => web.background = true,
        "no-background" => web.background = false,
        "print-media-type" => web.media_type = MediaType::Print,
        "no-print-media-type" => web.media_type = MediaType::Screen,
        "zoom" => web.zoom = number(occurrence)?,
        "encoding" => web.encoding = Some(value(occurrence).to_string()),
        "minimum-font-size" => web.minimum_font_size = Some(number(occurrence)?),
        "viewport-size" => web.viewport = viewport(occurrence)?,

        // --- reaching the document -------------------------------------------
        "username" => web.username = Some(value(occurrence).to_string()),
        "password" => web.password = Some(value(occurrence).to_string()),
        "proxy" => web.proxy = Some(value(occurrence).to_string()),
        "cookie" => web.cookies.push(pair(occurrence)),
        "custom-header" => web.custom_headers.push(pair(occurrence)),
        "custom-header-propagation" => web.propagate_custom_headers = true,
        "no-custom-header-propagation" => web.propagate_custom_headers = false,

        // --- the outline ------------------------------------------------------
        "include-in-outline" => object.in_outline = true,
        "exclude-from-outline" => object.in_outline = false,

        // One number for the whole output, wherever it was written (D51).
        "page-offset" => global.page_offset = number(occurrence)?,

        // --- the table of contents, all of it CSS in the generated document ---
        "toc-header-text" => object.toc.header_text = value(occurrence).to_string(),
        "toc-level-indentation" => object.toc.level_indentation = value(occurrence).to_string(),
        "toc-text-size-shrink" => object.toc.text_size_shrink = number(occurrence)?,
        "disable-dotted-lines" => object.toc.dotted_lines = false,
        "disable-toc-links" => object.toc.links = false,
        "enable-toc-back-links" => object.toc.back_links = true,
        "disable-toc-back-links" => object.toc.back_links = false,

        // --- the browser's own plumbing --------------------------------------
        "bypass-proxy-for" => web.bypass_proxy_for.push(value(occurrence).to_string()),
        "cache-dir" => web.cache_dir = Some(PathBuf::from(value(occurrence))),

        // --- links --------------------------------------------------------------
        "enable-external-links" => object.links.external = true,
        "disable-external-links" => object.links.external = false,
        "enable-internal-links" => object.links.internal = true,
        "disable-internal-links" => object.links.internal = false,
        "resolve-relative-links" => object.links.resolve_relative = true,
        "keep-relative-links" => object.links.resolve_relative = false,

        // --- reaching the filesystem (D10) -----------------------------------
        "enable-local-file-access" => web.local_file_access = true,
        "disable-local-file-access" => web.local_file_access = false,
        "allow" => web.allowed_paths.push(PathBuf::from(value(occurrence))),

        // --- when it is finished ----------------------------------------------
        "javascript-delay" => load.javascript_delay = Duration::from_millis(number(occurrence)?),
        "window-status" => load.window_status = Some(value(occurrence).to_string()),
        "run-script" => load.run_scripts.push(value(occurrence).to_string()),
        "user-style-sheet" => load.user_style_sheet = file(occurrence),
        "no-stop-slow-scripts" => load.stop_slow_scripts = false,
        "load-error-handling" => load.on_document_error = handling(occurrence)?,
        "load-media-error-handling" => load.on_media_error = handling(occurrence)?,

        // --- headers and footers ----------------------------------------------
        "header-left" => object.header.left = Some(value(occurrence).to_string()),
        "header-center" => object.header.center = Some(value(occurrence).to_string()),
        "header-right" => object.header.right = Some(value(occurrence).to_string()),
        "header-font-name" => object.header.font_name = Some(value(occurrence).to_string()),
        "header-font-size" => object.header.font_size = Some(number(occurrence)?),
        "header-spacing" => object.header.spacing = Some(number(occurrence)?),
        "header-line" => object.header.line = true,
        "no-header-line" => object.header.line = false,
        "header-html" => object.header.html = file(occurrence),
        "footer-left" => object.footer.left = Some(value(occurrence).to_string()),
        "footer-center" => object.footer.center = Some(value(occurrence).to_string()),
        "footer-right" => object.footer.right = Some(value(occurrence).to_string()),
        "footer-font-name" => object.footer.font_name = Some(value(occurrence).to_string()),
        "footer-font-size" => object.footer.font_size = Some(number(occurrence)?),
        "footer-spacing" => object.footer.spacing = Some(number(occurrence)?),
        "footer-line" => object.footer.line = true,
        "no-footer-line" => object.footer.line = false,
        "footer-html" => object.footer.html = file(occurrence),
        "replace" => object.replacements.push(pair(occurrence)),
        "default-header" => object.header = default_header(),

        _ => return Ok(Applied::NotYet),
    }

    Ok(Applied::Here)
}

/// wkhtmltopdf's shorthand: the document's name on the left, the page number on
/// the right, and a rule under both.
fn default_header() -> Band {
    Band {
        left: Some("[webpage]".to_string()),
        right: Some("[page]/[topage]".to_string()),
        line: true,
        ..Band::default()
    }
}

/// Options answered before this layer runs at all.
///
/// `Planned` is deliberately not here, and that is the whole of the difference
/// between understanding an option and honouring it. An option the command line
/// understands but the conversion does not act on yet still fills the settings
/// model: the model is where the next layer's work lands, and leaving it empty
/// until the browser catches up would mean writing both halves blind. What keeps
/// the warning honest is that nothing downstream reads the field, which
/// `tests/plan.rs` holds rather than takes on trust (D27).
fn answered_elsewhere(occurrence: &Occurrence) -> Option<Applied> {
    match occurrence.spec.support {
        Support::Meta => Some(Applied::Elsewhere),
        _ => None,
    }
}

fn fail(occurrence: &Occurrence, reason: impl Into<String>) -> ApplyError {
    ApplyError {
        option: occurrence.as_written.clone(),
        reason: reason.into(),
    }
}

/// The value of an option that names a file to read, or nothing when it is
/// empty (D59).
///
/// `--header-html ""` is what a template engine writes when the document has
/// no header, and wkhtmltopdf ignores it: measured on 0.12.6.1, `--header-html
/// ""`, `--footer-html ""` and `--user-style-sheet ""` all convert and exit 0.
/// Read literally, the empty string is a file that is not there, which fails
/// the conversion over an option the caller did not mean to set.
///
/// A document is not in this family. `cover ""` is an object with nowhere to
/// load from, and both binaries refuse it — so this is about options that name
/// a file to read, not about every empty value.
fn file(occurrence: &Occurrence) -> Option<String> {
    match value(occurrence) {
        "" => None,
        path => Some(path.to_string()),
    }
}

fn value(occurrence: &Occurrence) -> &str {
    occurrence.values.first().map_or("", String::as_str)
}

fn pair(occurrence: &Occurrence) -> Pair {
    Pair {
        name: occurrence.values.first().cloned().unwrap_or_default(),
        value: occurrence.values.get(1).cloned().unwrap_or_default(),
    }
}

fn length(occurrence: &Occurrence) -> Result<Length, ApplyError> {
    // Reuses the parser's own message, which already names the units it accepts.
    Length::parse(value(occurrence)).map_err(|error| fail(occurrence, error.to_string()))
}

fn number<T>(occurrence: &Occurrence) -> Result<T, ApplyError>
where
    T: std::str::FromStr,
{
    let raw = value(occurrence);
    raw.trim()
        .parse()
        .map_err(|_| fail(occurrence, format!("`{raw}` is not a number")))
}

fn handling(occurrence: &Occurrence) -> Result<LoadErrorHandling, ApplyError> {
    let raw = value(occurrence);
    LoadErrorHandling::parse(raw).ok_or_else(|| {
        fail(
            occurrence,
            format!("unknown handler `{raw}`; expected abort, ignore or skip"),
        )
    })
}

/// `--viewport-size` is written the way wkhtmltopdf writes it, `1024x768`.
fn viewport(occurrence: &Occurrence) -> Result<(u32, u32), ApplyError> {
    let raw = value(occurrence);
    let (width, height) = raw.split_once(['x', 'X']).ok_or_else(|| {
        fail(
            occurrence,
            format!("`{raw}` is not a size; expected 1024x768"),
        )
    })?;

    let parse = |part: &str| -> Result<u32, ApplyError> {
        part.trim().parse().map_err(|_| {
            fail(
                occurrence,
                format!("`{raw}` is not a size; expected 1024x768"),
            )
        })
    };
    Ok((parse(width)?, parse(height)?))
}
