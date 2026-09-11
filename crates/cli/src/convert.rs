//! Running a conversion.
//!
//! Everything above this has turned a command line into settings; everything
//! below takes settings and knows nothing about how they were written. This is
//! the seam, and it is short on purpose.

use crate::{input, output};
use rchtmltopdf_browser::deadline;
use rchtmltopdf_browser::locate::{SystemEnvironment, locate};
use rchtmltopdf_browser::render::Progress;
use rchtmltopdf_browser::{Browser, LaunchOptions};
use rchtmltopdf_core::settings::{ObjectKind, Settings};
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
pub async fn convert(settings: &Settings) -> Result<(), ConvertError> {
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
    let options = LaunchOptions {
        no_sandbox: settings.global.browser.no_sandbox,
        extra_args: settings.global.browser.extra_args.clone(),
        allow_slow_scripts: !object.load.stop_slow_scripts,
        ..LaunchOptions::default()
    };

    let progress = Progress::new();
    // The browser is created inside the deadline, so expiry drops it and its Drop
    // stops the process group and removes the profile. Cleanup is not a step that
    // could be skipped.
    let pdf = deadline::within(settings.global.timeout, &progress, async {
        let browser = Browser::launch(&executable, &options).await?;
        let page = browser.new_page().await?;

        page.prepare(&object.web).await?;
        page.load(document.url(), &object.load, &progress).await?;
        let pdf = page
            .print_to_pdf(&settings.global.page, &object.web)
            .await?;

        // Asked to leave rather than killed, so it can finish writing.
        browser.close().await?;
        Ok(pdf)
    })
    .await?;

    output::write(&settings.global.output, &pdf)?;
    Ok(())
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
