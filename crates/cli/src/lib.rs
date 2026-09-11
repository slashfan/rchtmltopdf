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

pub mod table;
pub mod tokenizer;

pub use table::{OptionSpec, Scope, Support, lookup_long, lookup_short};
pub use tokenizer::{Input, Object, ObjectKind, Occurrence, Output, ParseError, Tokenized, tokenize};
