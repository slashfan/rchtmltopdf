//! Running a conversion.
//!
//! Everything above this has turned a command line into settings; everything
//! below takes settings and knows nothing about how they were written. This is
//! the seam, and it is short on purpose.

use crate::{PROGRAM, input, output};
use rchtmltopdf_browser::Browser;
use rchtmltopdf_browser::deadline;
use rchtmltopdf_browser::intercept;
use rchtmltopdf_browser::locate::{SystemEnvironment, locate};
use rchtmltopdf_browser::placeholder::Clock;
use rchtmltopdf_browser::plan::{self, Plan};
use rchtmltopdf_browser::render::Progress;
use rchtmltopdf_core::settings::{ObjectKind, Settings};
use rchtmltopdf_core::{ExitCode, LoadErrorHandling};
use std::fmt;

#[derive(Debug)]
pub enum ConvertError {
    /// Recognised, and not built yet. Said plainly rather than done partly.
    Unsupported(String),
    Input(input::InputError),
    Output(output::OutputError),
    Browser(rchtmltopdf_browser::Error),
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertError::Unsupported(what) => write!(f, "{what}"),
            ConvertError::Input(error) => write!(f, "{error}"),
            ConvertError::Output(error) => write!(f, "{error}"),
            ConvertError::Browser(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ConvertError {}

impl From<input::InputError> for ConvertError {
    fn from(error: input::InputError) -> Self {
        ConvertError::Input(error)
    }
}

impl From<output::OutputError> for ConvertError {
    fn from(error: output::OutputError) -> Self {
        ConvertError::Output(error)
    }
}

impl From<rchtmltopdf_browser::Error> for ConvertError {
    fn from(error: rchtmltopdf_browser::Error) -> Self {
        ConvertError::Browser(error)
    }
}

/// Convert, or say why not.
///
/// Returns an exit code rather than nothing, because "wrote the document and
/// still failed" is a real outcome and D14 turns on it: a subresource that fails
/// under `--load-media-error-handling abort` produces the PDF *and* exits 1.
/// Collapsing that into `Err` would throw the document away, and collapsing it
/// into `Ok` would lose the exit code KnpSnappy reads.
pub async fn convert(settings: &Settings) -> Result<ExitCode, ConvertError> {
    let object = only_page(settings)?;
    let source = object
        .input
        .as_ref()
        .ok_or_else(|| ConvertError::Unsupported("this object has no document to read".into()))?;

    // Resolved before a browser is started, so a missing file fails in a
    // millisecond rather than after a launch. Held for the whole conversion: a
    // document read from standard input lives in a file that goes away when this
    // is dropped, on every path out including the deadline.
    let document = input::resolve(source)?;

    let executable = locate(settings.global.browser.path.as_deref(), &SystemEnvironment)?;

    // Everything the settings decide, decided in one place before any of it
    // happens. The page halves are rebuilt from the same functions below rather
    // than passed down, so a test can hold the option table to what a conversion
    // would actually do (D27).
    let plan = Plan::new(&settings.global, object, Clock::now(), document.url());

    // Two options that cannot always be applied, said before a browser starts
    // because both are facts about the command line and the document rather than
    // about the rendering.
    if settings.global.log_level.shows_warnings() {
        if object.web.encoding.is_some() && plan.requests.serve_as.is_none() {
            eprintln!(
                "{PROGRAM}: warning: --encoding does not apply to a document fetched over the \
                 network; it is read as the server said it should be"
            );
        }
        if !object.web.cookies.is_empty() && !plan::takes_cookies(document.url()) {
            eprintln!(
                "{PROGRAM}: warning: --cookie needs a document fetched over http or https; \
                 a local document has no origin to scope a cookie to, so it is being ignored"
            );
        }
    }

    // Said before the browser starts, because it is a fact about the command
    // line rather than about the document.
    if settings.global.log_level.shows_warnings() {
        for name in &plan.unsupported_placeholders {
            eprintln!(
                "{PROGRAM}: warning: [{name}] names a position in the document outline, which is \
                 not built yet (planned for V2); it is being left empty"
            );
        }
    }

    let progress = Progress::new();
    // The browser is created inside the deadline, so expiry drops it and its Drop
    // stops the process group and removes the profile. Cleanup is not a step that
    // could be skipped.
    let (pdf, refused, report) = deadline::within(plan.deadline, &progress, async {
        let browser = Browser::launch(&executable, &plan.launch).await?;
        let page = browser.new_page().await?;

        // Before anything is fetched, the document included: the policy has to
        // be in place for the first request, not the second (D10).
        let policing = intercept::install(page.session(), plan.requests.clone()).await?;

        page.prepare(&plan.prepare).await?;
        say(settings, "Loading page (1/2)");
        let report = page.load(document.url(), &object.load, &progress).await?;

        // Before printing, not after. A rejected password leaves the server's
        // own 401 body as the response, which renders perfectly well and is not
        // the document anybody asked for (D14).
        if policing
            .as_ref()
            .is_some_and(rchtmltopdf_browser::Interception::credentials_rejected)
        {
            return Err(rchtmltopdf_browser::Error::Credentials {
                url: document.url().to_string(),
            });
        }

        // The document itself. `--load-error-handling` decides, and only
        // `ignore` prints anything at all: D14 is explicit that a main document
        // which failed means exit 1 and no PDF.
        if let Some(failed) = &report.document {
            match object.load.on_document_error {
                LoadErrorHandling::Ignore => {}
                LoadErrorHandling::Abort => {
                    return Err(rchtmltopdf_browser::Error::Navigation {
                        url: failed.url.clone(),
                        reason: failed.error.name().to_string(),
                    });
                }
                // `skip` drops the failing object and carries on, and with one
                // object there is nothing to carry on to. Saying so is better
                // than writing an empty document; V2 is where it starts to mean
                // something.
                LoadErrorHandling::Skip => {
                    return Err(rchtmltopdf_browser::Error::Navigation {
                        url: failed.url.clone(),
                        reason: format!(
                            "{} (--load-error-handling skip leaves nothing to convert while \
                             there is one document)",
                            failed.error.name()
                        ),
                    });
                }
            }
        }
        say(settings, "Printing pages (2/2)");
        let pdf = page.print_to_pdf(&plan.print).await?;

        // Read before the guard is dropped, which is what stops interception.
        let refused = policing
            .as_ref()
            .map(intercept::Interception::refused)
            .unwrap_or_default();

        // Asked to leave rather than killed, so it can finish writing.
        browser.close().await?;
        Ok((pdf, refused, report))
    })
    .await?;

    // A document that renders with a stylesheet missing is the hardest kind of
    // failure to diagnose, because it renders. Said on stderr, where it cannot
    // corrupt a PDF written to stdout.
    if settings.global.log_level.shows_warnings() {
        for refusal in &refused {
            eprintln!("{PROGRAM}: warning: {refusal}");
        }
    }

    // Written before the media errors are judged, because D14 says a subresource
    // that failed under `abort` produces the document *and* exits 1. A wrapper
    // that raises on the exit code still has the PDF to look at.
    output::write(&settings.global.output, &pdf)?;

    let outcome = match (object.load.on_media_error, report.media.as_slice()) {
        (_, []) | (LoadErrorHandling::Ignore, _) => ExitCode::Success,
        (LoadErrorHandling::Skip, failures) => {
            for failed in failures {
                report_media(settings, failed);
            }
            ExitCode::Success
        }
        (LoadErrorHandling::Abort, failures) => {
            for failed in failures {
                report_media(settings, failed);
            }
            // wkhtmltopdf's exact wording, and not prefixed with the program
            // name: applications grep for this line.
            println_stderr(&format!(
                "Exit with code 1 due to network error: {}",
                failures[0].error.name()
            ));
            ExitCode::Failure
        }
    };

    say(settings, "Done");
    Ok(outcome)
}

/// A progress line, in wkhtmltopdf's shape and on its stream.
fn say(settings: &Settings, line: &str) {
    if settings.global.log_level.shows_progress() {
        eprintln!("{line}");
    }
}

/// One failed subresource, named the way wkhtmltopdf names it.
fn report_media(settings: &Settings, failed: &rchtmltopdf_browser::render::Failed) {
    if settings.global.log_level.shows_warnings() {
        eprintln!("{PROGRAM}: warning: {failed} was not loaded");
    }
}

/// Straight to stderr whatever the level, because this line is a contract.
fn println_stderr(line: &str) {
    eprintln!("{line}");
}

/// V0 converts one page.
///
/// Anything else is refused by name. Converting the first of several and saying
/// nothing would produce a document that looks right and is missing most of
/// itself, which is the worst outcome available.
fn only_page(
    settings: &Settings,
) -> Result<&rchtmltopdf_core::settings::ObjectSettings, ConvertError> {
    if settings.objects.len() > 1 {
        return Err(ConvertError::Unsupported(format!(
            "{} documents were given, and only one is supported so far (several are planned for V2)",
            settings.objects.len()
        )));
    }

    let object = settings
        .objects
        .first()
        .ok_or_else(|| ConvertError::Unsupported("no document to convert".into()))?;

    match object.kind {
        ObjectKind::Page => Ok(object),
        ObjectKind::Cover => Err(ConvertError::Unsupported(
            "a cover page is not supported yet (planned for V2)".into(),
        )),
        ObjectKind::Toc => Err(ConvertError::Unsupported(
            "a table of contents is not supported yet (planned for V3)".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchtmltopdf_core::Input;
    use rchtmltopdf_core::settings::ObjectSettings;

    fn with(objects: Vec<ObjectSettings>) -> Settings {
        Settings {
            objects,
            ..Settings::default()
        }
    }

    fn page() -> ObjectSettings {
        ObjectSettings::page(Input::classify("a.html"))
    }

    #[test]
    fn one_page_is_what_v0_converts() {
        assert!(only_page(&with(vec![page()])).is_ok());
    }

    /// Converting the first of several and saying nothing would produce a
    /// document that looks right and is missing most of itself.
    #[test]
    fn several_documents_are_refused_by_name_not_truncated() {
        let error = only_page(&with(vec![page(), page(), page()])).unwrap_err();
        let message = error.to_string();
        assert!(message.contains('3'), "should say how many: {message}");
        assert!(message.contains("V2"), "should say when: {message}");
    }

    #[test]
    fn a_cover_and_a_table_of_contents_say_which_milestone_they_wait_for() {
        let mut cover = page();
        cover.kind = ObjectKind::Cover;
        assert!(
            only_page(&with(vec![cover]))
                .unwrap_err()
                .to_string()
                .contains("V2")
        );

        let mut toc = page();
        toc.kind = ObjectKind::Toc;
        assert!(
            only_page(&with(vec![toc]))
                .unwrap_err()
                .to_string()
                .contains("V3")
        );
    }

    #[test]
    fn nothing_to_convert_is_reported_rather_than_panicking() {
        assert!(only_page(&with(vec![])).is_err());
    }
}
