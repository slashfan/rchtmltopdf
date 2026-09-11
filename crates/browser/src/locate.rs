//! Finding a browser to drive.
//!
//! The order is fixed (D09) and it never downloads anything:
//!
//! 1. an explicit `--chromium-path`
//! 2. an environment variable
//! 3. a known system location
//! 4. the cache directory that `fetch-chromium` fills
//!
//! Downloading at conversion time would mean a production container reaching
//! out to the network in the middle of rendering an invoice, so the fourth rung
//! only ever *reads* a directory somebody filled on purpose.

use crate::error::{Error, Result, SearchAttempt};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The version of Chrome for Testing this build expects.
///
/// Read from the single file at the repository root, so CI, the Docker image
/// and `fetch-chromium` cannot drift apart. Moving that file breaks the build,
/// which is the intent.
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
    /// Downloaded earlier by `fetch-chromium`.
    Cache,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Flag => f.write_str("--chromium-path"),
            Origin::Environment { variable } => write!(f, "${variable}"),
            Origin::SystemLocation => f.write_str("a system location"),
            Origin::Cache => f.write_str("the download cache"),
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

/// A browser, and how it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Browser {
    pub path: PathBuf,
    pub origin: Origin,
    pub flavour: Flavour,
}

impl Browser {
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
    /// How Chrome for Testing names the archive for this platform, used to find
    /// what `fetch-chromium` unpacked.
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
pub fn locate(flag: Option<&Path>, env: &dyn Environment) -> Result<Browser> {
    let mut attempts = Vec::new();

    if let Some(path) = flag {
        if env.is_executable_file(path) {
            return Ok(Browser::at(path.to_path_buf(), Origin::Flag));
        }
        attempts.push(SearchAttempt {
            source: "--chromium-path".into(),
            path: path.to_path_buf(),
        });
    }

    for variable in ENVIRONMENT_VARIABLES {
        let Some(value) = env.var(variable) else {
            continue;
        };
        let path = PathBuf::from(value);
        if env.is_executable_file(&path) {
            return Ok(Browser::at(path, Origin::Environment { variable }));
        }
        attempts.push(SearchAttempt {
            source: (*variable).to_string(),
            path,
        });
    }

    for (source, path) in system_candidates(env) {
        if env.is_executable_file(&path) {
            return Ok(Browser::at(path, Origin::SystemLocation));
        }
        attempts.push(SearchAttempt { source, path });
    }

    for path in cache_candidates(env) {
        if env.is_executable_file(&path) {
            return Ok(Browser::at(path, Origin::Cache));
        }
        attempts.push(SearchAttempt {
            source: "download cache".into(),
            path,
        });
    }

    Err(Error::BrowserNotFound { attempts })
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

/// The directory `fetch-chromium` writes into.
///
/// `RCHTMLTOPDF_CACHE_DIR` wins, which is how CI points it at a path inside the
/// workspace so the download can be cached between runs. Otherwise it follows
/// the platform's own convention, never the repository.
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
        let shell = Browser::at("/x/chrome-headless-shell".into(), Origin::Flag);
        assert_eq!(shell.flavour, Flavour::HeadlessShell);
        let full = Browser::at("/x/google-chrome".into(), Origin::Flag);
        assert_eq!(full.flavour, Flavour::FullBrowser);
        // A launcher must not pass a headless switch to the shell, so this
        // distinction has to survive an unusual path.
        let renamed = Browser::at("/opt/Chrome-Headless-Shell".into(), Origin::Flag);
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

        assert!(message.contains("fetch-chromium"), "{message}");
        // Never suggests that it might download something by itself.
        assert!(!message.to_lowercase().contains("downloading"), "{message}");
    }

    #[test]
    fn a_machine_with_nothing_set_still_produces_a_usable_message() {
        let env = Fake::new(Platform::Linux);
        let message = locate(None, &env).unwrap_err().to_string();
        assert!(message.contains("could not find Chromium"), "{message}");
        assert!(message.contains("fetch-chromium"), "{message}");
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;

    /// Runs the real ladder against whatever machine this is.
    ///
    /// Deliberately tolerant: a machine with no browser is a legitimate outcome
    /// and must produce a good error rather than a panic. What it pins is that
    /// a success is coherent, and that a failure explains itself.
    #[test]
    fn resolving_on_this_machine_either_finds_something_or_explains_itself() {
        let env = SystemEnvironment;
        match locate(None, &env) {
            Ok(browser) => {
                println!("found {} via {}", browser.path.display(), browser.origin);
                assert!(env.is_executable_file(&browser.path));
            }
            Err(error) => {
                println!("{error}");
                let message = error.to_string();
                assert!(message.contains("fetch-chromium"));
            }
        }
    }
}
