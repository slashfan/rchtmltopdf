//! Driving a headless Chromium.
//!
//! Today this is the protocol client in [`cdp`]. Launching a browser, waiting
//! for a page to settle and printing it follow.
//!
//! The interface is session-oriented on purpose. One browser per conversion is
//! the current model, but nothing here assumes it, so a pooled or long-running
//! mode can be added later without the command line layer noticing.

// The pipe transport puts the protocol on file descriptors 3 and 4, which is a
// Unix arrangement. Chromium does support a pipe transport on Windows, using
// handles passed a different way, and nothing here implements it. Fail with a
// sentence rather than a page of errors about a missing `attach_pipes`.
#[cfg(not(unix))]
compile_error!(
    "rchtmltopdf-browser supports Unix only: the protocol travels over file \
     descriptors 3 and 4. Windows would need the handle-based transport, which \
     is not written."
);

pub mod band;
pub mod cdp;
pub mod clock;
pub mod deadline;
pub mod error;
pub mod file_access;
pub mod intercept;
pub mod launch;
pub mod locate;
pub mod placeholder;
pub mod plan;
pub mod print;
pub mod render;

pub use cdp::{Client, Event, Session, SessionId};
pub use error::{Error, ProtocolError, Result};
pub use file_access::{FileAccess, Policy, Verdict};
pub use intercept::{Interception, Refusal};
pub use launch::{Browser, LaunchOptions, Page};
pub use locate::{Executable, Flavour, Origin, Platform, locate};
pub use plan::{Command, LoadPlan, Plan, Settle};
pub use render::{Progress, Stage};
