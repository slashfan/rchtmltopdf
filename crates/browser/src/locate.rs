//! Finding a browser to drive.
//!
//! The order is fixed (D09) and **nothing here ever downloads anything** (D31):
//!
//! 1. an explicit `--chromium-path`
//! 2. an environment variable
//! 3. a known system location
//! 4. a cache directory somebody else filled
//!
//! Downloading at conversion time would mean a production container reaching out
//! to the network in the middle of rendering an invoice. Downloading at any
//! other time is a job this program does not want either: it is the one thing
//! that would put a TLS stack, a checksum and an archive unpacker in a binary
//! whose whole networking story is otherwise "the browser does it" (D31). The
//! README says how to install one instead.
//!
//! # What a path may be
//!
//! The first two rungs take whatever the user had to hand, because what they
//! have to hand is usually not an executable. A macOS browser is an `.app`
//! bundle; `@puppeteer/browsers` leaves an unpacked directory. Both used to be
//! rejected for not being a file, which is a bad answer to somebody who has the
//! browser right there. See [`given_candidates`].

use crate::error::{Error, Result, SearchAttempt};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The version of Chrome for Testing this build expects.
///
/// Read from the single file at the repository root, so CI, the Docker image
/// and the README's install command cannot drift apart. Moving that file breaks
/// the build, which is the intent.
pub const PINNED_VERSION: &str = include_str!("../../../.chromium-version").trim_ascii();

/// Which rung of the ladder produced a browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Given explicitly on the command line.
    Flag,
    /// Named by an environment variable.
    Environment { variable: &'static str },
    /// Found where browsers usually live.
    SystemLocation,
    /// Unpacked into the cache directory by something else — a CI step, a
    /// package script, a person. Nothing here puts it there (D31).
    Cache,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Flag => f.write_str("--chromium-path"),
            Origin::Environment { variable } => write!(f, "${variable}"),
            Origin::SystemLocation => f.write_str("a system location"),
            Origin::Cache => f.write_str("the cache directory"),
        }
    }
}

/// Which binary we found, which decides how it has to be launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    /// `chrome-headless-shell`: already headless, smaller, faster to start, and
    /// with no profile or GPU surface. Passing it a headless switch is wrong.
    HeadlessShell,
    /// A full browser, which has to be told to run headless.
    FullBrowser,
}

/// A browser executable, and how it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Executable {
    pub path: PathBuf,
    pub origin: Origin,
    pub flavour: Flavour,
}

impl Executable {
    fn at(path: PathBuf, origin: Origin) -> Self {
        let flavour = if file_name(&path).contains("headless-shell") {
            Flavour::HeadlessShell
        } else {
            Flavour::FullBrowser
        };
        Self {
            path,
            origin,
            flavour,
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// The surroundings the search reads. Separated out so the ladder can be tested
/// on every platform rather than only on whichever one happens to run the tests.
pub trait Environment {
    fn var(&self, key: &str) -> Option<OsString>;
    /// True when the path is a file that can actually be run.
    fn is_executable_file(&self, path: &Path) -> bool;
    fn home(&self) -> Option<PathBuf>;
    fn platform(&self) -> Platform;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    /// How Chrome for Testing names the archive for this platform, used to
    /// recognise an unpacked one wherever it was put.
    fn download_slug(self, arm: bool) -> &'static str {
        match (self, arm) {
            (Platform::Linux, false) => "linux64",
            (Platform::Linux, true) => "linux-arm64",
            (Platform::MacOs, false) => "mac-x64",
            (Platform::MacOs, true) => "mac-arm64",
            (Platform::Windows, _) => "win64",
        }
    }

    fn executable_suffix(self) -> &'static str {
        match self {
            Platform::Windows => ".exe",
            _ => "",
        }
    }
}

/// The real surroundings.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnvironment;

impl Environment for SystemEnvironment {
    fn var(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }

    fn is_executable_file(&self, path: &Path) -> bool {
        let Ok(metadata) = std::fs::metadata(path) else {
            return false;
        };
        if !metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    }

    fn home(&self) -> Option<PathBuf> {
        self.var("HOME")
            .or_else(|| self.var("USERPROFILE"))
            .map(PathBuf::from)
    }

    fn platform(&self) -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// Environment variables consulted, in order. The project's own comes first so
/// it can override a value set for some other tool.
const ENVIRONMENT_VARIABLES: &[&str] = &[
    "RCHTMLTOPDF_CHROMIUM",
    "CHROME_PATH",
    "CHROMIUM_PATH",
    "PUPPETEER_EXECUTABLE_PATH",
];

/// What the executable inside a macOS `.app` bundle is called.
///
/// Never the bundle's own name, and never predictable from it: `Chromium.app`
/// holds `Chromium`, but `Google Chrome for Testing.app` holds
/// `Google Chrome for Testing` and `Chromium.app` from some builds holds
/// `Chromium Helper`. Trying all of them costs nothing.
const BUNDLE_EXECUTABLES: &[&str] = &[
    "Google Chrome for Testing",
    "Chromium",
    "Google Chrome",
    "Google Chrome Canary",
];

/// Executable names looked for on `PATH`, in preference order.
///
/// `chrome-headless-shell` leads deliberately: it starts faster and carries no
/// profile or GPU surface, so when both are installed it is the better choice.
const EXECUTABLE_NAMES: &[&str] = &[
    "chrome-headless-shell",
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
];

/// Find a browser, or explain everywhere that was looked.
pub fn locate(flag: Option<&Path>, env: &dyn Environment) -> Result<Executable> {
    let mut attempts = Vec::new();

    if let Some(given) = flag {
        if let Some(path) = resolve_given(given, env) {
            return Ok(Executable::at(path, Origin::Flag));
        }
        attempts.push(SearchAttempt {
            source: "--chromium-path".into(),
            path: given.to_path_buf(),
        });
    }

    for variable in ENVIRONMENT_VARIABLES {
        let Some(value) = env.var(variable) else {
            continue;
        };
        let given = PathBuf::from(value);
        if let Some(path) = resolve_given(&given, env) {
            return Ok(Executable::at(path, Origin::Environment { variable }));
        }
        attempts.push(SearchAttempt {
            source: (*variable).to_string(),
            path: given,
        });
    }

    for (source, path) in system_candidates(env) {
        if env.is_executable_file(&path) {
            return Ok(Executable::at(path, Origin::SystemLocation));
        }
        attempts.push(SearchAttempt { source, path });
    }

    for path in cache_candidates(env) {
        if env.is_executable_file(&path) {
            return Ok(Executable::at(path, Origin::Cache));
        }
        attempts.push(SearchAttempt {
            source: "download cache".into(),
            path,
        });
    }

    Err(Error::BrowserNotFound { attempts })
}

/// Turn whatever the user pointed at into an executable, if it is one.
pub fn resolve_given(given: &Path, env: &dyn Environment) -> Option<PathBuf> {
    given_candidates(given, env)
        .into_iter()
        .find(|candidate| env.is_executable_file(candidate))
}

/// Everything a path the user gave might mean.
///
/// **Probed rather than listed**, so this needs nothing from the environment
/// beyond "is that a runnable file", and so a test can describe a whole machine
/// as a set of paths.
///
/// Three shapes, because these are the three a person actually has:
///
/// - the executable itself, which is the easy case and is tried first;
/// - a macOS **`.app` bundle**, which is what `/Applications` holds and what a
///   file picker gives you. The executable is buried four levels down under a
///   name that is not the bundle's;
/// - a **directory**, which is what unpacking an archive leaves. Both the plain
///   layout and Chrome for Testing's `chrome-headless-shell-<slug>/` one, since
///   `@puppeteer/browsers` produces the second and the README recommends it.
pub fn given_candidates(given: &Path, env: &dyn Environment) -> Vec<PathBuf> {
    let platform = env.platform();
    let suffix = platform.executable_suffix();
    let mut candidates = vec![given.to_path_buf()];

    // A bundle's executable is not named after the bundle, so every name it
    // could be is tried.
    if given
        .extension()
        .is_some_and(|extension| extension == "app")
    {
        let inside = given.join("Contents").join("MacOS");
        candidates.extend(BUNDLE_EXECUTABLES.iter().map(|name| inside.join(name)));
    }

    // A directory, with the browser either directly inside it or inside the
    // folder an archive unpacked.
    for name in EXECUTABLE_NAMES {
        candidates.push(given.join(format!("{name}{suffix}")));
    }
    for arm in [true, false] {
        let slug = platform.download_slug(arm);
        for name in EXECUTABLE_NAMES {
            candidates.push(
                given
                    .join(format!("{name}-{slug}"))
                    .join(format!("{name}{suffix}")),
            );
        }
    }

    candidates
}

/// Everywhere a browser might already be installed, in preference order.
fn system_candidates(env: &dyn Environment) -> Vec<(String, PathBuf)> {
    let platform = env.platform();
    let suffix = platform.executable_suffix();
    let mut candidates = Vec::new();

    // Anything on PATH, headless shell first.
    if let Some(path_var) = env.var("PATH") {
        for directory in std::env::split_paths(&path_var) {
            for name in EXECUTABLE_NAMES {
                candidates.push((
                    "PATH".to_string(),
                    directory.join(format!("{name}{suffix}")),
                ));
            }
        }
    }

    let fixed: Vec<PathBuf> = match platform {
        Platform::Linux => vec![
            PathBuf::from("/opt/google/chrome/chrome"),
            PathBuf::from("/opt/chromium.org/chromium/chrome"),
            PathBuf::from("/snap/bin/chromium"),
        ],
        Platform::MacOs => {
            let bundles = [
                "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
                "Chromium.app/Contents/MacOS/Chromium",
                "Google Chrome.app/Contents/MacOS/Google Chrome",
            ];
            let mut paths: Vec<PathBuf> = bundles
                .iter()
                .map(|bundle| Path::new("/Applications").join(bundle))
                .collect();
            if let Some(home) = env.home() {
                paths.extend(
                    bundles
                        .iter()
                        .map(|bundle| home.join("Applications").join(bundle)),
                );
            }
            paths
        }
        Platform::Windows => {
            let mut paths = Vec::new();
            for variable in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
                if let Some(root) = env.var(variable) {
                    paths.push(
                        PathBuf::from(root)
                            .join("Google")
                            .join("Chrome")
                            .join("Application")
                            .join("chrome.exe"),
                    );
                }
            }
            paths
        }
    };

    candidates.extend(
        fixed
            .into_iter()
            .map(|path| ("known location".to_string(), path)),
    );
    candidates
}

/// Where an unpacked browser is looked for, last.
///
/// `RCHTMLTOPDF_CACHE_DIR` wins, which is how CI points it at a path inside the
/// workspace so a download can be cached between runs. Otherwise it follows the
/// platform's own convention, never the repository.
///
/// Nothing in this program writes here (D31). It is a rung for whoever does —
/// a CI step, a container build, a person with an archive.
pub fn cache_directory(env: &dyn Environment) -> Option<PathBuf> {
    if let Some(explicit) = env.var("RCHTMLTOPDF_CACHE_DIR") {
        return Some(PathBuf::from(explicit));
    }
    match env.platform() {
        Platform::Linux => env
            .var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env.home().map(|home| home.join(".cache")))
            .map(|base| base.join("rchtmltopdf")),
        Platform::MacOs => env
            .home()
            .map(|home| home.join("Library").join("Caches").join("rchtmltopdf")),
        Platform::Windows => env
            .var("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| env.home().map(|home| home.join("AppData").join("Local")))
            .map(|base| base.join("rchtmltopdf").join("cache")),
    }
}

/// Where a downloaded browser would sit, mirroring how Chrome for Testing packs
/// its archives. Both architectures are tried because the cache may have been
/// filled on a machine we cannot ask about.
fn cache_candidates(env: &dyn Environment) -> Vec<PathBuf> {
    let Some(cache) = cache_directory(env) else {
        return Vec::new();
    };
    let platform = env.platform();
    let suffix = platform.executable_suffix();
    let version = cache.join(PINNED_VERSION);

    [true, false]
        .into_iter()
        .map(|arm| {
            let slug = platform.download_slug(arm);
            version
                .join(format!("chrome-headless-shell-{slug}"))
                .join(format!("chrome-headless-shell{suffix}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    // --- what a path the user gave may be (D31) ------------------------------

    /// What `/Applications` holds and what a file picker hands back. The
    /// executable is four levels down under a name that is not the bundle's.
    #[test]
    fn a_macos_bundle_is_a_browser() {
        let executable = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
        let machine = Fake::new(Platform::MacOs).with_executable(executable);

        let found = locate(Some(Path::new("/Applications/Google Chrome.app")), &machine)
            .expect("a bundle should resolve");
        assert_eq!(found.path, PathBuf::from(executable));
        assert_eq!(found.origin, Origin::Flag);
    }

    /// What unpacking an archive leaves behind.
    #[test]
    fn a_directory_with_a_browser_in_it_is_a_browser() {
        let machine =
            Fake::new(Platform::Linux).with_executable("/opt/browsers/chrome-headless-shell");

        let found =
            locate(Some(Path::new("/opt/browsers")), &machine).expect("a directory should resolve");
        assert_eq!(
            found.path,
            PathBuf::from("/opt/browsers/chrome-headless-shell")
        );
    }

    /// The layout `@puppeteer/browsers` produces, which the README recommends,
    /// so the directory somebody copies out of it has one more level in it.
    #[test]
    fn an_unpacked_chrome_for_testing_directory_is_a_browser() {
        let executable = "/downloads/chrome-headless-shell-linux64/chrome-headless-shell";
        let machine = Fake::new(Platform::Linux).with_executable(executable);

        let found = locate(Some(Path::new("/downloads")), &machine)
            .expect("an unpacked archive should resolve");
        assert_eq!(found.path, PathBuf::from(executable));
    }

    /// The same widening applies to the environment, because somebody exporting
    /// `CHROME_PATH` has exactly the same thing to hand.
    #[test]
    fn an_environment_variable_may_also_name_a_directory() {
        let machine = Fake::new(Platform::Linux)
            .with_var("CHROME_PATH", "/opt/browsers")
            .with_executable("/opt/browsers/chromium");

        let found = locate(None, &machine).expect("should resolve");
        assert_eq!(found.path, PathBuf::from("/opt/browsers/chromium"));
        assert_eq!(
            found.origin,
            Origin::Environment {
                variable: "CHROME_PATH"
            }
        );
    }

    /// A path that is none of those is still a failure, and the failure names
    /// the path **as the user wrote it** rather than the several places that
    /// were probed underneath it.
    #[test]
    fn a_path_that_is_nothing_names_itself_in_the_failure() {
        let machine = Fake::new(Platform::Linux);
        let error = locate(Some(Path::new("/nowhere")), &machine).expect_err("nothing is there");

        let message = error.to_string();
        assert!(message.contains("/nowhere"), "{message}");
        assert_eq!(
            message.matches("/nowhere").count(),
            1,
            "the probes are noise, not attempts to report:\n{message}"
        );
    }

    /// The failure is the one thing a first-time user is guaranteed to read, so
    /// it has to say what to do rather than only what failed (D31).
    #[test]
    fn the_failure_says_how_to_get_a_browser() {
        let message = locate(None, &Fake::new(Platform::Linux))
            .expect_err("nothing is there")
            .to_string();

        assert!(message.contains("apt install chromium"), "{message}");
        assert!(message.contains("@puppeteer/browsers"), "{message}");
        // And the pin, so the command can be pasted rather than researched.
        assert!(message.contains(PINNED_VERSION), "{message}");
        assert!(message.contains("--chromium-path"), "{message}");
        // Nothing promises a downloader that does not exist.
        assert!(!message.contains("fetch-chromium"), "{message}");
    }

    /// A made-up machine. Lets the ladder be tested on all three platforms from
    /// whichever one happens to be running the tests.
    struct Fake {
        platform: Platform,
        vars: HashMap<String, OsString>,
        executables: HashSet<PathBuf>,
    }

    impl Fake {
        fn new(platform: Platform) -> Self {
            Self {
                platform,
                vars: HashMap::new(),
                executables: HashSet::new(),
            }
        }

        fn with_var(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.into(), value.into());
            self
        }

        fn with_executable(mut self, path: impl AsRef<Path>) -> Self {
            self.executables.insert(path.as_ref().to_path_buf());
            self
        }
    }

    impl Environment for Fake {
        fn var(&self, key: &str) -> Option<OsString> {
            self.vars.get(key).cloned()
        }
        fn is_executable_file(&self, path: &Path) -> bool {
            self.executables.contains(path)
        }
        fn home(&self) -> Option<PathBuf> {
            self.vars.get("HOME").map(PathBuf::from)
        }
        fn platform(&self) -> Platform {
            self.platform
        }
    }

    fn linux() -> Fake {
        Fake::new(Platform::Linux)
            .with_var("HOME", "/home/nobody")
            .with_var("PATH", "/usr/local/bin:/usr/bin")
    }

    // --- the ladder, rung by rung ------------------------------------------

    #[test]
    fn the_flag_wins_over_everything_else() {
        let env = linux()
            .with_var("CHROME_PATH", "/usr/bin/google-chrome")
            .with_executable("/usr/bin/google-chrome")
            .with_executable("/usr/bin/chromium")
            .with_executable("/opt/mine/chrome");

        let found = locate(Some(Path::new("/opt/mine/chrome")), &env).unwrap();
        assert_eq!(found.path, PathBuf::from("/opt/mine/chrome"));
        assert_eq!(found.origin, Origin::Flag);
    }

    /// A flag pointing at nothing must not silently fall through to something
    /// else. It falls through, but the attempt is recorded, and if nothing is
    /// found the message names it first.
    #[test]
    fn a_flag_pointing_at_nothing_is_recorded() {
        let env = linux();
        let error = locate(Some(Path::new("/nope/chrome")), &env).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("/nope/chrome"), "{message}");
        assert!(message.contains("--chromium-path"), "{message}");
    }

    #[test]
    fn environment_variables_are_tried_in_order() {
        let env = linux()
            .with_var("CHROME_PATH", "/from/chrome-path")
            .with_var("RCHTMLTOPDF_CHROMIUM", "/from/ours")
            .with_executable("/from/chrome-path")
            .with_executable("/from/ours");

        let found = locate(None, &env).unwrap();
        assert_eq!(found.path, PathBuf::from("/from/ours"));
        assert_eq!(
            found.origin,
            Origin::Environment {
                variable: "RCHTMLTOPDF_CHROMIUM"
            }
        );
    }

    #[test]
    fn a_variable_pointing_at_nothing_falls_through_to_the_next_rung() {
        let env = linux()
            .with_var("CHROME_PATH", "/stale/path")
            .with_executable("/usr/bin/chromium");

        let found = locate(None, &env).unwrap();
        assert_eq!(found.path, PathBuf::from("/usr/bin/chromium"));
        assert_eq!(found.origin, Origin::SystemLocation);
    }

    #[test]
    fn puppeteer_cache_variable_is_honoured() {
        let env = linux()
            .with_var("PUPPETEER_EXECUTABLE_PATH", "/cache/puppeteer/chrome")
            .with_executable("/cache/puppeteer/chrome");
        let found = locate(None, &env).unwrap();
        assert_eq!(
            found.origin,
            Origin::Environment {
                variable: "PUPPETEER_EXECUTABLE_PATH"
            }
        );
    }

    // --- the headless shell preference --------------------------------------

    /// The gotcha from the issue: when both are installed, take the shell. It
    /// starts faster and has no profile or GPU surface.
    #[test]
    fn the_headless_shell_is_preferred_over_a_full_browser() {
        let env = linux()
            .with_executable("/usr/bin/google-chrome")
            .with_executable("/usr/bin/chromium")
            .with_executable("/usr/bin/chrome-headless-shell");

        let found = locate(None, &env).unwrap();
        assert_eq!(found.path, PathBuf::from("/usr/bin/chrome-headless-shell"));
        assert_eq!(found.flavour, Flavour::HeadlessShell);
    }

    #[test]
    fn earlier_path_entries_win() {
        let env = linux()
            .with_executable("/usr/local/bin/chromium")
            .with_executable("/usr/bin/chromium");
        let found = locate(None, &env).unwrap();
        assert_eq!(found.path, PathBuf::from("/usr/local/bin/chromium"));
    }

    #[test]
    fn flavour_is_read_from_the_file_name() {
        let shell = Executable::at("/x/chrome-headless-shell".into(), Origin::Flag);
        assert_eq!(shell.flavour, Flavour::HeadlessShell);
        let full = Executable::at("/x/google-chrome".into(), Origin::Flag);
        assert_eq!(full.flavour, Flavour::FullBrowser);
        // A launcher must not pass a headless switch to the shell, so this
        // distinction has to survive an unusual path.
        let renamed = Executable::at("/opt/Chrome-Headless-Shell".into(), Origin::Flag);
        assert_eq!(renamed.flavour, Flavour::HeadlessShell);
    }

    // --- platforms -----------------------------------------------------------

    #[test]
    fn macos_looks_inside_application_bundles() {
        let env = Fake::new(Platform::MacOs)
            .with_var("HOME", "/Users/nobody")
            .with_executable("/Applications/Chromium.app/Contents/MacOS/Chromium");
        let found = locate(None, &env).unwrap();
        assert_eq!(found.origin, Origin::SystemLocation);
        assert_eq!(found.flavour, Flavour::FullBrowser);
    }

    #[test]
    fn macos_also_looks_in_the_users_own_applications_folder() {
        let env = Fake::new(Platform::MacOs)
            .with_var("HOME", "/Users/nobody")
            .with_executable(
                "/Users/nobody/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            );
        assert!(locate(None, &env).is_ok());
    }

    #[test]
    fn windows_looks_under_program_files() {
        // Built with the same joins the resolver uses. Writing the path as a
        // literal would only pass on Windows, because joining on a Unix host
        // inserts forward slashes.
        let chrome = PathBuf::from(r"C:\Program Files")
            .join("Google")
            .join("Chrome")
            .join("Application")
            .join("chrome.exe");
        let env = Fake::new(Platform::Windows)
            .with_var("PROGRAMFILES", r"C:\Program Files")
            .with_executable(&chrome);

        let found = locate(None, &env).unwrap();
        assert_eq!(found.path, chrome);
        assert_eq!(found.origin, Origin::SystemLocation);
        assert_eq!(found.flavour, Flavour::FullBrowser);
    }

    // --- the cache -----------------------------------------------------------

    #[test]
    fn the_cache_directory_follows_platform_convention() {
        let linux_cache = cache_directory(&linux()).unwrap();
        assert_eq!(
            linux_cache,
            PathBuf::from("/home/nobody/.cache/rchtmltopdf")
        );

        let with_xdg = linux().with_var("XDG_CACHE_HOME", "/xdg");
        assert_eq!(
            cache_directory(&with_xdg).unwrap(),
            PathBuf::from("/xdg/rchtmltopdf")
        );

        let mac = Fake::new(Platform::MacOs).with_var("HOME", "/Users/nobody");
        assert_eq!(
            cache_directory(&mac).unwrap(),
            PathBuf::from("/Users/nobody/Library/Caches/rchtmltopdf")
        );
    }

    /// CI sets this so the pinned browser can be cached on a path inside the
    /// workspace. It must beat the platform convention.
    #[test]
    fn the_cache_directory_can_be_overridden() {
        let env = linux().with_var("RCHTMLTOPDF_CACHE_DIR", "/work/.chromium-cache");
        assert_eq!(
            cache_directory(&env).unwrap(),
            PathBuf::from("/work/.chromium-cache")
        );
    }

    #[test]
    fn a_browser_in_the_cache_is_found_last_but_is_found() {
        let cached = format!(
            "/work/cache/{PINNED_VERSION}/chrome-headless-shell-linux64/chrome-headless-shell"
        );
        let env = linux()
            .with_var("RCHTMLTOPDF_CACHE_DIR", "/work/cache")
            .with_executable(&cached);

        let found = locate(None, &env).unwrap();
        assert_eq!(found.path, PathBuf::from(&cached));
        assert_eq!(found.origin, Origin::Cache);
        assert_eq!(found.flavour, Flavour::HeadlessShell);
    }

    #[test]
    fn the_pinned_version_is_read_from_the_single_source_of_truth() {
        // Sanity: it should look like a Chrome version, not an empty string or a
        // stray newline from the file.
        assert!(!PINNED_VERSION.is_empty());
        assert!(!PINNED_VERSION.contains('\n'));
        assert_eq!(PINNED_VERSION.split('.').count(), 4, "{PINNED_VERSION}");
    }

    // --- failure -------------------------------------------------------------

    #[test]
    fn finding_nothing_lists_everywhere_it_looked_and_says_what_to_do() {
        let env = linux()
            .with_var("CHROME_PATH", "/stale")
            .with_var("RCHTMLTOPDF_CACHE_DIR", "/work/cache");
        let message = locate(Some(Path::new("/flag/chrome")), &env)
            .unwrap_err()
            .to_string();

        // Every rung is represented, in order.
        let rungs = [
            "/flag/chrome",
            "/stale",
            "/usr/local/bin/chrome-headless-shell",
            "/opt/google/chrome/chrome",
            "/work/cache",
        ];
        let mut cursor = 0;
        for rung in rungs {
            let at = message[cursor..]
                .find(rung)
                .unwrap_or_else(|| panic!("{rung} missing or out of order in:\n{message}"));
            cursor += at;
        }

        // Says what to do about it, and never suggests it might fetch a browser
        // itself (D31).
        assert!(message.contains("--chromium-path"), "{message}");
        assert!(!message.to_lowercase().contains("downloading"), "{message}");
    }

    #[test]
    fn a_machine_with_nothing_set_still_produces_a_usable_message() {
        let env = Fake::new(Platform::Linux);
        let message = locate(None, &env).unwrap_err().to_string();
        assert!(message.contains("could not find Chromium"), "{message}");
        assert!(message.contains("apt install chromium"), "{message}");
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;

    /// Runs the real ladder against whatever machine this is.
    ///
    /// Deliberately tolerant: a machine with no browser is a legitimate outcome
    /// and must produce a good error rather than a panic.
    ///
    /// The success arm asks the filesystem directly rather than re-running the
    /// predicate the ladder used to choose the answer, which could only fail in
    /// a race and therefore told us nothing.
    #[test]
    fn resolving_on_this_machine_either_finds_something_or_explains_itself() {
        match locate(None, &SystemEnvironment) {
            Ok(browser) => {
                println!("found {} via {}", browser.path.display(), browser.origin);
                let metadata = std::fs::metadata(&browser.path)
                    .expect("the resolved path should exist on disk");
                assert!(metadata.is_file(), "a browser should be a file");
                assert!(browser.path.is_absolute(), "the path should be absolute");
            }
            Err(error) => {
                println!("{error}");
                let message = error.to_string();
                assert!(message.contains("--chromium-path"));
            }
        }
    }

    /// The only part of the ladder that touches a disk, and the only one with a
    /// permission check. A symbolic link must be followed: on a Linux runner
    /// `/usr/bin/google-chrome` is a link into `/opt`, so rejecting links would
    /// break every browser test with a "not found" message and no unit test
    /// would flag it.
    #[test]
    #[cfg(unix)]
    fn executability_is_judged_by_the_filesystem() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("rch-exec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let env = SystemEnvironment;

        let runnable = root.join("runnable");
        std::fs::write(&runnable, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&runnable, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(env.is_executable_file(&runnable));

        let plain = root.join("plain");
        std::fs::write(&plain, b"data").unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!env.is_executable_file(&plain), "not executable");

        let directory = root.join("a-directory");
        std::fs::create_dir(&directory).unwrap();
        assert!(
            !env.is_executable_file(&directory),
            "a directory is not a browser"
        );

        let link = root.join("link");
        std::os::unix::fs::symlink(&runnable, &link).unwrap();
        assert!(
            env.is_executable_file(&link),
            "a link to an executable must be followed"
        );

        let dangling = root.join("dangling");
        std::os::unix::fs::symlink(root.join("nothing"), &dangling).unwrap();
        assert!(!env.is_executable_file(&dangling), "a dangling link");

        assert!(!env.is_executable_file(&root.join("absent")));

        let _ = std::fs::remove_dir_all(&root);
    }
}
