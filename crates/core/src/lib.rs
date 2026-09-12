//! Shared model for rchtmltopdf: units, page sizes and exit semantics.
//!
//! This crate holds what every layer agrees on. It knows nothing about
//! Chromium, about PDF encoding, or about the command line.

pub mod document;
pub mod error;
pub mod page_size;
pub mod settings;
pub mod units;

pub use document::{Input, Output, has_url_scheme};
pub use error::{ExitCode, LoadErrorHandling, NetworkError};
pub use page_size::{Orientation, PageDimensions};
pub use settings::{GlobalSettings, ObjectSettings, PageSetup, Settings};
pub use units::{Length, Unit};
