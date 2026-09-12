//! Running a conversion.
//!
//! Everything above this has turned a command line into settings; everything
//! below takes settings and knows nothing about how they were written. This is
//! the seam, and it is short on purpose.
//!
//! # Several documents
//!
//! Each is loaded and printed on its own page of one browser, in command line
//! order, and the printed documents are combined afterwards (#36). One browser
//! rather than one per document (D11), restarted only when a document asks for
//! something that is decided on the browser's command line rather than over the
//! protocol: a proxy, a minimum font size, images off. Two documents that share
//! those share a browser; two that differ each get one, and the result is the
//! same either way.
//!
//! Every input is resolved before any browser starts, so a missing file at the
//! end of a ten document command line fails in a millisecond rather than after
//! nine conversions.

use crate::{PROGRAM, VERSION, input, output};
use rchtmltopdf_browser::Browser;
use rchtmltopdf_browser::LaunchOptions;
use rchtmltopdf_browser::clock;
use rchtmltopdf_browser::deadline;
use rchtmltopdf_browser::intercept;
use rchtmltopdf_browser::locate::{SystemEnvironment, locate};
use rchtmltopdf_browser::plan::{self, Plan};
use rchtmltopdf_browser::render::{Failed, Progress};
use rchtmltopdf_core::settings::{ObjectKind, ObjectSettings, Settings};
use rchtmltopdf_core::{ExitCode, Input, LoadErrorHandling};
use std::fmt;

#[derive(Debug)]
pub enum ConvertError {
    /// Recognised, and not built yet. Said plainly rather than done partly.
    Unsupported(String),
    /// Every document was dropped by `--load-error-handling skip`, so there is
    /// nothing to write. One entry per document, as `could not load` lines.
    NothingLeft(Vec<String>),
    Input(input::InputError),
    Output(output::OutputError),
    Browser(rchtmltopdf_browser::Error),
    Pdf(rchtmltopdf_pdf::Error),
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertError::Unsupported(what) => write!(f, "{what}"),
            ConvertError::NothingLeft(failures) => write!(
                f,
                "--load-error-handling skip left nothing to convert: {}",
                failures.join("; ")
            ),
            ConvertError::Input(error) => write!(f, "{error}"),
            ConvertError::Output(error) => write!(f, "{error}"),
            ConvertError::Browser(error) => write!(f, "{error}"),
            ConvertError::Pdf(error) => write!(f, "{error}"),
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

impl From<rchtmltopdf_pdf::Error> for ConvertError {
    fn from(error: rchtmltopdf_pdf::Error) -> Self {
        ConvertError::Pdf(error)
    }
}

impl From<rchtmltopdf_browser::Error> for ConvertError {
    fn from(error: rchtmltopdf_browser::Error) -> Self {
        ConvertError::Browser(error)
    }
}

/// What came out of the browser for the documents that made it.
struct Printed {
    /// One PDF per document that was printed, in command line order.
    documents: Vec<Vec<u8>>,
    /// Subresources the interception refused, across every document.
    refused: Vec<intercept::Refusal>,
    /// Subresources that failed, paired with the object whose
    /// `--load-media-error-handling` judges them.
    media: Vec<(usize, Vec<Failed>)>,
    /// Documents `--load-error-handling skip` dropped, as `could not load` lines.
    skipped: Vec<String>,
}

/// Convert, or say why not.
///
/// Returns an exit code rather than nothing, because "wrote the document and
/// still failed" is a real outcome and D14 turns on it: a subresource that fails
/// under `--load-media-error-handling abort` produces the PDF *and* exits 1.
/// Collapsing that into `Err` would throw the document away, and collapsing it
/// into `Ok` would lose the exit code KnpSnappy reads.
pub async fn convert(settings: &Settings) -> Result<ExitCode, ConvertError> {
    let objects = pages(settings)?;

    // Resolved before a browser is started, so a missing file fails in a
    // millisecond rather than after a launch. Held for the whole conversion: a
    // document read from standard input lives in a file that goes away when this
    // is dropped, on every path out including the deadline.
    let mut documents = Vec::with_capacity(objects.len());
    for object in &objects {
        let source = object.input.as_ref().ok_or_else(|| {
            ConvertError::Unsupported("this object has no document to read".into())
        })?;
        documents.push(input::resolve(source)?);
    }

    let executable = locate(settings.global.browser.path.as_deref(), &SystemEnvironment)?;

    // Everything the settings decide, decided in one place before any of it
    // happens. The page halves are rebuilt from the same functions below rather
    // than passed down, so a test can hold the option table to what a conversion
    // would actually do (D27).
    //
    // One clock for every document: a `[date]` in the first footer and one in
    // the last must agree, whatever midnight does in between.
    let now = clock::now();
    let plans: Vec<Plan> = objects
        .iter()
        .zip(&documents)
        .map(|(object, document)| Plan::new(&settings.global, object, now, document.url()))
        .collect();

    // Facts about the command line and the documents rather than about the
    // rendering, so they are said before a browser starts. Once each: ten
    // documents with the same `--encoding` are one warning, not ten.
    if settings.global.log_level.shows_warnings() {
        let mut said: Vec<String> = Vec::new();
        let mut warn = |line: String| {
            if !said.contains(&line) {
                eprintln!("{PROGRAM}: warning: {line}");
                said.push(line);
            }
        };
        for ((object, document), plan) in objects.iter().zip(&documents).zip(&plans) {
            if object.web.encoding.is_some() && plan.requests.serve_as.is_none() {
                warn(
                    "--encoding does not apply to a document fetched over the network; it is \
                     read as the server said it should be"
                        .into(),
                );
            }
            if !object.web.cookies.is_empty() && !plan::takes_cookies(document.url()) {
                warn(
                    "--cookie needs a document fetched over http or https; a local document \
                     has no origin to scope a cookie to, so it is being ignored"
                        .into(),
                );
            }
            for name in &plan.unsupported_placeholders {
                warn(format!(
                    "[{name}] names a position in the document outline, which is not built \
                     yet (planned for V2); it is being left empty"
                ));
            }
        }
    }

    let progress = Progress::new();
    let total = objects.len();
    // The browser is created inside the deadline, so expiry drops it and its Drop
    // stops the process group and removes the profile. Cleanup is not a step that
    // could be skipped.
    let printed = deadline::within(settings.global.timeout, &progress, async {
        let mut printed = Printed {
            documents: Vec::with_capacity(total),
            refused: Vec::new(),
            media: Vec::new(),
            skipped: Vec::new(),
        };
        let mut browser: Option<Browser> = None;
        let mut running_with: Option<LaunchOptions> = None;

        for (index, ((object, document), plan)) in
            objects.iter().zip(&documents).zip(&plans).enumerate()
        {
            // One browser for the conversion, restarted only when this document
            // needs one started differently. Closed rather than dropped, so it
            // can finish writing its profile away.
            if running_with.as_ref() != Some(&plan.launch) {
                if let Some(previous) = browser.take() {
                    previous.close().await?;
                }
                browser = Some(Browser::launch(&executable, &plan.launch).await?);
                running_with = Some(plan.launch.clone());
            }
            let browser = browser.as_ref().expect("launched just above");

            say(settings, &loading_line(index, total));
            let page = browser.new_page().await?;

            // Before anything is fetched, the document included: the policy has
            // to be in place for the first request, not the second (D10).
            let policing = intercept::install(page.session(), plan.requests.clone()).await?;

            page.prepare(&plan.prepare).await?;
            let report = page.load(document.url(), &object.load, &progress).await?;

            // Before printing, not after. A rejected password leaves the
            // server's own 401 body as the response, which renders perfectly
            // well and is not the document anybody asked for (D14).
            if policing
                .as_ref()
                .is_some_and(rchtmltopdf_browser::Interception::credentials_rejected)
            {
                return Err(rchtmltopdf_browser::Error::Credentials {
                    url: document.url().to_string(),
                });
            }

            // The document itself. `--load-error-handling` decides, and only
            // `ignore` prints anything at all: D14 is explicit that a main
            // document which failed means exit 1 and no PDF.
            if let Some(failed) = &report.document {
                match object.load.on_document_error {
                    LoadErrorHandling::Ignore => {}
                    LoadErrorHandling::Abort => {
                        return Err(rchtmltopdf_browser::Error::Navigation {
                            url: failed.url.clone(),
                            reason: failed.error.name().to_string(),
                        });
                    }
                    // `skip` drops the failing document and carries on with the
                    // others. Said now rather than at the end, in wkhtmltopdf's
                    // words, so the line sits next to the load it belongs to.
                    LoadErrorHandling::Skip => {
                        let line =
                            format!("could not load {}: {}", failed.url, failed.error.name());
                        if settings.global.log_level.shows_warnings() {
                            eprintln!(
                                "{PROGRAM}: warning: failed loading page {} (skipped)",
                                failed.url
                            );
                        }
                        printed.skipped.push(line);
                        continue;
                    }
                }
            }

            printed
                .documents
                .push(page.print_to_pdf(&plan.print).await?);

            // Read before the guard is dropped, which is what stops interception.
            printed.refused.extend(
                policing
                    .as_ref()
                    .map(intercept::Interception::refused)
                    .unwrap_or_default(),
            );
            printed.media.push((index, report.media));
        }

        // Asked to leave rather than killed, so it can finish writing.
        if let Some(browser) = browser {
            browser.close().await?;
        }
        Ok(printed)
    })
    .await?;

    // A document that renders with a stylesheet missing is the hardest kind of
    // failure to diagnose, because it renders. Said on stderr, where it cannot
    // corrupt a PDF written to stdout.
    if settings.global.log_level.shows_warnings() {
        for refusal in &printed.refused {
            eprintln!("{PROGRAM}: warning: {refusal}");
        }
    }

    if printed.documents.is_empty() {
        return Err(ConvertError::NothingLeft(printed.skipped));
    }

    say(settings, "Printing pages (2/2)");
    let parts: Vec<&[u8]> = printed.documents.iter().map(Vec::as_slice).collect();
    let pdf = rchtmltopdf_pdf::merge(&parts)?;

    // The print call takes the title from the document's own `<title>` and
    // offers no override, so `--title` can only be honoured by rewriting the
    // file afterwards. The same pass names the producer and dates the document
    // (#29).
    let pdf = rchtmltopdf_pdf::set_metadata(
        &pdf,
        &rchtmltopdf_pdf::Metadata {
            title: settings.global.title.clone(),
            producer: format!("{PROGRAM} {VERSION}"),
            creator: format!("{PROGRAM} {VERSION}"),
            created: now,
        },
    )?;

    // Written before the media errors are judged, because D14 says a subresource
    // that failed under `abort` produces the document *and* exits 1. A wrapper
    // that raises on the exit code still has the PDF to look at.
    output::write(&settings.global.output, &pdf)?;

    // Each document is judged by its own handler. The first `abort` decides the
    // exit code and writes the line applications grep for; the rest are only
    // reported.
    let mut outcome = ExitCode::Success;
    for (index, failures) in &printed.media {
        match (objects[*index].load.on_media_error, failures.as_slice()) {
            (_, []) | (LoadErrorHandling::Ignore, _) => {}
            (LoadErrorHandling::Skip, failures) => {
                for failed in failures {
                    report_media(settings, failed);
                }
            }
            (LoadErrorHandling::Abort, failures) => {
                for failed in failures {
                    report_media(settings, failed);
                }
                if outcome == ExitCode::Success {
                    // wkhtmltopdf's exact wording, and not prefixed with the
                    // program name: applications grep for this line.
                    println_stderr(&format!(
                        "Exit with code 1 due to network error: {}",
                        failures[0].error.name()
                    ));
                }
                outcome = ExitCode::Failure;
            }
        }
    }

    say(settings, "Done");
    Ok(outcome)
}

/// The progress line for one document, in wkhtmltopdf's shape.
///
/// One document keeps the line wrappers have seen since V0. Several say which
/// one this is, because ten identical lines say nothing.
fn loading_line(index: usize, total: usize) -> String {
    if total == 1 {
        "Loading page (1/2)".to_string()
    } else {
        format!("Loading page {} of {total} (1/2)", index + 1)
    }
}

/// A progress line, in wkhtmltopdf's shape and on its stream.
fn say(settings: &Settings, line: &str) {
    if settings.global.log_level.shows_progress() {
        eprintln!("{line}");
    }
}

/// One failed subresource, named the way wkhtmltopdf names it.
fn report_media(settings: &Settings, failed: &Failed) {
    if settings.global.log_level.shows_warnings() {
        eprintln!("{PROGRAM}: warning: {failed} was not loaded");
    }
}

/// Straight to stderr whatever the level, because this line is a contract.
fn println_stderr(line: &str) {
    eprintln!("{line}");
}

/// The documents to convert, in order.
///
/// Pages and covers. A cover is a page that was given no bands and will be
/// left out of the outline (#40); nothing about printing it differs. A table
/// of contents is refused by name: converting the pages around it and saying
/// nothing would produce a document that looks right and is missing part of
/// itself, which is the worst outcome available.
fn pages(settings: &Settings) -> Result<Vec<&ObjectSettings>, ConvertError> {
    if settings.objects.is_empty() {
        return Err(ConvertError::Unsupported("no document to convert".into()));
    }

    for object in &settings.objects {
        match object.kind {
            ObjectKind::Page | ObjectKind::Cover => {}
            ObjectKind::Toc => {
                return Err(ConvertError::Unsupported(
                    "a table of contents is not supported yet (planned for V3)".into(),
                ));
            }
        }
    }

    // Standard input is a stream, and the second read of it gets nothing.
    // Converting an empty document in its place would look like a page that
    // rendered blank, so it is refused instead.
    let from_stdin = settings
        .objects
        .iter()
        .filter(|object| object.input == Some(Input::Stdin))
        .count();
    if from_stdin > 1 {
        return Err(ConvertError::Unsupported(format!(
            "standard input can be read once, and it was given as {from_stdin} documents"
        )));
    }

    Ok(settings.objects.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn one_page_or_several_are_what_gets_converted() {
        assert_eq!(pages(&with(vec![page()])).unwrap().len(), 1);
        assert_eq!(pages(&with(vec![page(), page(), page()])).unwrap().len(), 3);
    }

    #[test]
    fn a_cover_is_converted_like_a_page() {
        let mut cover = page();
        cover.kind = ObjectKind::Cover;
        assert_eq!(pages(&with(vec![cover, page()])).unwrap().len(), 2);
    }

    /// Converting the pages around it and saying nothing would produce a
    /// document that looks right and is missing part of itself.
    #[test]
    fn a_table_of_contents_says_which_milestone_it_waits_for() {
        let mut toc = page();
        toc.kind = ObjectKind::Toc;
        assert!(
            pages(&with(vec![toc, page()]))
                .unwrap_err()
                .to_string()
                .contains("V3")
        );
    }

    /// The second read of a stream gets nothing, and a blank page in its place
    /// would look like a rendering problem.
    #[test]
    fn standard_input_twice_is_refused_rather_than_read_empty() {
        let stdin = || ObjectSettings::page(Input::Stdin);
        assert!(pages(&with(vec![stdin(), page()])).is_ok());

        let error = pages(&with(vec![stdin(), page(), stdin()])).unwrap_err();
        assert!(error.to_string().contains("standard input"), "{error}");
        assert!(error.to_string().contains('2'), "{error}");
    }

    #[test]
    fn nothing_to_convert_is_reported_rather_than_panicking() {
        assert!(pages(&with(vec![])).is_err());
    }

    /// One document keeps the line wrappers have seen since V0.
    #[test]
    fn the_progress_line_says_which_document_only_when_there_are_several() {
        assert_eq!(loading_line(0, 1), "Loading page (1/2)");
        assert_eq!(loading_line(0, 3), "Loading page 1 of 3 (1/2)");
        assert_eq!(loading_line(2, 3), "Loading page 3 of 3 (1/2)");
    }

    #[test]
    fn nothing_left_names_every_document_that_was_skipped() {
        let error = ConvertError::NothingLeft(vec![
            "could not load a: ContentNotFoundError".into(),
            "could not load b: HostNotFoundError".into(),
        ]);
        let message = error.to_string();
        assert!(message.contains("skip"), "{message}");
        assert!(message.contains("could not load a"), "{message}");
        assert!(message.contains("could not load b"), "{message}");
    }
}
