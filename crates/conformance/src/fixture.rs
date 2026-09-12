//! Documents that carry their own font.
//!
//! **Page count is not font independent.** Line wrapping follows font metrics,
//! and the fonts installed differ between a GitHub runner image, a developer's
//! Mac and the Docker image. A fixture that asks for `sans-serif` is measured
//! against a different typeface in each, so a page count that passes on a laptop
//! can fail in CI for a reason that has nothing to do with the change under
//! test.
//!
//! So every fixture declares [`FAMILY`], which no system font answers to, with
//! **no fallback**. Page count is then a function of the pinned Chromium alone.
//!
//! The font travels inside the document as a `data:` URI rather than as a
//! relative URL, because a fixture also has to work when it is read from
//! standard input, where there is no base URL to resolve against, and from
//! `file://`, where sub-resource access is off by default (D10).
//!
//! See `fixtures/fonts/README.md` for where the file came from.

use base64::Engine;
use std::path::{Path, PathBuf};

/// The Latin subset of Noto Sans Regular. See `fixtures/fonts/README.md`.
pub const FONT: &[u8] = include_bytes!("../fixtures/fonts/noto-sans-latin-400-normal.woff2");

/// The family name every fixture asks for.
///
/// Deliberately not the font's real name: if a machine happened to have Noto
/// Sans installed, a fixture that failed to load the vendored file would still
/// render, and the determinism this whole arrangement buys would be gone without
/// a single test going red.
pub const FAMILY: &str = "Conformance Sans";

/// Wrap a body in a document that brings its own font.
pub fn document(body: &str) -> String {
    format!("{}{body}</body></html>\n", head(Some("utf-8")))
}

/// The same document with **no charset declaration**, as bytes.
///
/// For asking what `--encoding` does, which a document that declares its own
/// charset answers by itself. Bytes rather than a string, because the whole
/// point is content that is not valid in the encoding it will be read as.
///
/// The font still travels inside it, so a page count is still a function of the
/// pinned Chromium: the `data:` URI is ASCII and survives being decoded as
/// anything.
pub fn undeclared(body: &[u8]) -> Vec<u8> {
    let mut document = head(None).into_bytes();
    document.extend_from_slice(body);
    document.extend_from_slice(b"</body></html>\n");
    document
}

/// Everything up to and including `<body>`, with or without a charset.
fn head(charset: Option<&str>) -> String {
    let font = base64::engine::general_purpose::STANDARD.encode(FONT);
    let declaration = match charset {
        Some(name) => format!("<meta charset=\"{name}\">"),
        None => String::new(),
    };
    format!(
        "<!doctype html>\n\
         <html><head>{declaration}<title>conformance</title><style>\n\
         @font-face {{\n  \
           font-family: \"{FAMILY}\";\n  \
           src: url(data:font/woff2;base64,{font}) format(\"woff2\");\n  \
           font-display: block;\n\
         }}\n\
         /* No fallback, on purpose: a fixture that cannot load the font must\n   \
            render wrong rather than render with something else. */\n\
         html, body {{ font-family: \"{FAMILY}\"; }}\n\
         /* Page margins come from the command line, never from the document, so\n   \
            a margin assertion measures what was asked for. */\n\
         body {{ margin: 0; }}\n\
         </style></head>\n\
         <body>"
    )
}

/// Base64, for a fixture that needs to carry something inline.
pub fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Write a document to a file and hand back its path.
pub fn write(directory: &Path, name: &str, body: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, document(body)).expect("a fixture should be writable");
    path
}

/// A throwaway directory that removes itself.
///
/// Tests write a fixture and an output PDF somewhere. `std::env::temp_dir()` is
/// shared with every other process on the machine, so the name carries the
/// process id and the directory is emptied on the way in as well as on the way
/// out: a run killed part-way through otherwise leaves a directory that the next
/// run with the same pid inherits, complete with a stale PDF that a test could
/// mistake for one it just produced.
#[derive(Debug)]
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rchtmltopdf-conformance-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory should be creatable");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A swap guard for the vendored file.
    ///
    /// Not a security control and not the SHA-256 in the README: hashing would
    /// mean a digest dependency for one assertion. This is FNV-1a, which is
    /// ample for catching the thing that actually happens — the font being
    /// regenerated, re-subset or upgraded by someone who has not read why it is
    /// pinned, silently moving every page count in the suite.
    #[test]
    fn font_is_the_vendored_one() {
        fn fnv1a(bytes: &[u8]) -> u64 {
            bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        }

        assert_eq!(
            FONT.len(),
            13_120,
            "size changed; see fixtures/fonts/README.md"
        );
        assert_eq!(
            fnv1a(FONT),
            0xcfb6_0170_86bb_8818,
            "the vendored font changed; see fixtures/fonts/README.md"
        );
        assert_eq!(&FONT[..4], b"wOF2", "should be a woff2");
    }

    #[test]
    fn a_document_declares_the_font_with_no_fallback() {
        let html = document("<p>hello</p>");
        assert!(html.contains("@font-face"), "{html:.400}");
        assert!(html.contains("data:font/woff2;base64,"));
        assert!(html.contains("<p>hello</p>"));
        // The trap this guards: a stray `, sans-serif` would let a fixture that
        // fails to load the font render anyway, on the runner's own typeface.
        assert!(
            !html.contains("sans-serif") && !html.contains("serif,"),
            "no fallback family may appear"
        );
    }

    /// The charset is the one thing that differs, because it is the one thing
    /// `--encoding` is asked about. Everything else has to match, or the two
    /// fixtures would not be measuring the same document.
    #[test]
    fn the_undeclared_document_differs_only_in_the_declaration() {
        let declared = document("<p>x</p>");
        let undeclared = String::from_utf8(undeclared(b"<p>x</p>")).unwrap();

        assert!(declared.contains("<meta charset=\"utf-8\">"));
        assert!(!undeclared.contains("charset"));
        assert_eq!(
            declared.replace("<meta charset=\"utf-8\">", ""),
            undeclared,
            "the two should be the same document otherwise"
        );
        assert!(undeclared.contains("data:font/woff2;base64,"));
    }

    #[test]
    fn a_scratch_directory_cleans_up_after_itself() {
        let path = {
            let scratch = Scratch::new("selftest");
            std::fs::write(scratch.join("a"), b"x").unwrap();
            scratch.path().to_path_buf()
        };
        assert!(!path.exists(), "the directory should be gone");
    }
}
