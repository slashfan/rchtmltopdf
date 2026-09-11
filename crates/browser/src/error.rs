//! Errors from driving a browser.

use std::fmt;
use std::path::PathBuf;

/// The error object a failed command reply carries.
///
/// This is the browser refusing or failing a command, as opposed to the
/// connection breaking. It is reported, never panicked on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    pub code: i64,
    pub message: String,
    /// Some failures carry a longer explanation here.
    pub data: Option<String>,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)?;
        if let Some(data) = &self.data {
            write!(f, ": {data}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ProtocolError {}

/// One place the search for a browser looked, and did not find one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchAttempt {
    /// Where the candidate came from, for example `--chromium-path` or `PATH`.
    pub source: String,
    pub path: PathBuf,
}

#[derive(Debug)]
pub enum Error {
    /// No browser anywhere the resolution order looks.
    ///
    /// Carries every path tried so the message can name them. Nothing is ever
    /// downloaded to recover from this (D09); the user is told how to do it.
    BrowserNotFound {
        attempts: Vec<SearchAttempt>,
    },
    /// The browser would not start, or started and never answered.
    ///
    /// Carries the browser's own diagnostics, because that is where a missing
    /// shared library or a refused sandbox actually shows up.
    Launch {
        detail: String,
    },
    /// The browser answered the command with an error.
    Protocol(ProtocolError),
    /// The connection went away before the reply arrived. Usually means the
    /// browser exited, which is worth reporting differently from a refusal.
    ConnectionClosed,
    /// A message exceeded the size cap, which in practice means the stream
    /// desynchronised rather than that the browser genuinely sent something
    /// enormous. Large payloads are meant to travel as streams instead.
    MessageTooLarge {
        limit: usize,
    },
    /// The browser sent something that is not the message shape we expect.
    Malformed {
        detail: String,
    },
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BrowserNotFound { attempts } => {
                writeln!(f, "could not find Chromium. Tried, in order:")?;
                if attempts.is_empty() {
                    writeln!(f, "  (nothing: no candidate locations for this platform)")?;
                }
                for attempt in attempts {
                    writeln!(f, "  {:<24} {}", attempt.source, attempt.path.display())?;
                }
                write!(
                    f,
                    "Pass --chromium-path, set CHROME_PATH, or run `rchtmltopdf fetch-chromium` to download a pinned build."
                )
            }
            Error::Launch { detail } => write!(f, "{detail}"),
            Error::Protocol(error) => write!(f, "the browser rejected the command: {error}"),
            Error::ConnectionClosed => write!(f, "the connection to the browser closed"),
            Error::MessageTooLarge { limit } => write!(
                f,
                "a protocol message exceeded {limit} bytes without ending; the stream is probably out of sync"
            ),
            Error::Malformed { detail } => {
                write!(f, "the browser sent a message we could not read: {detail}")
            }
            Error::Io(error) => write!(f, "talking to the browser failed: {error}"),
            Error::Json(error) => {
                write!(f, "could not encode or decode a protocol message: {error}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Protocol(error) => Some(error),
            Error::Io(error) => Some(error),
            Error::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error::Json(error)
    }
}

impl From<ProtocolError> for Error {
    fn from(error: ProtocolError) -> Self {
        Error::Protocol(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_protocol_error_reads_like_a_sentence() {
        let error = Error::Protocol(ProtocolError {
            code: -32000,
            message: "Cannot navigate to invalid URL".into(),
            data: None,
        });
        assert_eq!(
            error.to_string(),
            "the browser rejected the command: Cannot navigate to invalid URL (code -32000)"
        );
    }

    #[test]
    fn protocol_error_data_is_appended_when_present() {
        let error = ProtocolError {
            code: -32602,
            message: "Invalid parameters".into(),
            data: Some("url: string value expected".into()),
        };
        assert_eq!(
            error.to_string(),
            "Invalid parameters (code -32602): url: string value expected"
        );
    }
}
