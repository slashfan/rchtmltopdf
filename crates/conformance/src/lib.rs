//! The conformance harness: run the real binary against a real Chromium and
//! assert on the bytes that come out (D15).
//!
//! This is a separate workspace member rather than another `tests/` directory
//! inside `cli`, for two reasons. It needs a browser, so it is the one part of
//! the suite that is allowed to be slow and to be skipped. And it tests the
//! *binary*, not any crate's interface, so belonging to none of them is honest.
//!
//! Three things here are worth reading before writing a test with it:
//!
//! - [`require_chromium`] decides between running, skipping and failing. The
//!   skip is what keeps `cargo test` green on a laptop; the failure is what
//!   stops CI from going quiet when the pinned browser disappears.
//! - [`binary`] finds the built binary without cargo's help, because
//!   `CARGO_BIN_EXE_` only exists inside the package that declares the binary.
//! - [`fixture`] builds documents that carry their own font, because a page
//!   count measured against the runner's fonts is not a page count.

pub mod binary;
pub mod fixture;

use rchtmltopdf_browser::locate::{Executable, SystemEnvironment, locate};

/// Set to insist that a browser must be present. CI sets it; a laptop does not.
pub const REQUIRE: &str = "RCHTMLTOPDF_REQUIRE_CHROMIUM";

/// Set when the environment cannot give Chromium a sandbox. Never a default:
/// the product gives the sandbox up only when asked (D10).
pub const NO_SANDBOX: &str = "RCHTMLTOPDF_TEST_NO_SANDBOX";

/// What to do about the browser this machine does or does not have.
#[derive(Debug)]
pub enum Decision {
    /// Run, against this browser.
    Run(Box<Executable>),
    /// No browser, and none was demanded. Say so and pass.
    Skip(String),
    /// No browser, and one was demanded. Fail.
    Fail(String),
}

/// Find the browser the product would find, or say why this test is not running.
///
/// Resolution goes through [`locate`] rather than reading `PATH` here, so the
/// harness cannot end up testing against a browser the product itself would
/// never have chosen — the headless shell preference and the cache rung of D09
/// are part of what is under test.
///
/// Returns `None` after printing a skip line when there is no browser, so
/// `cargo test` works on a machine that has never seen Chromium. When [`REQUIRE`]
/// is set it panics instead. That pair is the whole point: without the skip the
/// suite is unrunnable locally, and without the failure a vanished CI pin turns
/// every browser-backed test into a no-op while CI stays green.
pub fn require_chromium() -> Option<Executable> {
    match decide(
        locate(None, &SystemEnvironment),
        std::env::var_os(REQUIRE).is_some(),
    ) {
        Decision::Run(executable) => Some(*executable),
        Decision::Skip(why) => {
            eprintln!("skipping: {why}");
            None
        }
        Decision::Fail(why) => panic!("{why}"),
    }
}

/// The decision itself, with the machine taken out of it.
///
/// Split from [`require_chromium`] because the interesting cases are a machine
/// with no browser and a machine with no browser *and* [`REQUIRE`] set, and
/// neither can be arranged from inside a test that runs on a developer's laptop
/// with Chrome installed.
pub fn decide(found: rchtmltopdf_browser::Result<Executable>, required: bool) -> Decision {
    match found {
        Ok(executable) => Decision::Run(Box::new(executable)),
        Err(error) if required => Decision::Fail(format!(
            "{REQUIRE} is set, so a browser is required, but none was found.\n\
             This means the pinned browser did not arrive, not that the test is \
             wrong.\n\n{error}"
        )),
        Err(error) => Decision::Skip(format!("no browser on this machine\n{error}")),
    }
}

/// True when the environment has said it cannot provide a sandbox.
///
/// Read by [`binary::Run`] to pass `--no-sandbox` to the child. The product
/// never decides this for itself (D10); only the environment does, and only for
/// tests.
pub fn sandbox_unavailable() -> bool {
    std::env::var_os(NO_SANDBOX).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchtmltopdf_browser::error::{Error, SearchAttempt};
    use rchtmltopdf_browser::locate::{Flavour, Origin};
    use std::path::PathBuf;

    fn found() -> rchtmltopdf_browser::Result<Executable> {
        Ok(Executable {
            path: PathBuf::from("/usr/bin/chrome-headless-shell"),
            origin: Origin::SystemLocation,
            flavour: Flavour::HeadlessShell,
        })
    }

    fn nothing() -> rchtmltopdf_browser::Result<Executable> {
        Err(Error::BrowserNotFound {
            attempts: vec![SearchAttempt {
                source: "PATH".into(),
                path: PathBuf::from("/usr/bin/chromium"),
            }],
        })
    }

    #[test]
    fn a_browser_that_is_there_is_used_whether_or_not_it_was_required() {
        assert!(matches!(decide(found(), false), Decision::Run(_)));
        assert!(matches!(decide(found(), true), Decision::Run(_)));
    }

    /// The half that keeps `cargo test` usable on a machine with no browser.
    #[test]
    fn no_browser_and_none_required_skips() {
        assert!(matches!(decide(nothing(), false), Decision::Skip(_)));
    }

    /// The half that stops CI going quiet. If this ever returns `Skip`, a pinned
    /// browser could vanish and the whole conformance suite would pass by not
    /// running.
    #[test]
    fn no_browser_when_one_was_required_fails() {
        let Decision::Fail(why) = decide(nothing(), true) else {
            panic!("a required browser that is missing must fail, not skip");
        };
        assert!(why.contains(REQUIRE), "{why}");
        // The underlying search is carried through, so the failure says where it
        // looked rather than only that it looked.
        assert!(why.contains("/usr/bin/chromium"), "{why}");
    }
}
