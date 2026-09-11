//! A small Chrome DevTools Protocol client, written by hand.
//!
//! Only a handful of the protocol's domains are ever needed here, so this is a
//! few hundred lines rather than a dependency on generated bindings for the
//! whole surface.
//!
//! # The pipe transport
//!
//! Chromium is launched with `--remote-debugging-pipe`. It then reads commands
//! from **file descriptor 3** and writes replies and events to **4**. There is no
//! HTTP server and no port, which is the point: a port has to be allocated,
//! guessed or scanned for, and several conversions running at once race for it.
//!
//! Two things bite when wiring this up, and both belong to the launcher rather
//! than to this module:
//!
//! - Rust marks every descriptor it creates close-on-exec, so the pipe ends have
//!   to be duplicated onto 3 and 4 from inside `pre_exec`. Duplicating clears the
//!   flag on the new descriptor. Skip this and Chromium starts, sees nothing on
//!   its input, and hangs with no error.
//! - `pre_exec` runs between fork and exec, so it must be async-signal-safe.
//!   Duplicate descriptors and nothing else. No allocation, no logging.
//!
//! Do not pass `--remote-debugging-port` as well. The two transports are
//! mutually exclusive and asking for both gets you neither.
//!
//! # Framing
//!
//! Messages are separated by a zero byte, not a newline. See [`framing`].
//!
//! # Sessions
//!
//! Attaching to a target is always flattened, so a session id rides as a
//! top-level field on each message. The legacy alternative nests one message
//! inside another as a string, and is not supported here.

pub mod client;
pub mod framing;
pub mod message;

pub use client::{Client, Events, Session};
pub use framing::MAX_MESSAGE_BYTES;
pub use message::{Event, SessionId};
