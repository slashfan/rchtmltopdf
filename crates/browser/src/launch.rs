//! Starting a browser and stopping it again.
//!
//! One browser per conversion, with a throwaway profile, which is the stateless
//! model wkhtmltopdf had and what callers expect (D11). Nothing here assumes it,
//! though: the type is a handle over a connection, so a pooled or long-running
//! mode can be added later without the command line layer noticing.
//!
//! # Placing the pipe on descriptors 3 and 4
//!
//! This is the part the protocol module documents and deliberately does not do.
//! Chromium reads commands from descriptor 3 and writes to 4, and getting them
//! there has two traps.
//!
//! Rust marks every descriptor it opens close-on-exec, so handing one to a child
//! means duplicating it after fork and before exec. `dup2` clears the flag on the
//! descriptor it creates, which is exactly what is wanted. But `dup2(fd, fd)`
//! with the same number on both sides is a no-op that does **not** clear the
//! flag, and a freshly created pipe can easily land on 3 or 4 already. Getting
//! that wrong produces the worst kind of failure: Chromium starts, reports
//! nothing, and hangs forever waiting on an input that was closed out from under
//! it.
//!
//! So both ends are first moved somewhere out of the way, above descriptor 10,
//! and only then placed. That removes the aliasing question entirely, and the
//! intermediate copies keep close-on-exec so they do not leak into the browser.
//!
//! The second trap is that the hook runs between fork and exec, where only
//! async-signal-safe calls are allowed. `fcntl`, `dup2` and `setpgid` all
//! qualify. There is no allocation and no logging in there, and there must not
//! be.

use crate::cdp::Client;
use crate::error::{Error, Result};
use crate::locate::{Executable, Flavour};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

/// How long to wait for a freshly started browser to answer, by default.
///
/// A browser that has not answered in this long on a normal machine is broken,
/// not slow. It is only a backstop: the conversion deadline is the real bound
/// and will usually cut in well before this. Overridable, because a heavily
/// loaded machine starting several browsers at once is genuinely slower than
/// one that is not.
const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

/// How much of the browser's own diagnostics to keep for error reports.
const STDERR_KEPT_BYTES: usize = 16 * 1024;

/// Flags every launch gets.
///
/// Deliberately short. Each one is here to stop the browser doing something a
/// one-shot, headless, throwaway-profile render has no use for: phoning home,
/// syncing, recording metrics, playing audio, drawing scrollbars into the
/// output.
const BASELINE_FLAGS: &[&str] = &[
    "--remote-debugging-pipe",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-sync",
    "--metrics-recording-only",
    "--mute-audio",
    "--disable-dev-shm-usage",
    "--hide-scrollbars",
    "--disable-gpu",
];

/// What to start, and how.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// Run without the sandbox.
    ///
    /// Never the default. The threat model here is untrusted HTML (D10), and an
    /// unsandboxed browser rendering an attacker's invoice is the thing being
    /// protected against. Containers that genuinely cannot provide a sandbox opt
    /// in explicitly.
    pub no_sandbox: bool,
    /// Extra flags, appended verbatim after the baseline so a caller can
    /// override any of it.
    pub extra_args: Vec<String>,
    /// Use this profile directory instead of a throwaway one.
    pub user_data_dir: Option<PathBuf>,
    /// Override how long to wait for the browser to answer after starting.
    pub handshake_timeout: Option<Duration>,
}

/// A directory that deletes itself.
///
/// Rolled by hand rather than pulled in, because the cleanup has to happen on
/// every exit path including a timeout, and owning it makes that explicit.
#[derive(Debug)]
struct TempProfile {
    path: PathBuf,
}

impl TempProfile {
    fn create() -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let base = std::env::temp_dir();
        for _ in 0..16 {
            let unique = format!(
                "rchtmltopdf-{}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos())
                    .unwrap_or(0)
            );
            let path = base.join(unique);
            // create_dir fails when the name is taken, so this is the collision
            // check as well as the creation.
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not create a unique temporary profile directory",
        ))
    }
}

impl Drop for TempProfile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A running browser.
pub struct Browser {
    client: Client,
    child: Child,
    /// Kept so it is removed when this is dropped. `None` when the caller
    /// supplied their own profile directory, which is theirs to manage.
    _profile: Option<TempProfile>,
    /// The process group, so the whole tree can be stopped rather than just the
    /// browser process, which spawns several children of its own.
    process_group: Option<i32>,
    diagnostics: Arc<Mutex<String>>,
}

impl Browser {
    /// Start a browser and wait until it answers.
    ///
    /// Returning successfully means the connection works, not merely that a
    /// process was spawned. A browser that starts and immediately dies, which is
    /// what a missing library or a refused sandbox looks like, fails here with
    /// its own diagnostics attached rather than at the first command.
    pub async fn launch(executable: &Executable, options: &LaunchOptions) -> Result<Self> {
        let profile = match &options.user_data_dir {
            Some(_) => None,
            None => Some(TempProfile::create()?),
        };
        let profile_path = options
            .user_data_dir
            .clone()
            .unwrap_or_else(|| profile.as_ref().expect("just created").path.clone());

        let mut command = std::process::Command::new(&executable.path);
        command.arg(format!("--user-data-dir={}", profile_path.display()));
        command.args(BASELINE_FLAGS);

        // The shell is already headless; telling it to be headless is an error.
        // A full browser has to be told.
        if executable.flavour == Flavour::FullBrowser {
            command.arg("--headless=new");
        }
        if options.no_sandbox {
            command.arg("--no-sandbox");
        }
        // Last, so a caller can override anything above.
        command.args(&options.extra_args);

        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::piped());

        let (to_browser, from_browser) = attach_pipes(&mut command)?;

        let mut child = Command::from(command).spawn().map_err(|error| {
            Error::Io(std::io::Error::new(
                error.kind(),
                format!("could not start {}: {error}", executable.path.display()),
            ))
        })?;

        let process_group = child.id().map(|id| id as i32);
        let diagnostics = capture_stderr(&mut child);
        let client = Client::new(from_browser, to_browser);

        let browser = Self {
            client,
            child,
            _profile: profile,
            process_group,
            diagnostics,
        };

        browser
            .handshake(
                executable,
                options
                    .handshake_timeout
                    .unwrap_or(DEFAULT_HANDSHAKE_TIMEOUT),
            )
            .await?;
        Ok(browser)
    }

    /// Confirm the browser is really talking before handing it to a caller.
    async fn handshake(&self, executable: &Executable, patience: Duration) -> Result<()> {
        let version = tokio::time::timeout(
            patience,
            self.client.send("Browser.getVersion", Value::Null),
        )
        .await;

        match version {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(self.explain(executable, error)),
            Err(_) => Err(self.explain(
                executable,
                Error::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "{} did not answer within {}s",
                        executable.path.display(),
                        patience.as_secs()
                    ),
                )),
            )),
        }
    }

    /// Turn a launch failure into something actionable.
    ///
    /// The browser's own stderr is where a missing shared library or a refused
    /// sandbox actually shows up, so it goes in the message. A refused sandbox
    /// gets the specific suggestion, because it is the single most common thing
    /// to hit when moving an existing deployment into a container.
    fn explain(&self, executable: &Executable, cause: Error) -> Error {
        let output = self.diagnostics.lock().unwrap().clone();
        let mut detail = format!("{} failed to start: {cause}", executable.path.display());

        if !output.trim().is_empty() {
            detail.push_str("\nthe browser said:\n");
            for line in output.lines().take(20) {
                detail.push_str("  ");
                detail.push_str(line);
                detail.push('\n');
            }
        }

        let lowered = output.to_lowercase();
        if lowered.contains("sandbox") {
            detail.push_str(
                "\nThis looks like the sandbox being refused, which is usual in a container.\n\
                 Retry with --no-sandbox, but only for HTML you trust: the sandbox is what\n\
                 contains a hostile document.",
            );
        }

        Error::Launch { detail }
    }

    /// The underlying connection, for commands aimed at the browser itself.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Open a page and attach to it.
    pub async fn new_page(&self) -> Result<Page> {
        let created = self
            .client
            .send("Target.createTarget", json!({ "url": "about:blank" }))
            .await?;

        let target_id = created
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Malformed {
                detail: "Target.createTarget replied without a targetId".into(),
            })?
            .to_string();

        let session = self.client.attach_to_target(&target_id).await?;
        Ok(Page { session, target_id })
    }

    /// Ask the browser to exit, and wait for it.
    ///
    /// Dropping works too, but abruptly. Prefer this where the outcome matters.
    pub async fn close(mut self) -> Result<()> {
        let _ = self.client.send("Browser.close", Value::Null).await;
        if tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .is_ok()
        {
            // Reaped. The identifier is now the kernel's to hand out again, so
            // Drop must not signal it.
            self.process_group = None;
        }
        Ok(())
    }

    /// The browser's process identifier, while it is running.
    pub fn process_id(&self) -> Option<u32> {
        self.child.id()
    }

    /// The throwaway profile directory, when this browser owns one.
    ///
    /// `None` when the caller supplied their own, which is theirs to manage.
    pub fn profile_path(&self) -> Option<&std::path::Path> {
        self._profile.as_ref().map(|profile| profile.path.as_path())
    }

    /// Whatever the browser has written to its own error stream so far.
    pub fn diagnostics(&self) -> String {
        self.diagnostics.lock().unwrap().clone()
    }
}

impl Drop for Browser {
    /// Stop the whole process tree, not just the browser process.
    ///
    /// Chromium spawns children, and killing only the parent leaves them behind.
    /// The launch puts everything in its own process group precisely so this can
    /// be one call. The temporary profile goes with it, through `TempProfile`.
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(group) = self.process_group {
            // Negative pid means the process group.
            unsafe { libc::kill(-group, libc::SIGKILL) };
        }
        let _ = self.child.start_kill();
    }
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser")
            .field("pid", &self.child.id())
            .finish_non_exhaustive()
    }
}

/// One page in a running browser.
#[derive(Debug)]
pub struct Page {
    session: crate::cdp::Session,
    target_id: String,
}

impl Page {
    /// Build a page over an existing session.
    ///
    /// Exists so the printing and stream-reading paths can be driven by the
    /// stand-in browser the protocol tests already use. Without it those error
    /// branches are unreachable without a real Chromium, which means in practice
    /// they are never exercised at all: a browser cannot be asked to return
    /// invalid base64, or to stop a stream without ending it.
    #[doc(hidden)]
    pub fn over_session(session: crate::cdp::Session, target_id: impl Into<String>) -> Self {
        Self {
            session,
            target_id: target_id.into(),
        }
    }

    pub fn session(&self) -> &crate::cdp::Session {
        &self.session
    }

    pub fn target_id(&self) -> &str {
        &self.target_id
    }
}

impl Drop for Page {
    /// Give the target back.
    ///
    /// Costs nothing in the one-browser-per-conversion model, where the whole
    /// process is about to be killed anyway. It matters for the pooled model
    /// D11 is keeping the door open for, where a page left open is a renderer
    /// process leaked per conversion.
    ///
    /// Sent without waiting: there is no caller left to report to, and if the
    /// connection has already gone the command is simply dropped.
    fn drop(&mut self) {
        self.session
            .send_detached("Target.closeTarget", json!({ "targetId": self.target_id }));
    }
}

/// Read the browser's error stream into a capped buffer.
///
/// Draining it matters beyond diagnostics: an undrained pipe fills, and a
/// browser blocked writing to it stops doing anything else.
fn capture_stderr(child: &mut Child) -> Arc<Mutex<String>> {
    let collected = Arc::new(Mutex::new(String::new()));
    let Some(mut stderr) = child.stderr.take() else {
        return collected;
    };
    let sink = Arc::clone(&collected);

    tokio::spawn(async move {
        let mut chunk = [0u8; 4096];
        loop {
            match stderr.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let mut held = sink.lock().unwrap();
                    held.push_str(&String::from_utf8_lossy(&chunk[..read]));
                    if held.len() > STDERR_KEPT_BYTES {
                        // Keep the end: the failure is what came last.
                        let cut = held.len() - STDERR_KEPT_BYTES;
                        let boundary = (cut..held.len())
                            .find(|i| held.is_char_boundary(*i))
                            .unwrap_or(held.len());
                        *held = held[boundary..].to_string();
                    }
                }
            }
        }
    });

    collected
}

#[cfg(unix)]
fn attach_pipes(
    command: &mut std::process::Command,
) -> Result<(
    tokio::net::unix::pipe::Sender,
    tokio::net::unix::pipe::Receiver,
)> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    // Commands travel one way, replies and events the other.
    let (browser_reads, we_write) = new_pipe()?;
    let (we_read, browser_writes) = new_pipe()?;

    // The browser's ends are moved into the hook, which is what keeps them open
    // until the fork and closes them in this process straight after.
    //
    // They must NOT be closed before spawning. Doing that leaves the child with
    // nothing on 3 and 4, and worse, the descriptor numbers can be handed
    // straight back out to something else in the meantime. The symptom is a
    // broken pipe on the first write, with no hint as to why.

    // SAFETY: the hook runs between fork and exec. Everything it calls is
    // async-signal-safe, and it neither allocates nor touches shared state.
    unsafe {
        command.pre_exec(move || {
            place_descriptor(browser_reads.as_raw_fd(), 3)?;
            place_descriptor(browser_writes.as_raw_fd(), 4)?;
            // Its own process group, so the whole tree can be stopped at once.
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let sender = tokio::net::unix::pipe::Sender::from_owned_fd(we_write)?;
    let receiver = tokio::net::unix::pipe::Receiver::from_owned_fd(we_read)?;

    Ok((sender, receiver))
}

/// Put `fd` on exactly `target`, with close-on-exec cleared.
///
/// Goes via a copy above descriptor 10 rather than duplicating straight onto the
/// target. Two reasons, both of which produce a browser that hangs silently
/// rather than an error: `dup2` onto the same number it was already on is a
/// no-op that leaves close-on-exec set, and duplicating onto 3 could clobber the
/// other end if it happens to live there.
///
/// Async-signal-safe: `fcntl` and `dup2` only.
#[cfg(unix)]
fn place_descriptor(fd: libc::c_int, target: libc::c_int) -> std::io::Result<()> {
    // SAFETY: called between fork and exec with a valid descriptor.
    unsafe {
        let parked = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10);
        if parked == -1 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::dup2(parked, target) == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// A pipe whose ends do not leak into unrelated children.
#[cfg(unix)]
fn new_pipe() -> std::io::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::{FromRawFd, OwnedFd};

    let mut ends = [0 as libc::c_int; 2];

    #[cfg(target_os = "linux")]
    // SAFETY: `ends` is a valid array of two descriptors.
    let created = unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) };

    #[cfg(not(target_os = "linux"))]
    // SAFETY: `ends` is a valid array of two descriptors.
    let created = unsafe {
        let result = libc::pipe(ends.as_mut_ptr());
        if result == 0 {
            for end in ends {
                libc::fcntl(end, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }
        result
    };

    if created != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // SAFETY: both descriptors were just created and are owned here.
    unsafe { Ok((OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1]))) }
}

/// Helpers for testing the descriptor plumbing without a browser.
#[cfg(unix)]
pub mod testing {
    use super::attach_pipes;
    use crate::error::Result;
    use std::process::Stdio;
    use tokio::process::{Child, Command};

    /// Spawn any program with a protocol pipe on descriptors 3 and 4.
    ///
    /// Exists so the plumbing can be proved against something predictable, such
    /// as a shell copying one descriptor to the other, rather than only against
    /// a browser where a failure has many possible causes.
    pub fn spawn_with_pipe(
        program: &str,
        args: &[&str],
    ) -> Result<(
        Child,
        tokio::net::unix::pipe::Sender,
        tokio::net::unix::pipe::Receiver,
    )> {
        let mut command = std::process::Command::new(program);
        command.args(args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::piped());
        let (sender, receiver) = attach_pipes(&mut command)?;
        let child = Command::from(command).spawn()?;
        Ok((child, sender, receiver))
    }
}
