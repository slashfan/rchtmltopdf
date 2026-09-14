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

/// The network error names wkhtmltopdf prints, which are Qt's.
///
/// **Applications grep for these strings.** A Symfony application that retries on
/// `HostNotFoundError` and gives up on `ContentNotFoundError` is reading the
/// stderr of a program it did not write, and it will go on doing that after the
/// program underneath has been replaced. So the names are wkhtmltopdf's, spelled
/// exactly as Qt spelled them, and the mapping from what Chromium actually
/// reports is the thing being delivered here rather than an implementation
/// detail.
///
/// Nothing here knows about Chromium's protocol: these take the strings and
/// numbers a network layer produces and say what wkhtmltopdf would have called
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkError {
    /// A 404, or a local file that is not there.
    ContentNotFound,
    /// The name would not resolve.
    HostNotFound,
    /// Nothing was listening, including a proxy that is not there.
    ConnectionRefused,
    Timeout,
    /// A 401 or 403, or a file the policy would not allow (D10).
    ContentAccessDenied,
    /// The far end hung up part-way.
    RemoteHostClosed,
    /// A scheme nothing can fetch.
    ProtocolUnknown,
    /// Everything else. Qt had one of these too.
    UnknownContent,
}

impl NetworkError {
    /// The name as wkhtmltopdf writes it.
    pub const fn name(self) -> &'static str {
        match self {
            NetworkError::ContentNotFound => "ContentNotFoundError",
            NetworkError::HostNotFound => "HostNotFoundError",
            NetworkError::ConnectionRefused => "ConnectionRefusedError",
            NetworkError::Timeout => "TimeoutError",
            NetworkError::ContentAccessDenied => "ContentAccessDenied",
            NetworkError::RemoteHostClosed => "RemoteHostClosedError",
            NetworkError::ProtocolUnknown => "ProtocolUnknownError",
            NetworkError::UnknownContent => "UnknownContentError",
        }
    }

    /// The number Qt gave it, which wkhtmltopdf prints beside the URL.
    ///
    /// `Failed to load <url>, with network status code 3 and http status code
    /// 0 - …` is `QNetworkReply::NetworkError` as an integer, and applications
    /// that parse that line read the number rather than the name. The values
    /// are Qt 4.8's enumerators, which are not contiguous: the connection
    /// errors are 1–4, the content errors 201–299 and the protocol errors 301
    /// onward.
    pub const fn code(self) -> u16 {
        match self {
            NetworkError::ConnectionRefused => 1,
            NetworkError::RemoteHostClosed => 2,
            NetworkError::HostNotFound => 3,
            NetworkError::Timeout => 4,
            NetworkError::ContentAccessDenied => 201,
            NetworkError::ContentNotFound => 203,
            NetworkError::UnknownContent => 299,
            NetworkError::ProtocolUnknown => 301,
        }
    }

    /// Read one of Chromium's `net::ERR_*` strings.
    ///
    /// The prefix is optional so this can be given either `net::ERR_TIMED_OUT`
    /// or `ERR_TIMED_OUT`, and an unknown code lands on `UnknownContent` rather
    /// than being dropped: an error nobody mapped is still an error, and a
    /// wrapper reading stderr would rather see a name it does not recognise than
    /// silence.
    pub fn from_chromium(error_text: &str) -> Self {
        let code = error_text
            .trim()
            .rsplit("::")
            .next()
            .unwrap_or(error_text)
            .trim();

        match code {
            "ERR_NAME_NOT_RESOLVED"
            | "ERR_NAME_RESOLUTION_FAILED"
            | "ERR_INTERNET_DISCONNECTED" => NetworkError::HostNotFound,
            "ERR_CONNECTION_REFUSED"
            | "ERR_PROXY_CONNECTION_FAILED"
            | "ERR_TUNNEL_CONNECTION_FAILED"
            | "ERR_ADDRESS_UNREACHABLE" => NetworkError::ConnectionRefused,
            "ERR_TIMED_OUT" | "ERR_CONNECTION_TIMED_OUT" => NetworkError::Timeout,
            "ERR_FILE_NOT_FOUND" => NetworkError::ContentNotFound,
            "ERR_ACCESS_DENIED"
            | "ERR_BLOCKED_BY_CLIENT"
            | "ERR_BLOCKED_BY_RESPONSE"
            | "ERR_INVALID_AUTH_CREDENTIALS" => NetworkError::ContentAccessDenied,
            "ERR_CONNECTION_CLOSED"
            | "ERR_CONNECTION_RESET"
            | "ERR_CONNECTION_ABORTED"
            | "ERR_EMPTY_RESPONSE" => NetworkError::RemoteHostClosed,
            "ERR_UNKNOWN_URL_SCHEME" | "ERR_DISALLOWED_URL_SCHEME" => NetworkError::ProtocolUnknown,
            _ => NetworkError::UnknownContent,
        }
    }

    /// Read an HTTP status, if it is one worth reporting.
    ///
    /// A response that arrived is not a network failure to Chromium — the bytes
    /// came back, and a 404 page is a page. It was one to Qt, which is why
    /// `ContentNotFoundError` exists at all, so the status is inspected here to
    /// keep the two programs saying the same thing about the same server.
    pub const fn from_status(status: u16) -> Option<Self> {
        match status {
            404 | 410 => Some(NetworkError::ContentNotFound),
            401 | 403 => Some(NetworkError::ContentAccessDenied),
            // Everything else a server can refuse with, in one range.
            400..=599 => Some(NetworkError::UnknownContent),
            _ => None,
        }
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
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

    /// The eight names the issue lists, which are the ones applications grep
    /// for. A typo here is a silent compatibility break, so they are written out
    /// rather than derived.
    #[test]
    fn the_names_are_qts_spelled_qts_way() {
        assert_eq!(NetworkError::ContentNotFound.name(), "ContentNotFoundError");
        assert_eq!(NetworkError::HostNotFound.name(), "HostNotFoundError");
        assert_eq!(
            NetworkError::ConnectionRefused.name(),
            "ConnectionRefusedError"
        );
        assert_eq!(NetworkError::Timeout.name(), "TimeoutError");
        // The one without the `Error` suffix, which is Qt's own inconsistency
        // and is reproduced rather than tidied.
        assert_eq!(
            NetworkError::ContentAccessDenied.name(),
            "ContentAccessDenied"
        );
        assert_eq!(
            NetworkError::RemoteHostClosed.name(),
            "RemoteHostClosedError"
        );
        assert_eq!(NetworkError::ProtocolUnknown.name(), "ProtocolUnknownError");
        assert_eq!(NetworkError::UnknownContent.name(), "UnknownContentError");
    }

    #[test]
    fn chromium_codes_land_on_the_name_qt_would_have_used() {
        let cases = [
            ("net::ERR_NAME_NOT_RESOLVED", NetworkError::HostNotFound),
            (
                "net::ERR_CONNECTION_REFUSED",
                NetworkError::ConnectionRefused,
            ),
            (
                "net::ERR_PROXY_CONNECTION_FAILED",
                NetworkError::ConnectionRefused,
            ),
            ("net::ERR_TIMED_OUT", NetworkError::Timeout),
            ("net::ERR_FILE_NOT_FOUND", NetworkError::ContentNotFound),
            ("net::ERR_ACCESS_DENIED", NetworkError::ContentAccessDenied),
            (
                "net::ERR_INVALID_AUTH_CREDENTIALS",
                NetworkError::ContentAccessDenied,
            ),
            ("net::ERR_EMPTY_RESPONSE", NetworkError::RemoteHostClosed),
            ("net::ERR_UNKNOWN_URL_SCHEME", NetworkError::ProtocolUnknown),
        ];
        for (code, expected) in cases {
            assert_eq!(NetworkError::from_chromium(code), expected, "{code}");
        }
    }

    /// An error nobody mapped is still an error. A wrapper reading stderr would
    /// rather see a name it does not recognise than see nothing.
    #[test]
    fn an_unmapped_code_is_reported_rather_than_dropped() {
        assert_eq!(
            NetworkError::from_chromium("net::ERR_SOMETHING_NEW_IN_CHROMIUM_140"),
            NetworkError::UnknownContent
        );
        assert_eq!(
            NetworkError::from_chromium(""),
            NetworkError::UnknownContent
        );
    }

    /// The prefix is Chromium's, not part of the code.
    #[test]
    fn the_net_prefix_is_optional() {
        assert_eq!(
            NetworkError::from_chromium("ERR_TIMED_OUT"),
            NetworkError::Timeout
        );
        assert_eq!(
            NetworkError::from_chromium("  net::ERR_TIMED_OUT  "),
            NetworkError::Timeout
        );
    }

    /// A response that arrived is not a network failure to Chromium. It was one
    /// to Qt, which is where `ContentNotFoundError` comes from.
    #[test]
    fn http_statuses_map_the_way_qt_read_them() {
        assert_eq!(
            NetworkError::from_status(404),
            Some(NetworkError::ContentNotFound)
        );
        assert_eq!(
            NetworkError::from_status(403),
            Some(NetworkError::ContentAccessDenied)
        );
        assert_eq!(
            NetworkError::from_status(401),
            Some(NetworkError::ContentAccessDenied)
        );
        assert_eq!(
            NetworkError::from_status(500),
            Some(NetworkError::UnknownContent)
        );

        // Anything that is not an error is not one.
        for ok in [200, 204, 301, 302, 304] {
            assert_eq!(NetworkError::from_status(ok), None, "{ok}");
        }
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
