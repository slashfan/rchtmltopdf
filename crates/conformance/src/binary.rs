//! Running the built binary as a subprocess.
//!
//! Conversion is only half of what the binary promises. The exit code, which
//! stream each thing is written to, and producing nothing at all on failure are
//! the other half (D14), and none of it is visible to a test that calls a
//! library function. So these tests run the program.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The built `rchtmltopdf`.
///
/// **The trap.** `env!("CARGO_BIN_EXE_rchtmltopdf")` only exists inside test
/// targets of the package that declares that binary, and this is a different
/// package, so the path has to be worked out from where the test binary itself
/// was put: `target/<profile>/deps/<test>` sits one directory below the binary.
///
/// The consequence is that the binary has to have been built. `cargo test
/// --workspace` builds it; `cargo test -p rchtmltopdf-conformance` does not,
/// because cargo builds a dependency's library and not its binaries. Rather than
/// fail with "file not found", say which command is missing.
pub fn path() -> PathBuf {
    let mut directory = std::env::current_exe().expect("a running test has a path");
    directory.pop();
    if directory.ends_with("deps") {
        directory.pop();
    }
    let binary = directory.join(format!("rchtmltopdf{}", std::env::consts::EXE_SUFFIX));

    assert!(
        binary.is_file(),
        "the binary under test is not built:\n  {}\n\n\
         Building the conformance package alone does not build another package's \
         binary. Run `cargo build -p rchtmltopdf` first, or `cargo test --workspace`.",
        binary.display()
    );
    binary
}

/// One invocation of the binary.
#[derive(Debug, Default)]
pub struct Run {
    args: Vec<String>,
    stdin: Vec<u8>,
}

impl Run {
    /// A run that gives up the sandbox when, and only when, the environment has
    /// said it cannot provide one.
    ///
    /// A GitHub runner and a default container both refuse it. The product never
    /// gives it up on its own (D10), so this is passed as the flag a user would
    /// have to pass, not smuggled in through a back door the product does not
    /// have.
    pub fn new() -> Self {
        let mut run = Self::default();
        if crate::sandbox_unavailable() {
            run.args.push("--no-sandbox".into());
        }
        run
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// What to write to the child's standard input.
    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = bytes.into();
        self
    }

    /// Run it, and collect everything.
    ///
    /// The browser is deliberately **not** named on the command line. The child
    /// resolves one through D09's ladder from the same environment this process
    /// read, so the CI job exercises the cache rung that `fetch-chromium` fills
    /// rather than being handed an answer.
    pub fn output(self) -> Outcome {
        // One conversion at a time. Every run cold-starts a Chromium, and a
        // two-core runner asked to start six at once spends its time switching
        // rather than rendering.
        //
        // The honest limit: this is a static, so there is one per test binary
        // rather than one for the crate. It holds because cargo runs test
        // binaries one after another.
        static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _turn = TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        let mut child = Command::new(path())
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the binary should start");

        // Written from a thread rather than inline. A document carrying a font
        // is tens of kilobytes, a pipe buffer is not much more, and a parent
        // that blocks writing stdin while the child blocks writing stdout is a
        // deadlock that would look like a hung test rather than a bug here.
        let mut sink = child.stdin.take().expect("stdin was piped");
        let bytes = self.stdin;
        let writer = std::thread::spawn(move || {
            let _ = sink.write_all(&bytes);
            drop(sink);
        });

        let output = child.wait_with_output().expect("the binary should finish");
        writer.join().expect("the stdin writer should not panic");

        Outcome {
            args: self.args,
            code: output.status.code(),
            stdout: output.stdout,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

/// What one invocation produced.
#[derive(Debug)]
pub struct Outcome {
    /// Kept so a failed assertion can say which command line produced it.
    pub args: Vec<String>,
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Outcome {
    /// Assert the run succeeded, and show what it said if it did not.
    ///
    /// A browser failure is a page of diagnostics and the only way to read them
    /// is for the assertion to print stderr, so it does.
    pub fn succeeded(&self) -> &Self {
        assert_eq!(
            self.code,
            Some(0),
            "`rchtmltopdf {}` failed\n--- stderr ---\n{}",
            self.args.join(" "),
            self.stderr
        );
        self
    }

    /// Assert the run failed, the way KnpSnappy detects failure: a non-zero exit
    /// **and** something on stderr. It is the pair that matters (D14).
    pub fn failed(&self) -> &Self {
        assert_ne!(self.code, Some(0), "expected a failure, got success");
        assert!(
            !self.stderr.trim().is_empty(),
            "a failure must say why on stderr"
        );
        self
    }
}

/// True when these bytes begin a PDF.
///
/// The cheapest structural assertion there is, and the one that catches the
/// failure that matters most: a conversion that "succeeds" and writes an error
/// page, an empty file, or nothing.
pub fn is_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF-")
}
