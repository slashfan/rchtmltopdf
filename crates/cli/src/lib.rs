//! The wkhtmltopdf-compatible command line layer.
//!
//! This is not a conventional CLI. wkhtmltopdf's grammar is
//!
//! ```text
//! wkhtmltopdf [GLOBAL OPTION]... [OBJECT]... <output file>
//! ```
//!
//! where an object is `page <input>`, `cover <input>`, `toc`, or a bare input,
//! and object options attach to whichever object precedes them. Options take
//! zero, one or two values, and a bare number in a length argument means
//! millimetres. A general-purpose argument parser fights all of this, so the
//! grammar is tokenised by hand here.

/// The name the program answers to, and the prefix on every line it writes to
/// stderr. Shared so a diagnostic cannot come out looking like it belongs to a
/// different program depending on which module wrote it.
pub const PROGRAM: &str = "rchtmltopdf";

/// This program's version, as the PDF's Producer will name it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod apply;
pub mod convert;
pub mod input;
pub mod numbering;
pub mod outline;
pub mod output;
pub mod table;
pub mod tokenizer;

pub use apply::{Applied, ApplyError, apply};
pub use table::{OptionSpec, Scope, Support, lookup_long, lookup_short};
pub use tokenizer::{
    Input, Object, ObjectKind, Occurrence, Output, ParseError, Tokenized, find_meta_option,
    tokenize,
};
