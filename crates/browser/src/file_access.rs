//! Which local files a document is allowed to read.
//!
//! # The rule, and why it is not a launch flag
//!
//! D10's threat model is untrusted HTML: an invoice template a customer uploads,
//! rendered by a web worker. If that document can read `file:///etc/passwd` and
//! draw it into the PDF, the PDF is an exfiltration channel. wkhtmltopdf shipped
//! exactly that until 0.12.6 changed the default, and the change is the reason
//! `--enable-local-file-access` exists at all.
//!
//! Chromium's own default is the permissive one: a `file://` page may read
//! `file://` subresources. There is no switch that turns this into wkhtmltopdf's
//! rule — `--allow-file-access-from-files` governs scripted fetches rather than
//! images and stylesheets, so it is the wrong lever and turning it off changes
//! nothing here. The rule therefore has to be enforced per request, which is
//! what [`crate::intercept`] does with the decision made here.
//!
//! # Three rules
//!
//! - A `file://` subresource of an **http or https** document is always refused.
//!   No option relaxes this: a remote page has no business reading this disk,
//!   and a `--allow` written for a local document must not quietly extend to one
//!   fetched from the network.
//!
//!   **In practice this arm is a backstop.** Chromium's renderer refuses a
//!   `file://` subresource of an http page before it reaches the network stack,
//!   so no request is ever made and `Fetch.requestPaused` never fires for one.
//!   That was measured, not assumed: driving the case prints every paused
//!   request, and the stylesheet is not among them. The rule stays here because
//!   the guarantee is ours to make rather than the browser's to keep, and
//!   because the same policy will be asked about redirects and about whatever a
//!   later Chromium decides to route differently.
//! - From a **local** document it is refused unless `--enable-local-file-access`
//!   was given, or the file sits inside a directory named by `--allow`.
//! - The document itself is always readable. It is the thing we were asked to
//!   convert, and it is reached by a `file://` request like any other.
//!
//! # Symlinks are why this touches the disk
//!
//! An `--allow`ed directory containing a symlink to `/etc` would otherwise be a
//! way through: the path is textually inside the allowed directory and reads
//! something else entirely. Both sides are canonicalised, which resolves the
//! link, so the comparison is between real locations.
//!
//! A path that cannot be canonicalised is waved through rather than refused.
//! There is nothing behind it to protect, and Chromium's own "file not found"
//! is a better answer to a mistyped stylesheet than an invented access error.

use std::path::{Path, PathBuf};

/// What the command line asked for, before a document is known.
///
/// The document half arrives later, at [`Policy::about`], because it is
/// resolved after the settings are read: standard input becomes a file whose
/// name nothing could predict.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// `--enable-local-file-access`.
    pub enabled: bool,
    /// Directories from `--allow`, as written. Canonicalised when the policy is
    /// applied, so a path that does not exist yet is not silently dropped at
    /// parse time.
    pub allowed: Vec<PathBuf>,
}

/// A policy bound to the document it governs.
#[derive(Debug, Clone)]
pub struct FileAccess {
    enabled: bool,
    /// Canonical, and only the ones that exist. A directory named by `--allow`
    /// that is not there cannot contain the file being asked for either way.
    allowed: Vec<PathBuf>,
    /// Whether the document is itself local. A remote one may never read a file.
    document_is_local: bool,
    /// The document's own file, always readable.
    document: Option<PathBuf>,
}

/// What to do with one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Refused, with a sentence naming why. The sentence reaches the user: a
    /// silently missing stylesheet is the hardest kind of failure to diagnose,
    /// because the document renders.
    Block(&'static str),
}

const REMOTE: &str =
    "a document loaded over http or https may never read a local file, whatever the options say";
const NOT_ENABLED: &str = "local file access is off by default; use --enable-local-file-access, or --allow to name \
     the directory";

impl Policy {
    /// Bind this policy to a document, resolving both sides against the disk.
    pub fn about(&self, document_url: &str) -> FileAccess {
        FileAccess {
            enabled: self.enabled,
            allowed: self
                .allowed
                .iter()
                .filter_map(|path| std::fs::canonicalize(path).ok())
                .collect(),
            document_is_local: local_path(document_url).is_some(),
            document: local_path(document_url).and_then(|path| std::fs::canonicalize(path).ok()),
        }
    }
}

impl FileAccess {
    /// Whether any request could be refused.
    ///
    /// When none can be — a local document that has been given the run of the
    /// disk — interception is not installed at all, and every request keeps the
    /// protocol round trip it would otherwise pay to be waved through.
    pub fn polices_anything(&self) -> bool {
        !(self.document_is_local && self.enabled)
    }

    /// Decide about one request.
    ///
    /// Anything that is not a local file is allowed through: this is the local
    /// file policy and nothing else. What a document may reach over the network
    /// is a separate question with separate options.
    pub fn verdict(&self, request_url: &str) -> Verdict {
        let Some(requested) = local_path(request_url) else {
            return Verdict::Allow;
        };

        // Canonicalising first is what makes `..` and a symlink out of an
        // allowed directory the same question rather than two. A path that will
        // not resolve has nothing behind it to protect.
        let Ok(requested) = std::fs::canonicalize(&requested) else {
            return Verdict::Allow;
        };

        // The document we were asked to convert, reached the only way it can be.
        if self.document.as_deref() == Some(requested.as_path()) {
            return Verdict::Allow;
        }

        if !self.document_is_local {
            return Verdict::Block(REMOTE);
        }
        if self.enabled || self.allowed.iter().any(|root| inside(root, &requested)) {
            return Verdict::Allow;
        }
        Verdict::Block(NOT_ENABLED)
    }
}

/// Whether a canonical path sits inside a canonical directory.
///
/// Compares whole components, so `/safe-elsewhere` is not inside `/safe` the way
/// a string prefix would have it.
fn inside(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

/// The local path a URL names, if it names one.
///
/// Public because the conversion needs the same answer for a different question:
/// whether the document it was given is one we can serve ourselves.
///
/// `file://` with an empty host or `localhost`; anything else, including every
/// http and https URL, is not a local file.
pub fn local_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    // A query or fragment is not part of the path, and Chromium will report one
    // on a stylesheet asked for with a cache-busting suffix.
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    Some(PathBuf::from(percent_decode(&rest[..end])))
}

/// Undo the encoding `file_url` applies, so a directory with a space in its name
/// is compared as the path it is.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory with a document, a neighbour, a secret above it and a symlink
    /// pointing at the secret from inside.
    struct Fixtures {
        root: PathBuf,
    }

    impl Fixtures {
        fn new(label: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("rch-access-{}-{label}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("pages")).unwrap();
            std::fs::write(root.join("pages/doc.html"), b"<p>x</p>").unwrap();
            std::fs::write(root.join("pages/style.css"), b"body{}").unwrap();
            std::fs::write(root.join("secret.txt"), b"hunter2").unwrap();
            Self { root }
        }

        fn url(&self, relative: &str) -> String {
            format!(
                "file://{}/{relative}",
                self.root.canonicalize().unwrap().display()
            )
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }
    }

    impl Drop for Fixtures {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_remote_document_may_never_read_a_local_file() {
        let files = Fixtures::new("remote");
        // Everything turned on, and it still may not.
        let policy = Policy {
            enabled: true,
            allowed: vec![files.path("pages")],
        };
        let access = policy.about("https://example.com/invoice");
        assert_eq!(
            access.verdict(&files.url("pages/style.css")),
            Verdict::Block(REMOTE)
        );
    }

    #[test]
    fn a_local_document_reads_nothing_beside_it_by_default() {
        let files = Fixtures::new("default");
        let access = Policy::default().about(&files.url("pages/doc.html"));
        assert_eq!(
            access.verdict(&files.url("pages/style.css")),
            Verdict::Block(NOT_ENABLED)
        );
        // But the document itself has to be readable, or nothing converts at all.
        assert_eq!(access.verdict(&files.url("pages/doc.html")), Verdict::Allow);
    }

    #[test]
    fn the_flag_opens_it() {
        let files = Fixtures::new("enabled");
        let policy = Policy {
            enabled: true,
            allowed: Vec::new(),
        };
        let access = policy.about(&files.url("pages/doc.html"));
        assert_eq!(
            access.verdict(&files.url("pages/style.css")),
            Verdict::Allow
        );
        assert_eq!(access.verdict(&files.url("secret.txt")), Verdict::Allow);
    }

    #[test]
    fn allow_opens_one_directory_and_not_its_parent() {
        let files = Fixtures::new("allow");
        let policy = Policy {
            enabled: false,
            allowed: vec![files.path("pages")],
        };
        let access = policy.about(&files.url("pages/doc.html"));
        assert_eq!(
            access.verdict(&files.url("pages/style.css")),
            Verdict::Allow
        );
        assert_eq!(
            access.verdict(&files.url("secret.txt")),
            Verdict::Block(NOT_ENABLED)
        );
    }

    /// The traversal case, written the way it would actually be attempted.
    #[test]
    fn dots_do_not_climb_out_of_an_allowed_directory() {
        let files = Fixtures::new("traversal");
        let policy = Policy {
            enabled: false,
            allowed: vec![files.path("pages")],
        };
        let access = policy.about(&files.url("pages/doc.html"));
        assert_eq!(
            access.verdict(&files.url("pages/../secret.txt")),
            Verdict::Block(NOT_ENABLED)
        );
    }

    /// Comparing paths as strings would let a sibling directory through, because
    /// `/…/pages-public` starts with `/…/pages`.
    #[test]
    fn a_sibling_with_a_longer_name_is_not_inside() {
        let files = Fixtures::new("sibling");
        std::fs::create_dir_all(files.path("pages-public")).unwrap();
        std::fs::write(files.path("pages-public/leak.css"), b"body{}").unwrap();

        let policy = Policy {
            enabled: false,
            allowed: vec![files.path("pages")],
        };
        let access = policy.about(&files.url("pages/doc.html"));
        assert_eq!(
            access.verdict(&files.url("pages-public/leak.css")),
            Verdict::Block(NOT_ENABLED)
        );
    }

    #[test]
    fn anything_that_is_not_a_local_file_is_not_this_policys_business() {
        let files = Fixtures::new("elsewhere");
        let access = Policy::default().about(&files.url("pages/doc.html"));
        for url in [
            "https://example.com/a.css",
            "http://example.com/a.png",
            "data:text/css,body{}",
            "about:blank",
        ] {
            assert_eq!(access.verdict(url), Verdict::Allow, "{url}");
        }
    }

    /// Chromium says "file not found" better than an invented access error does,
    /// and there is nothing behind a path that will not resolve to protect.
    #[test]
    fn a_file_that_is_not_there_is_left_to_the_browser_to_report() {
        let files = Fixtures::new("missing");
        let access = Policy::default().about(&files.url("pages/doc.html"));
        assert_eq!(access.verdict(&files.url("pages/nope.css")), Verdict::Allow);
    }

    /// Interception costs a round trip on every request, so it is only installed
    /// when it could say no to one.
    #[test]
    fn nothing_is_intercepted_when_nothing_could_be_refused() {
        let files = Fixtures::new("pointless");
        let open = Policy {
            enabled: true,
            allowed: Vec::new(),
        };
        assert!(
            !open.about(&files.url("pages/doc.html")).polices_anything(),
            "a local document given the run of the disk needs no interception"
        );

        // A remote document is policed however the options are written: the rule
        // that stops it reading a local file is not one of them.
        assert!(open.about("https://example.com/a").polices_anything());
        assert!(
            Policy::default()
                .about(&files.url("pages/doc.html"))
                .polices_anything()
        );
    }

    #[test]
    fn an_encoded_path_is_compared_as_the_path_it_is() {
        let files = Fixtures::new("encoded");
        std::fs::create_dir_all(files.path("with space")).unwrap();
        std::fs::write(files.path("with space/a.css"), b"body{}").unwrap();

        let policy = Policy {
            enabled: false,
            allowed: vec![files.path("with space")],
        };
        let access = policy.about(&files.url("pages/doc.html"));
        let encoded = files.url("with%20space/a.css");
        assert_eq!(access.verdict(&encoded), Verdict::Allow, "{encoded}");
    }

    /// Chromium reports the URL it asked for, suffix and all.
    #[test]
    fn a_query_string_is_not_part_of_the_path() {
        let files = Fixtures::new("query");
        let policy = Policy {
            enabled: true,
            allowed: Vec::new(),
        };
        let access = policy.about(&files.url("pages/doc.html"));
        assert_eq!(
            access.verdict(&format!("{}?v=2", files.url("pages/style.css"))),
            Verdict::Allow
        );
    }

    #[test]
    fn a_blocked_verdict_says_what_to_do_about_it() {
        assert!(NOT_ENABLED.contains("--enable-local-file-access"));
        assert!(NOT_ENABLED.contains("--allow"));
    }
}
