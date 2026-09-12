//! Where a document comes from and where the result goes.
//!
//! These live here rather than in the command line layer because the browser
//! and PDF layers consume them too. The command line produces them; nothing
//! downstream should have to know how they were written.

use std::path::PathBuf;

/// A document to convert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Read from standard input.
    Stdin,
    /// Something with a scheme, such as `https://` or `file://`.
    Url(String),
    /// A path on this machine.
    Path(PathBuf),
}

impl Input {
    /// Work out what a command line argument refers to.
    ///
    /// `-` is standard input, anything with a scheme is a URL, everything else
    /// is a path.
    pub fn classify(raw: &str) -> Self {
        if raw == "-" {
            return Input::Stdin;
        }
        if has_url_scheme(raw) {
            return Input::Url(raw.to_string());
        }
        Input::Path(PathBuf::from(raw))
    }

    /// How it was written, for error messages.
    pub fn as_written(&self) -> String {
        match self {
            Input::Stdin => "-".to_string(),
            Input::Url(url) => url.clone(),
            Input::Path(path) => path.display().to_string(),
        }
    }
}

/// Does this look like `scheme://…` rather than a path?
///
/// A single-letter scheme is rejected so a Windows path such as `C:\tmp\a.html`
/// is not mistaken for a URL.
///
/// Public because the same question is asked of things that are not documents:
/// `--user-style-sheet` takes either a path or a URL and has to tell them apart
/// the same way, or two options would disagree about what `C:\a.css` is.
pub fn has_url_scheme(raw: &str) -> bool {
    let Some(colon) = raw.find(':') else {
        return false;
    };
    let scheme = &raw[..colon];
    scheme.len() > 1
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Where the finished PDF goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// Write to standard output.
    Stdout,
    Path(PathBuf),
}

impl Output {
    pub fn classify(raw: &str) -> Self {
        if raw == "-" {
            Output::Stdout
        } else {
            Output::Path(PathBuf::from(raw))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lone_dash_is_the_standard_stream() {
        assert_eq!(Input::classify("-"), Input::Stdin);
        assert_eq!(Output::classify("-"), Output::Stdout);
    }

    #[test]
    fn a_scheme_makes_a_url() {
        assert_eq!(
            Input::classify("https://example.com/a"),
            Input::Url("https://example.com/a".into())
        );
        assert_eq!(
            Input::classify("file:///tmp/a.html"),
            Input::Url("file:///tmp/a.html".into())
        );
    }

    #[test]
    fn a_windows_path_is_not_a_url() {
        // A single-letter "scheme" is a drive letter.
        assert!(matches!(Input::classify(r"C:\tmp\a.html"), Input::Path(_)));
    }

    #[test]
    fn everything_else_is_a_path() {
        assert_eq!(
            Input::classify("page.html"),
            Input::Path(PathBuf::from("page.html"))
        );
        assert_eq!(
            Input::classify("/tmp/knp_snappy_abc.html"),
            Input::Path(PathBuf::from("/tmp/knp_snappy_abc.html"))
        );
    }

    #[test]
    fn how_it_was_written_survives_for_error_messages() {
        assert_eq!(Input::classify("-").as_written(), "-");
        assert_eq!(
            Input::classify("https://example.com").as_written(),
            "https://example.com"
        );
    }
}
