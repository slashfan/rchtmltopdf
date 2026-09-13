//! What a conversion has been asked to do.
//!
//! The command line layer fills this in; the browser and PDF layers read it.
//! Nothing here knows about Chromium, about the protocol, or about how the
//! command line was written, which is what lets a pooled browser be slotted in
//! later without any of it noticing (D11).
//!
//! # The defaults are the point
//!
//! Every value here starts at wkhtmltopdf's default, not Chromium's (D03).
//! Chromium prints Letter with one-centimetre margins using print stylesheets;
//! wkhtmltopdf renders A4 with ten-millimetre margins using screen stylesheets.
//! A document migrated without changing a single option has to come out the way
//! it did before, so these values are load-bearing, and every later assertion is
//! calibrated against them.

use crate::document::{Input, Output};
use crate::error::LoadErrorHandling;
use crate::page_size::{Orientation, PageDimensions};
use crate::units::Length;
use std::path::PathBuf;
use std::time::Duration;

/// How much to say on the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    /// Say nothing at all. What `-q` selects.
    None,
    Error,
    Warn,
    #[default]
    Info,
}

impl LogLevel {
    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "none" => Some(LogLevel::None),
            "error" => Some(LogLevel::Error),
            "warn" => Some(LogLevel::Warn),
            "info" => Some(LogLevel::Info),
            _ => None,
        }
    }

    /// Whether a warning should be shown at this level.
    pub fn shows_warnings(self) -> bool {
        matches!(self, LogLevel::Warn | LogLevel::Info)
    }

    /// Whether the progress lines should be shown.
    ///
    /// Only at `info`, which is the default. They are chatter rather than
    /// diagnosis, and a run that has been asked to say less says less (D14).
    pub fn shows_progress(self) -> bool {
        matches!(self, LogLevel::Info)
    }
}

/// The four margins.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    pub top: Length,
    pub right: Length,
    pub bottom: Length,
    pub left: Length,
}

impl Margins {
    pub const fn uniform(length: Length) -> Self {
        Self {
            top: length,
            right: length,
            bottom: length,
            left: length,
        }
    }
}

impl Default for Margins {
    /// Ten millimetres all round, which is wkhtmltopdf's default.
    fn default() -> Self {
        Self::uniform(Length::mm(10.0))
    }
}

/// Which of the two vertical margins the command line named.
///
/// An HTML band replaces the margin on its side with its own measured height
/// unless that margin was written, in which case the band is fitted into the
/// margin asked for. That is wkhtmltopdf's rule, and it can only be followed
/// if "written" and "left at ten millimetres" are told apart, which the
/// lengths in [`Margins`] cannot do on their own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NamedMargins {
    pub top: bool,
    pub bottom: bool,
}

/// Paper, orientation and margins.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSetup {
    /// The portrait dimensions. Orientation is applied on top.
    pub size: PageDimensions,
    pub orientation: Orientation,
    pub margins: Margins,
    /// Whether `--margin-top` and `--margin-bottom` were written, as opposed
    /// to defaulted. Only an HTML band cares.
    pub named: NamedMargins,
}

impl PageSetup {
    /// The paper as it will actually be printed, orientation applied.
    ///
    /// Orientation is applied here rather than baked into `size`, so that
    /// setting a size after an orientation, or the other way round, gives the
    /// same answer.
    pub fn effective_size(&self) -> PageDimensions {
        match self.orientation {
            Orientation::Portrait => self.size,
            Orientation::Landscape => self.size.landscape(),
        }
    }

    pub fn width_inches(&self) -> f64 {
        self.effective_size().width.to_inches()
    }

    pub fn height_inches(&self) -> f64 {
        self.effective_size().height.to_inches()
    }

    /// Printable width, in inches: the paper less the left and right margins.
    ///
    /// Clamped at zero. Margins wider than the paper are a mistake worth
    /// reporting, but they must not turn into a negative box downstream.
    pub fn content_width_inches(&self) -> f64 {
        (self.width_inches() - self.margins.left.to_inches() - self.margins.right.to_inches())
            .max(0.0)
    }

    /// Printable height, in inches. Clamped at zero, as above.
    pub fn content_height_inches(&self) -> f64 {
        (self.height_inches() - self.margins.top.to_inches() - self.margins.bottom.to_inches())
            .max(0.0)
    }

    /// Whether the margins leave nothing to print on.
    pub fn is_degenerate(&self) -> bool {
        self.content_width_inches() <= 0.0 || self.content_height_inches() <= 0.0
    }
}

impl Default for PageSetup {
    /// A4 portrait with ten-millimetre margins, per D03.
    fn default() -> Self {
        Self {
            size: PageDimensions::mm(210.0, 297.0),
            orientation: Orientation::Portrait,
            margins: Margins::default(),
            named: NamedMargins::default(),
        }
    }
}

/// Which browser to drive, and how.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowserSettings {
    /// An explicit binary, from `--chromium-path`.
    pub path: Option<PathBuf>,
    /// Extra flags, appended after the baseline so they can override it.
    pub extra_args: Vec<String>,
    /// Give up the sandbox. Never a default (D10).
    pub no_sandbox: bool,
}

/// The outline — bookmarks, in a reader's sidebar — and what to do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineSettings {
    /// `--outline` and `--no-outline`. On, as in wkhtmltopdf.
    pub enabled: bool,
    /// `--outline-depth`: headings deeper than this are left out.
    pub depth: u32,
    /// `--dump-outline`: where to write the outline as XML, if anywhere.
    pub dump: Option<PathBuf>,
}

impl Default for OutlineSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            depth: 4,
            dump: None,
        }
    }
}

impl OutlineSettings {
    /// Whether the browser has to produce one at all: to keep, or to dump.
    /// `--no-outline --dump-outline x` still needs it generated.
    pub fn wanted(&self) -> bool {
        self.enabled || self.dump.is_some()
    }
}

/// Settings for the run as a whole.
///
/// Paper, orientation and margins live here rather than per object because
/// wkhtmltopdf treats them as global. Getting that boundary wrong is one of the
/// two things the option table has to be checked against a real binary for.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalSettings {
    pub page: PageSetup,
    pub output: Output,
    /// PDF metadata title. Falls back to the first document's own title.
    pub title: Option<String>,
    /// The whole-conversion deadline. `None` means no limit, which is what
    /// `--timeout 0` asks for.
    pub timeout: Option<Duration>,
    pub log_level: LogLevel,
    pub browser: BrowserSettings,
    pub outline: OutlineSettings,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            page: PageSetup::default(),
            output: Output::Stdout,
            title: None,
            outline: OutlineSettings::default(),
            // wkhtmltopdf has no deadline at all, which leaves a hung browser
            // blocking the caller for ever. Thirty seconds covers real documents
            // and still protects a web worker (D16).
            timeout: Some(Duration::from_secs(30)),
            log_level: LogLevel::default(),
            browser: BrowserSettings::default(),
        }
    }
}

/// Which stylesheets apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaType {
    /// What wkhtmltopdf uses unless told otherwise, and therefore what a
    /// migrated document expects. Chromium's own default is the other one, which
    /// is why this has to be set explicitly on every print (D03).
    #[default]
    Screen,
    Print,
}

/// How long to wait, and what to do when something fails to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadSettings {
    /// Extra wait after the page settles, in milliseconds.
    pub javascript_delay: Duration,
    /// Wait for `window.status` to equal this instead of waiting the delay.
    pub window_status: Option<String>,
    /// Scripts to run once the page has settled, in order.
    pub run_scripts: Vec<String>,
    /// A stylesheet to put into the document, from `--user-style-sheet`.
    ///
    /// Here rather than in [`WebSettings`] because what matters about it is
    /// *when*: it goes in after the document exists and before the wait for web
    /// fonts, which is the same kind of decision as when a script runs.
    pub user_style_sheet: Option<String>,
    pub on_document_error: LoadErrorHandling,
    pub on_media_error: LoadErrorHandling,
    /// Let slow scripts keep running.
    pub stop_slow_scripts: bool,
}

impl Default for LoadSettings {
    fn default() -> Self {
        Self {
            javascript_delay: Duration::from_millis(200),
            window_status: None,
            run_scripts: Vec::new(),
            user_style_sheet: None,
            // A failed subresource still produces a PDF, but exits non-zero and
            // names the error. Applications depend on that pair (D14).
            on_document_error: LoadErrorHandling::Abort,
            on_media_error: LoadErrorHandling::Ignore,
            stop_slow_scripts: true,
        }
    }
}

/// A name and value pair, used for cookies and headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub name: String,
    pub value: String,
}

/// The window wkhtmltopdf emulated, and therefore what a migrated document's
/// media queries were written against (D03).
pub const WKHTMLTOPDF_VIEWPORT: (u32, u32) = (1024, 768);

/// How the page itself should behave.
#[derive(Debug, Clone, PartialEq)]
pub struct WebSettings {
    pub javascript: bool,
    pub images: bool,
    pub background: bool,
    pub media_type: MediaType,
    /// Scale applied at print time, from `--zoom`.
    pub zoom: f64,
    /// What to read the document as when it does not say.
    ///
    /// Applies to a local document only. A document fetched over http or https
    /// is read as its server said, and nothing on this side overrides that.
    pub encoding: Option<String>,
    pub minimum_font_size: Option<u32>,
    /// Emulated window size.
    ///
    /// Affects media queries and what scripts read from the window, and **not**
    /// the printed layout width: Chromium lays a printed page out at the content
    /// width, which for A4 less 10mm margins is about 718 CSS pixels. A design
    /// built for 1024 reflows. That is the second biggest surprise in a
    /// migration after smart shrinking, and `docs/migration.md` carries it.
    pub viewport: (u32, u32),
    pub username: Option<String>,
    pub password: Option<String>,
    pub proxy: Option<String>,
    pub cookies: Vec<Pair>,
    pub custom_headers: Vec<Pair>,
    /// Send the custom headers with subresource requests too, not only the
    /// first one. wkhtmltopdf defaults this off, and the asymmetry surprises
    /// people.
    pub propagate_custom_headers: bool,
    /// Let a local document read other local files.
    ///
    /// Off by default. User-supplied HTML plus filesystem reach is the exact
    /// vulnerability that made wkhtmltopdf change this default in 0.12.6 (D10).
    pub local_file_access: bool,
    /// Directories a local document may read from even without the above.
    pub allowed_paths: Vec<PathBuf>,
}

impl Default for WebSettings {
    fn default() -> Self {
        Self {
            javascript: true,
            images: true,
            background: true,
            media_type: MediaType::default(),
            zoom: 1.0,
            encoding: None,
            minimum_font_size: None,
            viewport: WKHTMLTOPDF_VIEWPORT,
            username: None,
            password: None,
            proxy: None,
            cookies: Vec::new(),
            custom_headers: Vec::new(),
            propagate_custom_headers: false,
            local_file_access: false,
            allowed_paths: Vec::new(),
        }
    }
}

/// What becomes of the links in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSettings {
    /// Links to other places stay clickable. Off is `--disable-external-links`.
    pub external: bool,
    /// Links to an anchor stay clickable — in this document, or in another
    /// document of the same conversion. Off is `--disable-internal-links`.
    pub internal: bool,
    /// A link written relative in the document is kept as the browser resolved
    /// it, absolute. Off is `--keep-relative-links`.
    pub resolve_relative: bool,
}

impl Default for LinkSettings {
    fn default() -> Self {
        Self {
            external: true,
            internal: true,
            resolve_relative: true,
        }
    }
}

impl LinkSettings {
    /// Whether every link is left as the browser wrote it.
    pub fn leaves_everything(&self) -> bool {
        *self == Self::default()
    }
}

/// A header or a footer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Band {
    pub left: Option<String>,
    pub center: Option<String>,
    pub right: Option<String>,
    pub font_name: Option<String>,
    pub font_size: Option<f64>,
    /// Gap between the band and the content, in millimetres.
    pub spacing: Option<f64>,
    /// Draw a rule between the band and the content.
    pub line: bool,
    /// A document to use as the band instead of the text fields.
    pub html: Option<String>,
}

impl Band {
    /// Whether anything would actually be drawn.
    pub fn is_empty(&self) -> bool {
        self.left.is_none()
            && self.center.is_none()
            && self.right.is_none()
            && self.html.is_none()
            && !self.line
    }
}

/// `TOC Options`: what a generated table of contents looks like.
///
/// These are CSS rather than paper measurements. wkhtmltopdf substituted them
/// into the stylesheet it transformed the outline with, where the indentation
/// is a `padding-left` and its default is `1em` — which is why it is kept as
/// the string it was written as. [`Length`] normalises
/// to inches and has no `em`, so it could not carry the default, let alone
/// echo back what the user typed.
#[derive(Debug, Clone, PartialEq)]
pub struct TocSettings {
    /// `--toc-header-text`: the heading printed above the entries.
    pub header_text: String,
    /// `--toc-level-indentation`: how much further each level is indented.
    pub level_indentation: String,
    /// `--toc-text-size-shrink`: the factor the font is scaled by per level.
    pub text_size_shrink: f64,
    /// Whether a dotted line joins an entry to its page number. Off under
    /// `--disable-dotted-lines`.
    pub dotted_lines: bool,
}

impl Default for TocSettings {
    /// wkhtmltopdf's defaults, as its help states them.
    fn default() -> Self {
        Self {
            header_text: "Table of Contents".to_string(),
            level_indentation: "1em".to_string(),
            text_size_shrink: 0.8,
            dotted_lines: true,
        }
    }
}

/// What kind of object this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectKind {
    #[default]
    Page,
    /// A page excluded from numbering and from the outline.
    Cover,
    /// A generated table of contents, which has no input document.
    Toc,
}

/// One document in the output.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectSettings {
    pub kind: ObjectKind,
    /// Absent only for a table of contents, which is generated rather than read.
    pub input: Option<Input>,
    pub load: LoadSettings,
    pub web: WebSettings,
    pub header: Band,
    pub footer: Band,
    /// Replacements applied to header and footer text before placeholders are
    /// expanded.
    pub replacements: Vec<Pair>,
    /// `--include-in-outline` and `--exclude-from-outline`: whether this
    /// document's headings go into the outline and the table of contents.
    /// Off for a cover.
    pub in_outline: bool,
    pub links: LinkSettings,
    /// `--page-offset`: added to `[page]`, `[topage]` and `[frompage]` on this
    /// document's pages. Nought, as in wkhtmltopdf.
    pub page_offset: i64,
    /// How this object looks, when it is a table of contents. Left at its
    /// defaults on a page or a cover, which no `TOC Option` can be written on.
    pub toc: TocSettings,
}

impl ObjectSettings {
    /// A page reading the given input, everything else left at wkhtmltopdf's
    /// defaults.
    pub fn page(input: Input) -> Self {
        Self {
            kind: ObjectKind::Page,
            input: Some(input),
            load: LoadSettings::default(),
            web: WebSettings::default(),
            header: Band::default(),
            footer: Band::default(),
            replacements: Vec::new(),
            in_outline: true,
            links: LinkSettings::default(),
            page_offset: 0,
            toc: TocSettings::default(),
        }
    }
}

/// Everything one run of the program has been asked to do.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub global: GlobalSettings,
    pub objects: Vec<ObjectSettings>,
}

impl Settings {
    /// The single object, when there is exactly one.
    ///
    /// For a test that wrote one document and wants its settings without
    /// indexing. A conversion walks `objects` itself.
    pub fn single_object(&self) -> Option<&ObjectSettings> {
        match self.objects.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "{actual} is not approximately {expected}"
        );
    }

    /// D03, item by item. Every later assertion in the project is calibrated
    /// against these, so a change here is a change to what a migrated document
    /// looks like, and should be deliberate rather than incidental.
    #[test]
    fn the_defaults_are_wkhtmltopdfs_not_chromiums() {
        let settings = Settings::default();
        let page = settings.global.page;

        // A4 portrait, not Letter.
        approx(page.size.width.to_mm(), 210.0);
        approx(page.size.height.to_mm(), 297.0);
        assert_eq!(page.orientation, Orientation::Portrait);

        // Ten millimetres all round, not one centimetre.
        for margin in [
            page.margins.top,
            page.margins.right,
            page.margins.bottom,
            page.margins.left,
        ] {
            approx(margin.to_mm(), 10.0);
        }

        // Thirty seconds, where wkhtmltopdf had no limit at all (D16).
        assert_eq!(settings.global.timeout, Some(Duration::from_secs(30)));
        assert_eq!(settings.global.log_level, LogLevel::Info);
        assert!(!settings.global.browser.no_sandbox);

        let object = ObjectSettings::page(Input::classify("a.html"));

        // Screen stylesheets, which is the single easiest thing to get wrong:
        // Chromium prints with the print ones unless told otherwise.
        assert_eq!(object.web.media_type, MediaType::Screen);
        assert!(object.web.background);
        assert!(object.web.javascript);
        assert!(object.web.images);
        approx(object.web.zoom, 1.0);
        // The window wkhtmltopdf emulated. A migrated document's media queries
        // were written against this one, not against the printed width.
        assert_eq!(object.web.viewport, (1024, 768));
        assert!(object.web.encoding.is_none());
        assert_eq!(object.web.minimum_font_size, None);

        // Off, because user-supplied HTML plus filesystem reach is the bug that
        // made wkhtmltopdf change this in 0.12.6 (D10).
        assert!(!object.web.local_file_access);
        assert!(object.web.allowed_paths.is_empty());
        assert!(!object.web.propagate_custom_headers);

        assert_eq!(object.load.javascript_delay, Duration::from_millis(200));
        assert_eq!(object.load.on_document_error, LoadErrorHandling::Abort);
        assert_eq!(object.load.on_media_error, LoadErrorHandling::Ignore);
        assert!(object.load.stop_slow_scripts);
        assert!(object.load.run_scripts.is_empty());
        assert!(object.load.user_style_sheet.is_none());

        assert!(object.header.is_empty());
        assert!(object.footer.is_empty());

        // An outline, four levels deep, with every page in it.
        assert!(settings.global.outline.enabled);
        assert_eq!(settings.global.outline.depth, 4);
        assert!(settings.global.outline.dump.is_none());
        assert!(object.in_outline);

        // Every link clickable, written as the browser resolved it.
        assert!(object.links.external);
        assert!(object.links.internal);
        assert!(object.links.resolve_relative);
        assert!(object.links.leaves_everything());
        assert_eq!(object.page_offset, 0);
    }

    /// `--no-outline --dump-outline x` still needs the browser to produce one.
    #[test]
    fn an_outline_is_wanted_to_keep_or_to_dump() {
        let mut outline = OutlineSettings::default();
        assert!(outline.wanted());
        outline.enabled = false;
        assert!(!outline.wanted());
        outline.dump = Some(PathBuf::from("o.xml"));
        assert!(outline.wanted());
    }

    #[test]
    fn a4_measures_what_the_print_call_expects() {
        let page = PageSetup::default();
        // 210mm and 297mm in inches.
        approx(page.width_inches(), 210.0 / 25.4);
        approx(page.height_inches(), 297.0 / 25.4);
        // Less ten millimetres on each side.
        approx(page.content_width_inches(), 190.0 / 25.4);
        approx(page.content_height_inches(), 277.0 / 25.4);
    }

    /// Orientation is applied when the size is read, not folded into it, so the
    /// order the two options were written in cannot change the answer.
    // The sequential assignment below is deliberate and is what the test is
    // about: the command line applies options one at a time, in whatever order
    // they were written, so that is how they are applied here. A struct
    // initializer would prove nothing.
    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn landscape_swaps_the_axes_whichever_order_it_was_set_in() {
        let mut first = PageSetup::default();
        first.orientation = Orientation::Landscape;
        first.size = PageDimensions::mm(210.0, 297.0);

        let mut second = PageSetup::default();
        second.size = PageDimensions::mm(210.0, 297.0);
        second.orientation = Orientation::Landscape;

        assert_eq!(first.effective_size(), second.effective_size());
        approx(first.effective_size().width.to_mm(), 297.0);
        approx(first.effective_size().height.to_mm(), 210.0);
    }

    /// Margins wider than the paper are a mistake worth reporting, but they must
    /// never reach the print call as a negative box.
    #[test]
    fn absurd_margins_clamp_rather_than_going_negative() {
        let page = PageSetup {
            margins: Margins::uniform(Length::mm(500.0)),
            ..PageSetup::default()
        };
        approx(page.content_width_inches(), 0.0);
        approx(page.content_height_inches(), 0.0);
        assert!(page.is_degenerate());
        assert!(!PageSetup::default().is_degenerate());
    }

    #[test]
    fn the_unit_the_user_typed_survives() {
        // So an error can echo `15mm` rather than a converted decimal.
        let page = PageSetup {
            margins: Margins {
                top: Length::parse("0.5in").unwrap(),
                ..Margins::default()
            },
            ..PageSetup::default()
        };
        assert_eq!(page.margins.top.to_string(), "0.5in");
        approx(page.margins.top.to_mm(), 12.7);
    }

    #[test]
    fn log_levels_parse_and_decide_whether_warnings_show() {
        assert_eq!(LogLevel::parse("none"), Some(LogLevel::None));
        assert_eq!(LogLevel::parse("INFO"), Some(LogLevel::Info));
        assert_eq!(LogLevel::parse("shout"), None);
        assert!(!LogLevel::None.shows_warnings());
        assert!(!LogLevel::Error.shows_warnings());
        assert!(LogLevel::Warn.shows_warnings());
        assert!(LogLevel::Info.shows_warnings());

        // Progress is chatter, so it stops one level earlier than warnings do.
        assert!(LogLevel::Info.shows_progress());
        assert!(!LogLevel::Warn.shows_progress());
        assert!(!LogLevel::None.shows_progress());
    }

    #[test]
    fn a_band_with_only_a_rule_is_not_empty() {
        let mut footer = Band::default();
        assert!(footer.is_empty());
        footer.line = true;
        assert!(!footer.is_empty());
    }

    #[test]
    fn one_object_is_reachable_and_several_are_not_silently_truncated() {
        let mut settings = Settings::default();
        assert!(settings.single_object().is_none());

        settings.objects.push(ObjectSettings::page(Input::Stdin));
        assert!(settings.single_object().is_some());

        settings
            .objects
            .push(ObjectSettings::page(Input::classify("b.html")));
        // Two objects must not look like one.
        assert!(settings.single_object().is_none());
    }
}
