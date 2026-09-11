//! Errors and process exit codes.
//!
//! Exit codes follow wkhtmltopdf rather than inventing a scheme of our own:
//! wrappers such as KnpSnappy raise an exception when the exit code is non-zero
//! and stderr is non-empty, so the pair must keep behaving as it does today.

use std::fmt;

/// How a failed subresource or document load should be handled.
///
/// Mirrors wkhtmltopdf's `--load-error-handling` / `--load-media-error-handling`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadErrorHandling {
    /// Produce the PDF but exit non-zero, naming the error on stderr.
    Abort,
    /// Drop the failing object and carry on.
    Skip,
    /// Carry on as if nothing happened.
    Ignore,
}

impl LoadErrorHandling {
    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "abort" => Some(LoadErrorHandling::Abort),
            "skip" => Some(LoadErrorHandling::Skip),
            "ignore" => Some(LoadErrorHandling::Ignore),
            _ => None,
        }
    }
}

impl fmt::Display for LoadErrorHandling {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadErrorHandling::Abort => write!(f, "abort"),
            LoadErrorHandling::Skip => write!(f, "skip"),
            LoadErrorHandling::Ignore => write!(f, "ignore"),
        }
    }
}

/// The exit code the process should end with.
///
/// wkhtmltopdf uses 0 for success and 1 for everything else. Keeping that pair
/// is part of the drop-in promise, so this deliberately has only two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Success,
    Failure,
}

impl ExitCode {
    pub const fn as_i32(self) -> i32 {
        match self {
            ExitCode::Success => 0,
            ExitCode::Failure => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_wkhtmltopdf() {
        assert_eq!(ExitCode::Success.as_i32(), 0);
        assert_eq!(ExitCode::Failure.as_i32(), 1);
    }

    #[test]
    fn load_error_handling_parses() {
        assert_eq!(
            LoadErrorHandling::parse("abort"),
            Some(LoadErrorHandling::Abort)
        );
        assert_eq!(
            LoadErrorHandling::parse("SKIP"),
            Some(LoadErrorHandling::Skip)
        );
        assert_eq!(
            LoadErrorHandling::parse("ignore"),
            Some(LoadErrorHandling::Ignore)
        );
        assert_eq!(LoadErrorHandling::parse("explode"), None);
    }
}
