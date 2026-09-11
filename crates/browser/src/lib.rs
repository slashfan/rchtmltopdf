//! Driving a headless Chromium.
//!
//! Today this is the protocol client in [`cdp`]. Launching a browser, waiting
//! for a page to settle and printing it follow.
//!
//! The interface is session-oriented on purpose. One browser per conversion is
//! the current model, but nothing here assumes it, so a pooled or long-running
//! mode can be added later without the command line layer noticing.

pub mod cdp;
pub mod error;
pub mod launch;
pub mod locate;
pub mod print;
pub mod render;

pub use cdp::{Client, Event, Session, SessionId};
pub use error::{Error, ProtocolError, Result};
pub use launch::{Browser, LaunchOptions, Page};
pub use locate::{Executable, Flavour, Origin, Platform, locate};
pub use render::{Progress, Stage};
