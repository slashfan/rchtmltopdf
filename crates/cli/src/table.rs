//! The wkhtmltopdf option table.
//!
//! Every option wkhtmltopdf documents is listed here, whether or not we act on
//! it. Recognising an option is what keeps a wrapper such as KnpSnappy from
//! breaking on a flag we have not implemented: unimplemented options warn and
//! are ignored, while a genuinely unknown option is still an error.
//!
//! # Provenance
//!
//! Reconciled against a real **wkhtmltopdf 0.12.6.1 (with patched qt)**, whose
//! verbatim `--extended-help` is committed at
//! `tests/fixtures/wkhtmltopdf-0.12.6.1-extended-help.txt`. `reference_help.rs`
//! holds every entry here to it — long name, short alias, arity, scope and
//! section — so this table can no longer drift from the program it imitates,
//! and an option wkhtmltopdf does not have cannot be added without a test
//! failing.
//!
//! # What `Implemented` claims
//!
//! That an option changes what comes out of the program, and nothing weaker.
//! Sixty per cent of this table once said so falsely: the command line
//! understood the option, filled a field in the settings model, and no part of
//! a conversion ever read the field (#19). `tests/plan.rs` is what holds the
//! marker to a conversion now; the audit that set the current markers is D27.
//!
//! The reconciliation found less than feared, and something worse than expected.
//! All 122 options were present, every short alias was right and every arity was
//! right. But `--cookie-jar` was filed as an object option when it is global,
//! `--redirect-delay` was listed and does not exist in 0.12.6.1, and the
//! *placement* rules turned out to be three rules rather than one (D26).

use std::fmt;

/// Whether an option applies to the whole run or to one object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Applies once to the whole document, wherever it appears on the line.
    Global,
    /// Applies to the object it follows, or to every object if it appears
    /// before the first one.
    Object,
    /// Applies to the table of contents object.
    Toc,
}

/// What we actually do with an option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// Honoured end to end: writing it changes what comes out.
    ///
    /// Not the weaker "the command line understands it", which is what this
    /// marker used to mean without saying so. It was wrong about thirty-eight
    /// options: each filled a settings field, nothing downstream read the field,
    /// and the help advertised the option as working (#19). `tests/plan.rs`
    /// holds every one of these to the conversion it would actually run, so the
    /// claim cannot go back to being a hope.
    Implemented,
    /// Recognised and ignored for now, with a warning. Intended to work later.
    ///
    /// Several of these are understood as far as the settings model and stop
    /// there. That is a half-built option rather than a broken one — the model
    /// is where the browser layer's work lands — and it stays honest only while
    /// nothing downstream reads the field.
    Planned(&'static str),
    /// Recognised and ignored, with a warning. Chromium offers no equivalent,
    /// so this is not expected to change.
    NoEquivalent(&'static str),
    /// Handled before any conversion happens: help, version, and friends.
    Meta,
    /// Ours, not wkhtmltopdf's.
    Extension,
}

impl Support {
    /// The stderr line to emit when this option is used, if any.
    ///
    /// Silence here is the common case: implemented options say nothing.
    pub fn warning(self, as_written: &str) -> Option<String> {
        match self {
            Support::Implemented | Support::Meta | Support::Extension => None,
            Support::Planned(reason) => Some(format!(
                "{as_written} is accepted but not implemented yet ({reason}); it is being ignored"
            )),
            Support::NoEquivalent(reason) => Some(format!(
                "{as_written} is accepted for compatibility but has no Chromium equivalent ({reason}); it is being ignored"
            )),
        }
    }
}

/// One option: how it is written, how many values it takes, what we do with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionSpec {
    /// Long name without the leading dashes.
    pub long: &'static str,
    /// Single-letter alias, if wkhtmltopdf gives one.
    pub short: Option<char>,
    pub scope: Scope,
    pub support: Support,
    /// Placeholder names for the values, one per value taken.
    pub value_names: &'static [&'static str],
    /// Whether repeating the option accumulates rather than overwrites.
    pub repeatable: bool,
    pub help: &'static str,
}

impl OptionSpec {
    const fn new(
        long: &'static str,
        short: Option<char>,
        scope: Scope,
        support: Support,
        value_names: &'static [&'static str],
        help: &'static str,
    ) -> Self {
        Self {
            long,
            short,
            scope,
            support,
            value_names,
            repeatable: false,
            help,
        }
    }

    /// An option taking no value.
    const fn flag(
        long: &'static str,
        short: Option<char>,
        scope: Scope,
        support: Support,
        help: &'static str,
    ) -> Self {
        Self::new(long, short, scope, support, &[], help)
    }

    const fn repeats(mut self) -> Self {
        self.repeatable = true;
        self
    }

    /// How many values this option consumes from the command line.
    pub const fn arity(&self) -> usize {
        self.value_names.len()
    }
}

impl fmt::Display for OptionSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "--{}", self.long)
    }
}

use Scope::{Global, Object, Toc};
use Support::{Extension, Implemented, Meta, NoEquivalent, Planned};

const SHRINK: &str = "rendering is fixed at 96 CSS px per inch; see the smart shrinking section of the migration guide";

/// Why the form options have no equivalent: the print path draws a form
/// control as it looks on screen and writes no field behind it — no AcroForm,
/// no widget — so there is nothing to turn on or off (D37).
const FORMS: &str =
    "Chromium's print path draws form fields as they look and makes no interactive fields";
/// What a `Planned` marker names: the milestone that will build the option.
///
/// **A milestone that has shipped must not still be named here.** V2 closed
/// with seventeen options still pointing at it, which is the same kind of
/// claim #19 found in `Support::Implemented` — one nobody had to keep true.
/// They now say what is actually so: nothing is scheduled, and the option is
/// not built. The check that keeps V3 honest is in `plan.rs`.
const UNSCHEDULED: &str = "not built, and not scheduled";

/// The milestones that have shipped.
///
/// A `Planned` marker naming one of these is a promise nobody is left to keep,
/// which is what `a_shipped_milestone_is_not_still_promised` in
/// `tests/plan.rs` exists to catch. Add to this list when a milestone closes,
/// and the test will say which options were still pointing at it.
pub const SHIPPED_MILESTONES: &[&str] = &["V1", "V2", "V3"];

// The V1 surface: understood by the command line, not yet acted on by a
// conversion. Each says what is missing rather than which milestone it waits
// for, because a milestone is not something a user can check and "the band is
// not drawn" is.

/// `General Options` in wkhtmltopdf's extended help. Global scope.
pub const GENERAL_OPTIONS: &[OptionSpec] = &[
    OptionSpec::flag(
        "collate",
        None,
        Global,
        Planned(UNSCHEDULED),
        "Collate when printing multiple copies (default)",
    ),
    OptionSpec::flag(
        "no-collate",
        None,
        Global,
        Planned(UNSCHEDULED),
        "Do not collate when printing multiple copies",
    ),
    OptionSpec::new(
        "cookie-jar",
        None,
        Global,
        Planned(UNSCHEDULED),
        &["path"],
        "Read and write cookies from and to the supplied cookie jar file",
    ),
    OptionSpec::new(
        "copies",
        None,
        Global,
        Planned(UNSCHEDULED),
        &["number"],
        "Number of copies to print into the pdf file (default 1)",
    ),
    OptionSpec::new(
        "dpi",
        Some('d'),
        Global,
        NoEquivalent(SHRINK),
        &["dpi"],
        "Change the dpi explicitly (this has no effect on X11 based systems)",
    ),
    OptionSpec::flag(
        "extended-help",
        Some('H'),
        Global,
        Meta,
        "Display more extensive help, detailing less common command switches",
    ),
    OptionSpec::flag(
        "grayscale",
        Some('g'),
        Global,
        NoEquivalent("Page.printToPDF has no grayscale mode"),
        "PDF will be generated in grayscale",
    ),
    OptionSpec::flag("help", Some('h'), Global, Meta, "Display help"),
    OptionSpec::flag("htmldoc", None, Global, Meta, "Output program html help"),
    OptionSpec::new(
        "image-dpi",
        None,
        Global,
        NoEquivalent(SHRINK),
        &["integer"],
        "When embedding images scale them down to this dpi (default 600)",
    ),
    OptionSpec::new(
        "image-quality",
        None,
        Global,
        NoEquivalent("Chromium controls image encoding itself"),
        &["integer"],
        "When jpeg compressing images use this quality (default 94)",
    ),
    OptionSpec::flag(
        "license",
        None,
        Global,
        Meta,
        "Output license information and exit",
    ),
    OptionSpec::new(
        "log-level",
        None,
        Global,
        Implemented,
        &["level"],
        "Set log level to: none, error, warn or info (default info)",
    ),
    OptionSpec::flag(
        "lowquality",
        Some('l'),
        Global,
        NoEquivalent("Chromium has no reduced-quality print mode"),
        "Generates lower quality pdf/ps. Useful to shrink the result document space",
    ),
    OptionSpec::flag("manpage", None, Global, Meta, "Output program man page"),
    OptionSpec::new(
        "margin-bottom",
        Some('B'),
        Global,
        Implemented,
        &["unitreal"],
        "Set the page bottom margin",
    ),
    OptionSpec::new(
        "margin-left",
        Some('L'),
        Global,
        Implemented,
        &["unitreal"],
        "Set the page left margin (default 10mm)",
    ),
    OptionSpec::new(
        "margin-right",
        Some('R'),
        Global,
        Implemented,
        &["unitreal"],
        "Set the page right margin (default 10mm)",
    ),
    OptionSpec::new(
        "margin-top",
        Some('T'),
        Global,
        Implemented,
        &["unitreal"],
        "Set the page top margin",
    ),
    OptionSpec::new(
        "orientation",
        Some('O'),
        Global,
        Implemented,
        &["orientation"],
        "Set orientation to Landscape or Portrait (default Portrait)",
    ),
    OptionSpec::new(
        "page-height",
        None,
        Global,
        Implemented,
        &["unitreal"],
        "Page height",
    ),
    OptionSpec::new(
        "page-size",
        Some('s'),
        Global,
        Implemented,
        &["Size"],
        "Set paper size to: A4, Letter, etc. (default A4)",
    ),
    OptionSpec::new(
        "page-width",
        None,
        Global,
        Implemented,
        &["unitreal"],
        "Page width",
    ),
    OptionSpec::flag(
        "no-pdf-compression",
        None,
        Global,
        Planned(UNSCHEDULED),
        "Do not use lossless compression on pdf objects",
    ),
    OptionSpec::flag(
        "quiet",
        Some('q'),
        Global,
        Implemented,
        "Be less verbose, maintained for backwards compatibility; Same as using --log-level none",
    ),
    OptionSpec::flag(
        "read-args-from-stdin",
        None,
        Global,
        Planned(UNSCHEDULED),
        "Read command line arguments from stdin",
    ),
    OptionSpec::flag("readme", None, Global, Meta, "Output program readme"),
    OptionSpec::new(
        "title",
        None,
        Global,
        Implemented,
        &["text"],
        "The title of the generated pdf file (The title of the first document is used if not specified)",
    ),
    OptionSpec::flag(
        "use-xserver",
        None,
        Global,
        NoEquivalent("headless Chromium needs no X server"),
        "Use the X server (some plugins and other stuff might not work without X11)",
    ),
    OptionSpec::flag(
        "version",
        Some('V'),
        Global,
        Meta,
        "Output version information and exit",
    ),
];

/// `Outline Options`. Global scope, all deferred to V2.
pub const OUTLINE_OPTIONS: &[OptionSpec] = &[
    OptionSpec::flag(
        "dump-default-toc-xsl",
        None,
        Global,
        NoEquivalent(
            "no stylesheet is used, so there is none to dump; the table of contents it \
             described is generated directly (D41)",
        ),
        "Dump the default TOC xsl style sheet to stdout",
    ),
    OptionSpec::new(
        "dump-outline",
        None,
        Global,
        Implemented,
        &["file"],
        "Dump the outline to a file",
    ),
    OptionSpec::flag(
        "outline",
        None,
        Global,
        Implemented,
        "Put an outline into the pdf (default)",
    ),
    OptionSpec::flag(
        "no-outline",
        None,
        Global,
        Implemented,
        "Do not put an outline into the pdf",
    ),
    OptionSpec::new(
        "outline-depth",
        None,
        Global,
        Implemented,
        &["level"],
        "Set the depth of the outline (default 4)",
    ),
];

/// `Page Options`. Per-object scope.
pub const PAGE_OPTIONS: &[OptionSpec] = &[
    OptionSpec::new(
        "allow",
        None,
        Object,
        Implemented,
        &["path"],
        "Allow the file or files from the specified folder to be loaded (repeatable)",
    )
    .repeats(),
    OptionSpec::flag(
        "background",
        None,
        Object,
        Implemented,
        "Do print background (default)",
    ),
    OptionSpec::flag(
        "no-background",
        None,
        Object,
        Implemented,
        "Do not print background",
    ),
    OptionSpec::new(
        "bypass-proxy-for",
        None,
        Object,
        Implemented,
        &["value"],
        "Bypass proxy for host (repeatable)",
    )
    .repeats(),
    OptionSpec::new(
        "cache-dir",
        None,
        Object,
        Implemented,
        &["path"],
        "Web cache directory",
    ),
    OptionSpec::new(
        "checkbox-checked-svg",
        None,
        Object,
        NoEquivalent("Chromium renders form controls itself"),
        &["path"],
        "Use this SVG file when rendering checked checkboxes",
    ),
    OptionSpec::new(
        "checkbox-svg",
        None,
        Object,
        NoEquivalent("Chromium renders form controls itself"),
        &["path"],
        "Use this SVG file when rendering unchecked checkboxes",
    ),
    OptionSpec::new(
        "cookie",
        None,
        Object,
        Implemented,
        &["name", "value"],
        "Set an additional cookie (repeatable), value should be url encoded",
    )
    .repeats(),
    OptionSpec::new(
        "custom-header",
        None,
        Object,
        Implemented,
        &["name", "value"],
        "Set an additional HTTP header (repeatable)",
    )
    .repeats(),
    OptionSpec::flag(
        "custom-header-propagation",
        None,
        Object,
        Implemented,
        "Add HTTP headers specified by --custom-header for each resource request",
    ),
    OptionSpec::flag(
        "no-custom-header-propagation",
        None,
        Object,
        Implemented,
        "Do not add HTTP headers specified by --custom-header for each resource request",
    ),
    OptionSpec::flag(
        "debug-javascript",
        None,
        Object,
        Planned(UNSCHEDULED),
        "Show javascript debugging output",
    ),
    OptionSpec::flag(
        "no-debug-javascript",
        None,
        Object,
        Planned(UNSCHEDULED),
        "Do not show javascript debugging output (default)",
    ),
    OptionSpec::flag(
        "default-header",
        None,
        Object,
        Implemented,
        "Add a default header, with the name of the page to the left, and the page number to the right",
    ),
    OptionSpec::new(
        "encoding",
        None,
        Object,
        Implemented,
        &["encoding"],
        "Set the default text encoding, for input",
    ),
    OptionSpec::flag(
        "disable-external-links",
        None,
        Object,
        Implemented,
        "Do not make links to remote web pages",
    ),
    OptionSpec::flag(
        "enable-external-links",
        None,
        Object,
        Implemented,
        "Make links to remote web pages (default)",
    ),
    OptionSpec::flag(
        "disable-forms",
        None,
        Object,
        NoEquivalent(FORMS),
        "Do not turn HTML form fields into pdf form fields (default)",
    ),
    OptionSpec::flag(
        "enable-forms",
        None,
        Object,
        NoEquivalent(FORMS),
        "Turn HTML form fields into pdf form fields",
    ),
    OptionSpec::flag(
        "images",
        None,
        Object,
        Implemented,
        "Do load or print images (default)",
    ),
    OptionSpec::flag(
        "no-images",
        None,
        Object,
        Implemented,
        "Do not load or print images",
    ),
    OptionSpec::flag(
        "disable-internal-links",
        None,
        Object,
        Implemented,
        "Do not make local links",
    ),
    OptionSpec::flag(
        "enable-internal-links",
        None,
        Object,
        Implemented,
        "Make local links (default)",
    ),
    OptionSpec::flag(
        "disable-javascript",
        Some('n'),
        Object,
        Implemented,
        "Do not allow web pages to run javascript",
    ),
    OptionSpec::flag(
        "enable-javascript",
        None,
        Object,
        Implemented,
        "Do allow web pages to run javascript (default)",
    ),
    OptionSpec::new(
        "javascript-delay",
        None,
        Object,
        Implemented,
        &["msec"],
        "Wait some milliseconds for javascript finish (default 200)",
    ),
    OptionSpec::flag(
        "keep-relative-links",
        None,
        Object,
        Implemented,
        "Keep relative external links as relative external links",
    ),
    OptionSpec::new(
        "load-error-handling",
        None,
        Object,
        Implemented,
        &["handler"],
        "Specify how to handle pages that fail to load: abort, ignore or skip (default abort)",
    ),
    OptionSpec::new(
        "load-media-error-handling",
        None,
        Object,
        Implemented,
        &["handler"],
        "Specify how to handle media files that fail to load: abort, ignore or skip (default ignore)",
    ),
    OptionSpec::flag(
        "disable-local-file-access",
        None,
        Object,
        Implemented,
        "Do not allow conversion of a local file to read in other local files, unless explicitly allowed with --allow (default)",
    ),
    OptionSpec::flag(
        "enable-local-file-access",
        None,
        Object,
        Implemented,
        "Allow conversion of a local file to read in other local files",
    ),
    OptionSpec::new(
        "minimum-font-size",
        None,
        Object,
        Implemented,
        &["int"],
        "Minimum font size",
    ),
    OptionSpec::flag(
        "exclude-from-outline",
        None,
        Object,
        Implemented,
        "Do not include the page in the table of contents and outlines",
    ),
    OptionSpec::flag(
        "include-in-outline",
        None,
        Object,
        Implemented,
        "Include the page in the table of contents and outlines (default)",
    ),
    OptionSpec::new(
        "page-offset",
        None,
        Object,
        Implemented,
        &["offset"],
        "Set the starting page number (default 0)",
    ),
    OptionSpec::new(
        "password",
        None,
        Object,
        Implemented,
        &["password"],
        "HTTP Authentication password",
    ),
    OptionSpec::flag(
        "disable-plugins",
        None,
        Object,
        NoEquivalent("Chromium has no NPAPI plugin support"),
        "Disable installed plugins (default)",
    ),
    OptionSpec::flag(
        "enable-plugins",
        None,
        Object,
        NoEquivalent("Chromium has no NPAPI plugin support"),
        "Enable installed plugins (plugins will likely not work)",
    ),
    OptionSpec::new(
        "post",
        None,
        Object,
        Planned(UNSCHEDULED),
        &["name", "value"],
        "Add an additional post field (repeatable)",
    )
    .repeats(),
    OptionSpec::new(
        "post-file",
        None,
        Object,
        Planned(UNSCHEDULED),
        &["name", "path"],
        "Post an additional file (repeatable)",
    )
    .repeats(),
    OptionSpec::flag(
        "print-media-type",
        None,
        Object,
        Implemented,
        "Use print media-type instead of screen",
    ),
    OptionSpec::flag(
        "no-print-media-type",
        None,
        Object,
        Implemented,
        "Do not use print media-type instead of screen (default)",
    ),
    OptionSpec::new(
        "proxy",
        Some('p'),
        Object,
        Implemented,
        &["proxy"],
        "Use a proxy",
    ),
    OptionSpec::flag(
        "proxy-hostname-lookup",
        None,
        Object,
        Planned(UNSCHEDULED),
        "Use the proxy for resolving hostnames",
    ),
    OptionSpec::new(
        "radiobutton-checked-svg",
        None,
        Object,
        NoEquivalent("Chromium renders form controls itself"),
        &["path"],
        "Use this SVG file when rendering checked radiobuttons",
    ),
    OptionSpec::new(
        "radiobutton-svg",
        None,
        Object,
        NoEquivalent("Chromium renders form controls itself"),
        &["path"],
        "Use this SVG file when rendering unchecked radiobuttons",
    ),
    OptionSpec::flag(
        "resolve-relative-links",
        None,
        Object,
        Implemented,
        "Resolve relative external links into absolute links (default)",
    ),
    OptionSpec::new(
        "run-script",
        None,
        Object,
        Implemented,
        &["js"],
        "Run this additional javascript after the page is done loading (repeatable)",
    )
    .repeats(),
    OptionSpec::flag(
        "disable-smart-shrinking",
        None,
        Object,
        NoEquivalent(SHRINK),
        "Disable the intelligent shrinking strategy used by WebKit",
    ),
    OptionSpec::flag(
        "enable-smart-shrinking",
        None,
        Object,
        NoEquivalent(SHRINK),
        "Enable the intelligent shrinking strategy used by WebKit (default)",
    ),
    OptionSpec::new(
        "ssl-crt-path",
        None,
        Object,
        Planned(UNSCHEDULED),
        &["path"],
        "Path to the ssl client cert public key in OpenSSL PEM format",
    ),
    OptionSpec::new(
        "ssl-key-password",
        None,
        Object,
        Planned(UNSCHEDULED),
        &["password"],
        "Password to ssl client cert private key",
    ),
    OptionSpec::new(
        "ssl-key-path",
        None,
        Object,
        Planned(UNSCHEDULED),
        &["path"],
        "Path to ssl client cert private key in OpenSSL PEM format",
    ),
    OptionSpec::flag(
        "stop-slow-scripts",
        None,
        Object,
        Planned(UNSCHEDULED),
        "Stop slow running javascripts (default)",
    ),
    OptionSpec::flag(
        "no-stop-slow-scripts",
        None,
        Object,
        Implemented,
        "Do not Stop slow running javascripts",
    ),
    OptionSpec::flag(
        "disable-toc-back-links",
        None,
        Object,
        Implemented,
        "Do not link from section header to toc (default)",
    ),
    OptionSpec::flag(
        "enable-toc-back-links",
        None,
        Object,
        Implemented,
        "Link from section header to toc",
    ),
    OptionSpec::new(
        "user-style-sheet",
        None,
        Object,
        Implemented,
        &["url"],
        "Specify a user style sheet, to load with every page",
    ),
    OptionSpec::new(
        "username",
        None,
        Object,
        Implemented,
        &["username"],
        "HTTP Authentication username",
    ),
    OptionSpec::new(
        "viewport-size",
        None,
        Object,
        Implemented,
        &["size"],
        "Set viewport size if you have custom scrollbars or css attribute overflow to emulate window size",
    ),
    OptionSpec::new(
        "window-status",
        None,
        Object,
        Implemented,
        &["windowStatus"],
        "Wait until window.status is equal to this string before rendering page",
    ),
    OptionSpec::new(
        "zoom",
        None,
        Object,
        Implemented,
        &["float"],
        "Use this zoom factor (default 1)",
    ),
];

/// `Headers And Footer Options`. Per-object scope.
pub const HEADER_FOOTER_OPTIONS: &[OptionSpec] = &[
    OptionSpec::new(
        "footer-center",
        None,
        Object,
        Implemented,
        &["text"],
        "Centered footer text",
    ),
    OptionSpec::new(
        "footer-font-name",
        None,
        Object,
        Implemented,
        &["name"],
        "Set footer font name (default Arial)",
    ),
    OptionSpec::new(
        "footer-font-size",
        None,
        Object,
        Implemented,
        &["size"],
        "Set footer font size (default 12)",
    ),
    OptionSpec::new(
        "footer-html",
        None,
        Object,
        Implemented,
        &["url"],
        "Adds a html footer",
    ),
    OptionSpec::new(
        "footer-left",
        None,
        Object,
        Implemented,
        &["text"],
        "Left aligned footer text",
    ),
    OptionSpec::flag(
        "footer-line",
        None,
        Object,
        Implemented,
        "Display line above the footer",
    ),
    OptionSpec::flag(
        "no-footer-line",
        None,
        Object,
        Implemented,
        "Do not display line above the footer (default)",
    ),
    OptionSpec::new(
        "footer-right",
        None,
        Object,
        Implemented,
        &["text"],
        "Right aligned footer text",
    ),
    OptionSpec::new(
        "footer-spacing",
        None,
        Object,
        Implemented,
        &["real"],
        "Spacing between footer and content in mm (default 0)",
    ),
    OptionSpec::new(
        "header-center",
        None,
        Object,
        Implemented,
        &["text"],
        "Centered header text",
    ),
    OptionSpec::new(
        "header-font-name",
        None,
        Object,
        Implemented,
        &["name"],
        "Set header font name (default Arial)",
    ),
    OptionSpec::new(
        "header-font-size",
        None,
        Object,
        Implemented,
        &["size"],
        "Set header font size (default 12)",
    ),
    OptionSpec::new(
        "header-html",
        None,
        Object,
        Implemented,
        &["url"],
        "Adds a html header",
    ),
    OptionSpec::new(
        "header-left",
        None,
        Object,
        Implemented,
        &["text"],
        "Left aligned header text",
    ),
    OptionSpec::flag(
        "header-line",
        None,
        Object,
        Implemented,
        "Display line below the header",
    ),
    OptionSpec::flag(
        "no-header-line",
        None,
        Object,
        Implemented,
        "Do not display line below the header (default)",
    ),
    OptionSpec::new(
        "header-right",
        None,
        Object,
        Implemented,
        &["text"],
        "Right aligned header text",
    ),
    OptionSpec::new(
        "header-spacing",
        None,
        Object,
        Implemented,
        &["real"],
        "Spacing between header and content in mm (default 0)",
    ),
    OptionSpec::new(
        "replace",
        None,
        Object,
        Implemented,
        &["name", "value"],
        "Replace [name] with value in header and footer (repeatable)",
    )
    .repeats(),
];

/// `TOC Options`. Applies to the table of contents object.
pub const TOC_OPTIONS: &[OptionSpec] = &[
    OptionSpec::flag(
        "disable-dotted-lines",
        None,
        Toc,
        Implemented,
        "Do not use dotted lines in the toc",
    ),
    OptionSpec::new(
        "toc-header-text",
        None,
        Toc,
        Implemented,
        &["text"],
        "The header text of the toc (default Table of Contents)",
    ),
    OptionSpec::new(
        "toc-level-indentation",
        None,
        Toc,
        Implemented,
        &["width"],
        "For each level of headings in the toc indent by this length (default 1em)",
    ),
    OptionSpec::flag(
        "disable-toc-links",
        None,
        Toc,
        Implemented,
        "Do not link from toc to sections",
    ),
    OptionSpec::new(
        "toc-text-size-shrink",
        None,
        Toc,
        Implemented,
        &["real"],
        "For each level of headings in the toc the font is scaled by this factor (default 0.8)",
    ),
    OptionSpec::new(
        "xsl-style-sheet",
        None,
        Toc,
        NoEquivalent(
            "the table of contents is generated rather than transformed: Rust has no \
             XSLT engine, and Chromium's is removed in Chrome 158 (D41)",
        ),
        &["file"],
        "Use the supplied xsl style sheet for printing the table of content",
    ),
];

/// Options of our own that wkhtmltopdf never had.
pub const EXTENSION_OPTIONS: &[OptionSpec] = &[
    OptionSpec::flag(
        "dump-chromium",
        None,
        Global,
        Meta,
        "Report which browser would be used and where it was found, then exit",
    ),
    OptionSpec::new(
        "chromium-path",
        None,
        Global,
        Extension,
        &["path"],
        "Path to the Chromium or chrome-headless-shell binary to drive",
    ),
    OptionSpec::new(
        "chromium-arg",
        None,
        Global,
        Extension,
        &["arg"],
        "Pass an extra command line flag to Chromium (repeatable)",
    )
    .repeats(),
    OptionSpec::flag(
        "no-sandbox",
        None,
        Global,
        Extension,
        "Launch Chromium without its sandbox, for containers that cannot provide one",
    ),
    OptionSpec::new(
        "timeout",
        None,
        Global,
        Extension,
        &["seconds"],
        "Abort the whole conversion after this many seconds (default 30, 0 disables)",
    ),
    OptionSpec::flag(
        "dump-parse",
        None,
        Global,
        Extension,
        "Print how the command line was understood, then exit without converting",
    ),
];

/// The table, section by section, in the order wkhtmltopdf's help prints them.
pub const SECTIONS: &[(&str, &[OptionSpec])] = &[
    ("General Options", GENERAL_OPTIONS),
    ("Outline Options", OUTLINE_OPTIONS),
    ("Page Options", PAGE_OPTIONS),
    ("Headers And Footer Options", HEADER_FOOTER_OPTIONS),
    ("TOC Options", TOC_OPTIONS),
    ("rchtmltopdf Options", EXTENSION_OPTIONS),
];

/// Every option in the table.
pub fn all() -> impl Iterator<Item = &'static OptionSpec> {
    SECTIONS.iter().flat_map(|(_, options)| options.iter())
}

/// Find an option by its long name, without the leading dashes.
pub fn lookup_long(name: &str) -> Option<&'static OptionSpec> {
    all().find(|spec| spec.long == name)
}

/// Find an option by its single-letter alias.
pub fn lookup_short(letter: char) -> Option<&'static OptionSpec> {
    all().find(|spec| spec.short == Some(letter))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn long_names_are_unique() {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for spec in all() {
            *seen.entry(spec.long).or_insert(0) += 1;
        }
        let duplicates: Vec<&str> = seen
            .iter()
            .filter(|(_, count)| **count > 1)
            .map(|(name, _)| *name)
            .collect();
        assert!(
            duplicates.is_empty(),
            "duplicate long names: {duplicates:?}"
        );
    }

    #[test]
    fn short_flags_are_unique() {
        let mut seen: HashMap<char, Vec<&str>> = HashMap::new();
        for spec in all() {
            if let Some(letter) = spec.short {
                seen.entry(letter).or_default().push(spec.long);
            }
        }
        let clashes: Vec<_> = seen.iter().filter(|(_, names)| names.len() > 1).collect();
        assert!(clashes.is_empty(), "short flag used twice: {clashes:?}");
    }

    #[test]
    fn long_names_have_no_dashes_prefix_and_no_spaces() {
        for spec in all() {
            assert!(
                !spec.long.starts_with('-'),
                "{} has a dash prefix",
                spec.long
            );
            assert!(!spec.long.contains(' '), "{} contains a space", spec.long);
            assert!(!spec.long.is_empty());
        }
    }

    #[test]
    fn arity_never_exceeds_two() {
        for spec in all() {
            assert!(
                spec.arity() <= 2,
                "{} takes {} values",
                spec.long,
                spec.arity()
            );
        }
    }

    #[test]
    fn only_options_taking_values_can_repeat() {
        for spec in all() {
            if spec.repeatable {
                assert!(spec.arity() > 0, "{} repeats but takes no value", spec.long);
            }
        }
    }

    #[test]
    fn scope_matches_section() {
        for (section, options) in SECTIONS {
            for spec in *options {
                let expected = match *section {
                    "General Options" | "Outline Options" | "rchtmltopdf Options" => Scope::Global,
                    "Page Options" | "Headers And Footer Options" => Scope::Object,
                    "TOC Options" => Scope::Toc,
                    other => panic!("unknown section {other}"),
                };
                assert_eq!(spec.scope, expected, "{} has the wrong scope", spec.long);
            }
        }
    }

    #[test]
    fn lookup_finds_known_options() {
        assert_eq!(lookup_long("page-size").unwrap().arity(), 1);
        assert_eq!(lookup_long("cookie").unwrap().arity(), 2);
        assert_eq!(lookup_long("quiet").unwrap().arity(), 0);
        assert_eq!(lookup_short('s').unwrap().long, "page-size");
        assert_eq!(lookup_short('T').unwrap().long, "margin-top");
        assert!(lookup_long("no-such-option").is_none());
        assert!(lookup_short('z').is_none());
    }

    /// The V1 scope list, straight from `docs/brief.md`.
    ///
    /// What this can assert is that the promise is still keepable: the option
    /// exists, and the table does not declare it impossible. It used to assert
    /// they were all implemented, and passed while most of them did nothing at
    /// all, because nothing here can see past the settings model (#19).
    /// `tests/plan.rs` is where `Implemented` is held to something.
    #[test]
    fn every_option_the_brief_promises_for_v1_is_still_keepable() {
        let promised = [
            "page-size",
            "page-width",
            "page-height",
            "margin-top",
            "margin-right",
            "margin-bottom",
            "margin-left",
            "orientation",
            "zoom",
            "javascript-delay",
            "cookie",
            "custom-header",
            "username",
            "password",
            "user-style-sheet",
            "print-media-type",
            "enable-local-file-access",
            "footer-center",
            "footer-left",
            "footer-right",
            "header-center",
            "header-left",
            "header-right",
        ];
        for name in promised {
            let spec = lookup_long(name).unwrap_or_else(|| panic!("{name} missing from the table"));
            assert!(
                matches!(spec.support, Support::Implemented | Support::Planned(_)),
                "{name} is promised for V1 and the table calls it {:?}, \
                 which says the promise cannot be kept",
                spec.support
            );
        }
    }

    #[test]
    fn unimplemented_options_explain_themselves() {
        for spec in all() {
            match spec.support {
                Support::Planned(reason) | Support::NoEquivalent(reason) => {
                    assert!(!reason.is_empty(), "{} has an empty reason", spec.long);
                    let warning = spec.support.warning("--x").unwrap();
                    assert!(warning.contains(reason));
                }
                _ => assert!(spec.support.warning("--x").is_none()),
            }
        }
    }
}
