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

use rchtmltopdf_browser::locate::{Executable, PINNED_VERSION, SystemEnvironment, locate};
use std::path::Path;

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
    let required = std::env::var_os(REQUIRE).is_some();
    match decide(locate(None, &SystemEnvironment), required) {
        Decision::Run(executable) => {
            if let Err(why) = pin_check(
                version_of(&executable.path).as_deref(),
                PINNED_VERSION,
                required,
            ) {
                panic!(
                    "{}\n\nresolved {} from {}",
                    why,
                    executable.path.display(),
                    executable.origin
                );
            }
            Some(*executable)
        }
        Decision::Skip(why) => {
            eprintln!("skipping: {why}");
            None
        }
        Decision::Fail(why) => panic!("{why}"),
    }
}

/// Ask a browser what it is.
///
/// `--version` rather than the protocol, because this runs before every
/// browser-backed test and launching one to read a string would cost more than
/// the tests do.
pub fn version_of(path: &Path) -> Option<String> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&output.stdout).into_owned();
    parse_version(&line)
}

/// Pull the version out of a line like `Google Chrome 153.0.8010.36` or
/// `Chrome Headless Shell 153.0.8010.36`. The product name varies by flavour and
/// by platform, so the shape of the number is what is looked for, not the words
/// around it.
fn parse_version(line: &str) -> Option<String> {
    line.split_whitespace()
        .find(|token| {
            let parts: Vec<&str> = token.split('.').collect();
            parts.len() == 4
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(str::to_owned)
}

/// Insist that a demanded browser is the demanded *version*.
///
/// The rot this catches is the one that already happened once: the CI job
/// downloaded the pinned Chrome for Testing, D09's system rung matched the
/// runner's own `/usr/bin/chromium` first, and the suite ran against an
/// unpinned browser while reporting nothing unusual. `RCHTMLTOPDF_REQUIRE_CHROMIUM`
/// stops a browser from going *missing* quietly; this stops it from being the
/// *wrong* one, which is the same failure wearing a green tick.
///
/// Only when a browser was demanded. A laptop runs whatever Chrome it has, which
/// is the point of the skip-or-run pair, and conformance figures that depend on
/// the renderer are CI's to produce.
pub fn pin_check(reported: Option<&str>, pinned: &str, required: bool) -> Result<(), String> {
    if !required {
        return Ok(());
    }
    match reported {
        Some(version) if version == pinned => Ok(()),
        Some(version) => Err(format!(
            "{REQUIRE} is set, so the browser must be the pinned one, but it reports \
             {version} and .chromium-version pins {pinned}.\n\
             D09 puts system locations above the download cache, so a runner that \
             ships its own Chromium wins unless RCHTMLTOPDF_CHROMIUM names the \
             pinned build."
        )),
        None => Err(format!(
            "{REQUIRE} is set, so the browser must be the pinned one ({pinned}), but \
             it would not say which version it is."
        )),
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

    // --- the pin ------------------------------------------------------------

    #[test]
    fn a_version_is_read_out_of_whatever_the_browser_calls_itself() {
        assert_eq!(
            parse_version("Google Chrome 153.0.8010.36 ").as_deref(),
            Some("153.0.8010.36")
        );
        assert_eq!(
            parse_version("Chrome Headless Shell 153.0.8010.36").as_deref(),
            Some("153.0.8010.36")
        );
        assert_eq!(
            parse_version("Chromium 153.0.8010.36 snap").as_deref(),
            Some("153.0.8010.36")
        );
        assert_eq!(parse_version("").as_deref(), None);
        // Not four parts, so not a Chrome version.
        assert_eq!(parse_version("some-tool 1.2.3").as_deref(), None);
    }

    /// A laptop runs whatever Chrome it has. Demanding the pin there would make
    /// the suite unrunnable for anyone who has not downloaded a specific build.
    #[test]
    fn a_browser_that_was_not_demanded_may_be_any_version() {
        assert!(pin_check(Some("1.2.3.4"), "153.0.8010.36", false).is_ok());
        assert!(pin_check(None, "153.0.8010.36", false).is_ok());
    }

    #[test]
    fn the_pinned_version_satisfies_the_pin() {
        assert!(pin_check(Some("153.0.8010.36"), "153.0.8010.36", true).is_ok());
    }

    /// The failure that shipped a green tick: the job downloaded the pin and ran
    /// against the runner's own Chromium instead.
    #[test]
    fn a_demanded_browser_of_the_wrong_version_fails() {
        let why = pin_check(Some("140.0.1.2"), "153.0.8010.36", true).unwrap_err();
        assert!(why.contains("140.0.1.2"), "{why}");
        assert!(why.contains("153.0.8010.36"), "{why}");
        assert!(
            why.contains("RCHTMLTOPDF_CHROMIUM"),
            "should say how to fix it: {why}"
        );
    }

    #[test]
    fn a_demanded_browser_that_will_not_say_its_version_fails() {
        assert!(pin_check(None, "153.0.8010.36", true).is_err());
    }

    // --- skip or run ---------------------------------------------------------

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
