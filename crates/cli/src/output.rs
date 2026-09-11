//! Delivering the finished document.
//!
//! # Why a rename rather than a plain write
//!
//! KnpSnappy checks that the output file exists and is not empty, and throws
//! when it is not. A conversion that fails partway through a direct write leaves
//! a short or empty file behind, which reads as a corrupt PDF rather than as a
//! failure. Writing beside the target and renaming means the file at the
//! requested path is either absent or complete, never in between.
//!
//! # Why stdout is kept clear
//!
//! The result can *be* stdout. Anything else printed there ends up inside the
//! PDF, so warnings, progress and errors all go to stderr, and the two places
//! that print to stdout (help and version) answer before any conversion starts.

use rchtmltopdf_core::Output;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct OutputError {
    /// Absent when the failure was on stdout.
    pub path: Option<PathBuf>,
    pub source: std::io::Error,
}

impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "could not write {}: {}", path.display(), self.source),
            None => write!(f, "could not write to standard output: {}", self.source),
        }
    }
}

impl std::error::Error for OutputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Deliver the document.
pub fn write(output: &Output, document: &[u8]) -> Result<(), OutputError> {
    let mut stdout = std::io::stdout().lock();
    write_with(output, document, &mut stdout)
}

/// Deliver the document, with somewhere to send it when the target is stdout.
///
/// The separate stream is what lets this be tested without a terminal.
pub fn write_with(
    output: &Output,
    document: &[u8],
    stdout: &mut dyn Write,
) -> Result<(), OutputError> {
    match output {
        Output::Stdout => stdout
            .write_all(document)
            .and_then(|()| stdout.flush())
            .map_err(|source| OutputError { path: None, source }),
        Output::Path(path) => write_file(path, document),
    }
}

fn write_file(path: &Path, document: &[u8]) -> Result<(), OutputError> {
    let fail = |source| OutputError {
        path: Some(path.to_path_buf()),
        source,
    };

    // Beside the target, so it lands on the same filesystem and the rename is
    // atomic rather than a copy.
    let scratch = scratch_path(path);

    let outcome = std::fs::File::create(&scratch)
        .and_then(|mut file| file.write_all(document).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&scratch, path));

    if let Err(source) = outcome {
        // Leave nothing half-written where somebody might read it.
        let _ = std::fs::remove_file(&scratch);
        return Err(fail(source));
    }
    Ok(())
}

/// A name beside the target that nothing else is using.
fn scratch_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".rchtmltopdf-{}.part", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rch-out-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_receives_exactly_the_bytes_it_was_given() {
        let dir = scratch_dir("exact");
        let target = dir.join("out.pdf");

        // Binary, including a zero byte and something that is not valid UTF-8,
        // because a String round trip would mangle both.
        let document: Vec<u8> = vec![b'%', b'P', b'D', b'F', b'-', 0x00, 0xFF, 0xFE, b'\r', b'\n'];
        write(&Output::Path(target.clone()), &document).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), document);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stdout_receives_exactly_the_bytes_it_was_given() {
        let document = b"%PDF-1.4\x00\xff binary\n";
        let mut sink = Vec::new();
        write_with(&Output::Stdout, document, &mut sink).unwrap();
        assert_eq!(sink, document);
    }

    #[test]
    fn an_existing_file_is_replaced() {
        let dir = scratch_dir("replace");
        let target = dir.join("out.pdf");
        std::fs::write(&target, b"an older, longer document").unwrap();

        write(&Output::Path(target.clone()), b"newer").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"newer");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// KnpSnappy reads a missing file and an empty one as different failures,
    /// and a short one as a corrupt PDF. A failure must leave nothing at all.
    #[test]
    fn a_failure_leaves_nothing_behind() {
        let dir = scratch_dir("fail");
        // A directory where a file is wanted: creating the scratch file fails.
        let target = dir.join("taken");
        std::fs::create_dir(&target).unwrap();

        let error = write(&Output::Path(target.clone()), b"%PDF-").unwrap_err();
        assert!(
            error.to_string().contains("taken"),
            "the message should name the path: {error}"
        );

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "taken")
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unwritable_directory_is_reported_with_the_path() {
        let error = write(&Output::Path("/nowhere/at/all/out.pdf".into()), b"%PDF-").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("/nowhere/at/all/out.pdf"), "{message}");
        assert!(message.starts_with("could not write"), "{message}");
    }

    /// The scratch file sits beside the target so the rename stays on one
    /// filesystem. Somewhere like the system temporary directory could be a
    /// different mount, where a rename becomes a copy and stops being atomic.
    #[test]
    fn the_scratch_file_sits_beside_the_target() {
        let scratch = scratch_path(Path::new("/some/where/out.pdf"));
        assert_eq!(
            scratch.parent(),
            Path::new("/some/where")
                .parent()
                .map(|_| Path::new("/some/where"))
        );
        assert!(
            scratch
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".part"),
            "{scratch:?}"
        );
        assert_ne!(scratch, PathBuf::from("/some/where/out.pdf"));
    }

    #[test]
    fn an_empty_document_still_produces_a_file() {
        // Not our judgement to make here: refusing an empty document belongs to
        // whatever produced it.
        let dir = scratch_dir("empty");
        let target = dir.join("out.pdf");
        write(&Output::Path(target.clone()), b"").unwrap();
        assert!(target.exists());
        assert_eq!(std::fs::metadata(&target).unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
