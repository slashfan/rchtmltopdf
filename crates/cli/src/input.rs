//! Turning a document reference into something a browser can open.
//!
//! # Standard input becomes a file, and where that file goes matters
//!
//! D06 settles the first half: content read from standard input is written to a
//! file and opened as `file://`, not handed to the browser as a string. A
//! document set as a string has no origin, so every relative `src` and `href` in
//! it breaks.
//!
//! The second half is which directory. A relative reference resolves against the
//! document's own location, so a file in the system temporary directory would
//! look for `logo.png` there. Somebody running `cat page.html | rchtmltopdf - out.pdf`
//! expects `logo.png` to mean the one they can see, so the file is written into
//! the working directory and removed again afterwards.

use rchtmltopdf_core::{Input, NetworkError};
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct InputError {
    pub input: String,
    pub reason: String,
    /// What wkhtmltopdf would have called this, because it fetched even a local
    /// file through its network stack and named every failure from Qt's enum.
    /// It is what the `Exit with code 1 due to network error: …` line carries
    /// when a document never arrived (D14), and applications grep for it.
    pub error: NetworkError,
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not read {}: {}", self.input, self.reason)
    }
}

impl std::error::Error for InputError {}

/// A document a browser can open, plus anything that has to be tidied up.
#[derive(Debug)]
pub struct Resolved {
    url: String,
    /// Removed when this is dropped, which covers every way a conversion can
    /// end, including a deadline cutting it short.
    scratch: Option<Scratch>,
    /// Why the document could not be read, when [`resolve_or_report`] was
    /// asked to carry that rather than refuse (D55). `url` is then where the
    /// file would have been.
    missing: Option<InputError>,
}

impl Resolved {
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The reason this document cannot be loaded, if it was reported rather
    /// than refused: `--load-error-handling` decides what becomes of it.
    pub fn missing(&self) -> Option<&InputError> {
        self.missing.as_ref()
    }

    /// The file written for standard input, while one exists.
    pub fn scratch_path(&self) -> Option<&Path> {
        self.scratch.as_ref().map(|scratch| scratch.path.as_path())
    }
}

#[derive(Debug)]
struct Scratch {
    path: PathBuf,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Resolve a document, reading standard input if that is what it names.
pub fn resolve(input: &Input) -> Result<Resolved, InputError> {
    let directory = working_directory()?;
    let mut stdin = std::io::stdin().lock();
    resolve_in(input, &mut stdin, &directory)
}

/// Resolve a document, and report a file that cannot be read rather than
/// refusing it.
///
/// wkhtmltopdf fetched a local file through the same stack as a URL, so a
/// path that is not there was a load that failed, and `--load-error-handling`
/// said what became of it: `skip` dropped the document, `ignore` left a blank
/// page in its place, and both still wrote the file (D44, D55). [`resolve`]
/// fails at once, which is right under `abort` — nothing will be written, so
/// nothing need be started — and this is for the other two.
pub fn resolve_or_report(input: &Input) -> Result<Resolved, InputError> {
    let directory = working_directory()?;
    let mut stdin = std::io::stdin().lock();
    resolve_in_or_report(input, &mut stdin, &directory)
}

/// [`resolve_or_report`], with the stream and directory supplied.
pub fn resolve_in_or_report(
    input: &Input,
    stdin: &mut dyn Read,
    directory: &Path,
) -> Result<Resolved, InputError> {
    match (input, resolve_in(input, stdin, directory)) {
        (Input::Path(path), Err(error)) => Ok(Resolved {
            url: file_url(&directory.join(path)),
            scratch: None,
            missing: Some(error),
        }),
        (_, outcome) => outcome,
    }
}

fn working_directory() -> Result<PathBuf, InputError> {
    std::env::current_dir().map_err(|error| InputError {
        input: "-".to_string(),
        reason: format!("the working directory is unreadable: {error}"),
        error: NetworkError::UnknownContent,
    })
}

/// Resolve a document, with the stream and directory supplied.
///
/// The seam that lets standard input be tested without a terminal, and without
/// a test changing the process's working directory out from under its
/// neighbours.
pub fn resolve_in(
    input: &Input,
    stdin: &mut dyn Read,
    directory: &Path,
) -> Result<Resolved, InputError> {
    match input {
        // Handed over untouched. Resolving it is the browser's job, and it knows
        // more about URLs than we do.
        Input::Url(url) => Ok(Resolved {
            url: url.clone(),
            scratch: None,
            missing: None,
        }),
        Input::Path(path) => Ok(Resolved {
            url: file_url(&existing_file(path)?),
            scratch: None,
            missing: None,
        }),
        Input::Stdin => from_stdin(stdin, directory),
    }
}

/// Fail here rather than let the browser render an error page.
///
/// Starting a browser to discover a file does not exist costs a second and
/// produces a PDF of a "file not found" page instead of an error.
fn existing_file(path: &Path) -> Result<PathBuf, InputError> {
    let fail = |reason: &str, error: NetworkError| InputError {
        input: path.display().to_string(),
        reason: reason.to_string(),
        error,
    };

    let resolved = std::fs::canonicalize(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => fail("no such file", NetworkError::ContentNotFound),
        std::io::ErrorKind::PermissionDenied => {
            fail("permission denied", NetworkError::ContentAccessDenied)
        }
        _ => fail(&error.to_string(), NetworkError::UnknownContent),
    })?;

    if !resolved.is_file() {
        return Err(fail("not a file", NetworkError::ContentNotFound));
    }
    Ok(resolved)
}

fn from_stdin(stdin: &mut dyn Read, directory: &Path) -> Result<Resolved, InputError> {
    let fail = |reason: String| InputError {
        input: "standard input".to_string(),
        error: NetworkError::UnknownContent,
        reason,
    };

    let mut document = Vec::new();
    stdin
        .read_to_end(&mut document)
        .map_err(|error| fail(error.to_string()))?;

    // The suffix is load-bearing. Without it the browser sniffs the content and
    // may decide a document is plain text, which renders the markup itself
    // rather than the page.
    let path = directory.join(format!(".rchtmltopdf-stdin-{}.html", std::process::id()));

    let mut file = std::fs::File::create(&path).map_err(|error| {
        fail(format!(
            "could not write {} ({error}); reading a document from standard input needs a \
             writable working directory, because that is what its relative links resolve against",
            path.display()
        ))
    })?;
    file.write_all(&document)
        .map_err(|error| fail(error.to_string()))?;
    drop(file);

    let canonical = std::fs::canonicalize(&path).unwrap_or(path.clone());
    Ok(Resolved {
        url: file_url(&canonical),
        scratch: Some(Scratch { path }),
        missing: None,
    })
}

/// Write a document of our own to a file the browser can open, removed when
/// the result is dropped.
///
/// The system temporary directory rather than the working one: unlike a
/// document read from standard input, this one has no relative links to
/// resolve, and it should not appear next to the user's files.
pub fn scratch_document(label: &str, html: &str) -> Result<Resolved, InputError> {
    let fail = |reason: String| InputError {
        input: format!("the {label} document"),
        reason,
        error: NetworkError::UnknownContent,
    };
    let path =
        std::env::temp_dir().join(format!(".rchtmltopdf-{label}-{}.html", std::process::id()));
    std::fs::write(&path, html).map_err(|error| fail(error.to_string()))?;
    let canonical = std::fs::canonicalize(&path).unwrap_or(path.clone());
    Ok(Resolved {
        url: file_url(&canonical),
        scratch: Some(Scratch { path }),
        missing: None,
    })
}

/// Build a `file://` URL from an absolute path.
///
/// Percent-encodes everything outside the unreserved set, so a directory with a
/// space or a hash in its name does not truncate the URL or point somewhere else.
fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                url.push(byte as char);
            }
            other => url.push_str(&format!("%{other:02X}")),
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rch-in-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn resolve_here(input: &Input, stdin: &str, directory: &Path) -> Result<Resolved, InputError> {
        resolve_in(input, &mut stdin.as_bytes(), directory)
    }

    #[test]
    fn a_url_is_handed_over_untouched() {
        let resolved = resolve_here(
            &Input::Url("https://example.com/invoice/42?a=1#b".into()),
            "",
            Path::new("/tmp"),
        )
        .unwrap();
        assert_eq!(resolved.url(), "https://example.com/invoice/42?a=1#b");
        assert!(resolved.scratch_path().is_none());
    }

    #[test]
    fn a_file_becomes_an_absolute_file_url() {
        let dir = scratch_dir("file");
        let page = dir.join("page.html");
        std::fs::write(&page, b"<html></html>").unwrap();

        let resolved = resolve_here(&Input::Path(page.clone()), "", &dir).unwrap();
        assert!(resolved.url().starts_with("file:///"), "{}", resolved.url());
        assert!(resolved.url().ends_with("page.html"), "{}", resolved.url());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A space or a hash in a directory name would otherwise truncate the URL or
    /// send the browser somewhere else entirely.
    #[test]
    fn awkward_characters_in_a_path_are_encoded() {
        let dir = scratch_dir("awkward").join("a dir#with stuff");
        std::fs::create_dir_all(&dir).unwrap();
        let page = dir.join("my page.html");
        std::fs::write(&page, b"<html></html>").unwrap();

        let url = resolve_here(&Input::Path(page), "", &dir).unwrap();
        assert!(url.url().contains("%20"), "{}", url.url());
        assert!(url.url().contains("%23"), "{}", url.url());
        assert!(!url.url().contains(' '), "{}", url.url());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_missing_file_fails_before_any_browser_is_started() {
        let error = resolve_here(
            &Input::Path("/nowhere/page.html".into()),
            "",
            Path::new("/tmp"),
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("/nowhere/page.html"), "{message}");
        assert!(message.contains("no such file"), "{message}");
    }

    /// Asked to report rather than refuse, a missing file comes back as a
    /// document with its reason attached and the URL it would have had, for
    /// `--load-error-handling` to judge (D55).
    #[test]
    fn a_missing_file_is_reported_when_the_handler_is_to_judge() {
        let directory = std::env::temp_dir();
        let input = Input::Path(PathBuf::from("rchtmltopdf-no-such-file.html"));
        let resolved = resolve_in_or_report(&input, &mut std::io::empty(), &directory)
            .expect("reported, not refused");
        let missing = resolved.missing().expect("the reason travels with it");
        assert_eq!(missing.error, NetworkError::ContentNotFound);
        assert!(missing.to_string().contains("no such file"), "{missing}");
        assert!(
            resolved.url().starts_with("file://")
                && resolved.url().ends_with("rchtmltopdf-no-such-file.html"),
            "{}",
            resolved.url()
        );

        // A URL is never missing: the browser is the one to say.
        let input = Input::Url("http://example.invalid/".into());
        let resolved =
            resolve_in_or_report(&input, &mut std::io::empty(), &directory).expect("a URL");
        assert!(resolved.missing().is_none());
    }

    #[test]
    fn a_directory_is_not_a_document() {
        let dir = scratch_dir("dir");
        let error = resolve_here(&Input::Path(dir.clone()), "", &dir).unwrap_err();
        assert!(error.to_string().contains("not a file"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- standard input ------------------------------------------------------

    #[test]
    fn standard_input_becomes_a_file_beside_the_working_directory() {
        let dir = scratch_dir("stdin");
        let resolved = resolve_here(&Input::Stdin, "<html><body>hi</body></html>", &dir).unwrap();

        let written = resolved.scratch_path().expect("a file was written");
        assert_eq!(written.parent(), Some(dir.as_path()), "beside the cwd");
        assert_eq!(
            std::fs::read_to_string(written).unwrap(),
            "<html><body>hi</body></html>"
        );
        assert!(resolved.url().starts_with("file:///"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Without it the browser sniffs the content and may render the markup as
    /// visible text rather than as a page.
    #[test]
    fn the_written_file_is_named_as_html() {
        let dir = scratch_dir("suffix");
        let resolved = resolve_here(&Input::Stdin, "<html></html>", &dir).unwrap();
        let written = resolved.scratch_path().unwrap();
        assert_eq!(written.extension().and_then(|e| e.to_str()), Some("html"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dropping covers every way a conversion ends, including a deadline cutting
    /// it short, which is the path a plain "delete it afterwards" would miss.
    #[test]
    fn the_written_file_is_removed_when_the_document_is_dropped() {
        let dir = scratch_dir("cleanup");
        let path = {
            let resolved = resolve_here(&Input::Stdin, "<html></html>", &dir).unwrap();
            let path = resolved.scratch_path().unwrap().to_path_buf();
            assert!(path.exists(), "should exist while in use");
            path
        };
        assert!(!path.exists(), "should be gone once dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn binary_content_survives_unchanged() {
        let dir = scratch_dir("binary");
        let document = "<html>\u{00e9}\u{4e2d}\r\n</html>";
        let resolved = resolve_here(&Input::Stdin, document, &dir).unwrap();
        assert_eq!(
            std::fs::read(resolved.scratch_path().unwrap()).unwrap(),
            document.as_bytes()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unwritable_working_directory_says_why_it_matters() {
        let error =
            resolve_here(&Input::Stdin, "<html></html>", Path::new("/nowhere/at/all")).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("writable working directory"), "{message}");
        assert!(message.contains("relative links"), "{message}");
    }

    /// A drive letter is not a scheme. The classifier guards this; here it is
    /// asserted through the whole path a document actually takes.
    #[test]
    fn a_windows_style_path_is_a_path_not_a_url() {
        let input = Input::classify(r"C:\tmp\page.html");
        assert!(matches!(input, Input::Path(_)));
        // It does not exist here, so it fails as a file rather than being
        // handed over as a URL.
        let error = resolve_here(&input, "", Path::new("/tmp")).unwrap_err();
        assert!(error.to_string().contains("no such file"), "{error}");
    }
}
