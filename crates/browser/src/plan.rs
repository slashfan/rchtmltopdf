//! What a conversion will ask the browser to do, worked out before it asks.
//!
//! # Why this exists
//!
//! `Support::Implemented` in the option table is a claim that an option changes
//! what comes out of the program. For most of the surface that claim was false
//! in a way nothing could catch: the command line filled a settings field, no
//! part of the conversion ever read the field, and the help said the option
//! worked. Sixty per cent of the options marked as working did nothing at all
//! (#19).
//!
//! A claim like that is only checkable if what a conversion *would* do can be
//! produced without doing it. That is this module. A test drives one option at a
//! time and asserts the plan changes; an option that changes nothing here, and
//! nothing before a browser is started, is not implemented whatever the table
//! says (D27).
//!
//! # The contract that makes it a guard
//!
//! **Nothing above this may decide anything on its own.** [`Page::prepare`]
//! sends [`Plan::prepare`] and nothing else, [`Page::print_to_pdf`] sends
//! [`Plan::print`] and nothing else, and [`Page::load`] consults [`LoadPlan`]
//! and nothing else. The moment one of them reads a settings field directly, the
//! plan stops describing the conversion and the guard above it becomes a second
//! opinion rather than a check.
//!
//! Two things a plan cannot finish deciding. The local file policy, because
//! what it permits depends on the document it is bound to and the document is
//! resolved after the settings are read: the policy itself is here, binding it
//! is [`crate::file_access::Policy::about`]. And the height of a band that is
//! a document, which sizes a margin (D39) and is not known until the document
//! is loaded: [`print()`] decides everything else about the margins, and
//! [`reserve`] adds the measurement and nothing more.
//!
//! [`Page::prepare`]: crate::launch::Page::prepare
//! [`Page::print_to_pdf`]: crate::launch::Page::print_to_pdf
//! [`Page::load`]: crate::launch::Page::load

use crate::file_access::{self, Policy};
use crate::intercept::{Charset, Credentials, Rules};
use crate::launch::LaunchOptions;
use crate::placeholder;
use crate::placeholder::Context;
use rchtmltopdf_core::Clock;
use rchtmltopdf_core::settings::{
    Band, GlobalSettings, LinkSettings, LoadSettings, MediaType, ObjectKind, ObjectSettings,
    OutlineSettings, PageSetup, TocSettings, WebSettings,
};
use rchtmltopdf_core::units::Length;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

/// One protocol call, decided but not yet sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub method: &'static str,
    pub params: Value,
}

impl Command {
    pub fn new(method: &'static str, params: Value) -> Self {
        Self { method, params }
    }
}

/// How to tell that the page has finished.
///
/// Only the last rung of D07's ladder is a choice. The three below it — the load
/// event, the network going quiet, web fonts — are unconditional, so they are
/// not represented here: there is nothing to decide about them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settle {
    /// Wait this long after the page has gone quiet.
    Delay(Duration),
    /// Wait for `window.status` to read this instead of waiting a fixed time.
    WindowStatus(String),
}

/// A user stylesheet, and how it gets into the document.
///
/// `--user-style-sheet` takes either a path or a URL, and they are not the same
/// thing to do. A path is read by us and inlined, which is deliberately not the
/// same as letting the document fetch it: the user named this file on the
/// command line, so the policy governing what the *document* may reach off the
/// disk has nothing to say about it (D10). A URL is left to the browser, and is
/// fetched before the network is judged idle because it is put in first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Injection {
    File(PathBuf),
    Link(String),
}

/// What the wait ladder has been asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadPlan {
    /// Put in place once the document exists and **before the wait for web
    /// fonts**. A user stylesheet that declares a font face arriving after that
    /// wait makes the wait meaningless: it resolves against the fonts the page
    /// had, and the one being introduced is still being fetched when the page is
    /// printed.
    pub inject: Option<Injection>,
    pub settle: Settle,
    /// Run in order once the page has settled, each awaited.
    pub scripts: Vec<String>,
}

impl LoadPlan {
    pub fn new(load: &LoadSettings) -> Self {
        Self {
            inject: load.user_style_sheet.as_deref().map(injection),
            // Naming a status replaces the delay rather than adding to it, which
            // is what wkhtmltopdf does: the delay is the fallback for having no
            // signal, and a page that publishes one does not need it.
            settle: match &load.window_status {
                Some(wanted) => Settle::WindowStatus(wanted.clone()),
                None => Settle::Delay(load.javascript_delay),
            },
            scripts: load.run_scripts.clone(),
        }
    }
}

/// Tell a path from a URL the way every other option here tells them apart, so
/// `C:\styles.css` cannot be a path to one option and a URL to another.
fn injection(written: &str) -> Injection {
    if rchtmltopdf_core::has_url_scheme(written) {
        Injection::Link(written.to_string())
    } else {
        Injection::File(PathBuf::from(written))
    }
}

/// Everything one conversion will do, in the order it will do it.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The browser to start, and how.
    pub launch: LaunchOptions,
    /// Sent before the document is opened, because several of these decide what
    /// is fetched at all and are worth nothing afterwards.
    pub prepare: Vec<Command>,
    pub load: LoadPlan,
    pub print: Command,
    /// What the bands are expanded against. Held so the conversion and a test
    /// can be given the same clock: a plan built twice a second apart must not
    /// differ, or every option would look as though it changed something.
    pub context: Context,
    /// The bound over the whole of the above (D16). `None` is `--timeout 0`.
    pub deadline: Option<Duration>,
    /// What is done to the printed document once the browser is finished with
    /// it: the outline is bounded, dumped, or dropped after printing, because
    /// the print call can only be asked for all of it or none.
    pub finish: Finishing,
    /// What the interception handler will do with each request: the local file
    /// rule (D10), `--encoding`, the document-only headers and the credentials.
    ///
    /// Bound to the document, which is why [`Plan::new`] takes one. Everything
    /// here depends on knowing which request is the document's own, and on
    /// whether the document is itself local.
    pub requests: Rules,
    /// The table of contents this object *is*, if it is one (D41). `None` for
    /// a page and a cover, which is what makes writing a `TOC Option` on one
    /// change nothing — there is nothing for it to change.
    pub toc: Option<TocSettings>,
}

impl Plan {
    /// The clock is a parameter rather than read here, because a plan is
    /// compared in tests and two built a second apart must be equal.
    pub fn new(
        global: &GlobalSettings,
        object: &ObjectSettings,
        clock: Clock,
        document_url: &str,
    ) -> Self {
        let context = context(global, object, clock);
        Self {
            launch: launch(global, object),
            prepare: prepare(object, document_url),
            load: LoadPlan::new(&object.load),
            print: print(&global.page, object, &global.outline),
            context,
            deadline: global.timeout,
            requests: rules(object, document_url),
            finish: finish(global, object, document_url),
            toc: toc(object),
        }
    }
}

/// How this object's table of contents looks, when it is one.
///
/// The generation itself is the `cli` crate's — it is markup, and nothing here
/// prints it differently — but the settings pass through the plan like every
/// other decision, so that writing `--toc-header-text` moves the plan and the
/// option table can be held to it (D27).
pub fn toc(object: &ObjectSettings) -> Option<TocSettings> {
    (object.kind == ObjectKind::Toc).then(|| object.toc.clone())
}

/// What happens to the printed document after the browser is done with it.
#[derive(Debug, Clone, PartialEq)]
pub struct Finishing {
    pub outline: OutlineSettings,
    /// The bands to draw on this document's pages, once the counts are known
    /// (D38). Expanded against [`Plan::context`] and the page's numbers.
    pub bands: Bands,
    /// How this document's pages count, for the bands and for the outline
    /// dump (D40).
    pub numbering: Numbering,
    /// This document's links: which kinds stay, and whether a relative one is
    /// written back relative.
    pub links: LinkSettings,
    /// What the links were resolved against, so a relative one can be
    /// recognised afterwards and a link to another document of the same
    /// conversion can be pointed into it.
    pub document_url: String,
}

/// One document's bands: what is drawn on every page of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Bands {
    pub header: Band,
    pub footer: Band,
}

/// How one document's pages count.
///
/// Two things read this and they do not agree, which is what D40 is about: a
/// band prints `[page]`, which a cover is left out of, while the outline dump
/// numbers a page of the file, which a cover is one of. Both add
/// `page_offset`.
#[derive(Debug, Clone, PartialEq)]
pub struct Numbering {
    /// Whether the pages count towards `[page]` and `[topage]`: a cover's do
    /// not.
    pub counted: bool,
    /// `--page-offset`.
    pub page_offset: i64,
}

impl Bands {
    /// Whether anything at all would be drawn.
    pub fn is_empty(&self) -> bool {
        self.header.is_empty() && self.footer.is_empty()
    }

    /// Whether a band names a heading, and so needs the outline generated even
    /// when nobody asked to keep it. A band that is a document is handed every
    /// placeholder, the sections included, so it always does.
    pub fn names_a_section(&self) -> bool {
        [&self.header, &self.footer].iter().any(|band| {
            band.html.is_some()
                || [&band.left, &band.center, &band.right]
                    .iter()
                    .any(|cell| cell.as_deref().is_some_and(placeholder::names_a_section))
        })
    }

    /// Whether either band is a document, which has to be measured before
    /// the pages are printed (D39).
    pub fn any_document(&self) -> bool {
        self.header.html.is_some() || self.footer.html.is_some()
    }
}

/// Whether the outline has to be generated for this document: to keep, to
/// dump, or to name sections in a band.
pub fn outline_wanted(outline: &OutlineSettings, object: &ObjectSettings) -> bool {
    let bands = Bands {
        header: object.header.clone(),
        footer: object.footer.clone(),
    };
    (outline.wanted() || bands.names_a_section()) && object.in_outline
}

/// The post-print treatment the command line asked for.
pub fn finish(global: &GlobalSettings, object: &ObjectSettings, document_url: &str) -> Finishing {
    Finishing {
        outline: global.outline.clone(),
        bands: Bands {
            header: object.header.clone(),
            footer: object.footer.clone(),
        },
        numbering: Numbering {
            counted: object.kind != ObjectKind::Cover,
            page_offset: object.page_offset,
        },
        links: object.links.clone(),
        document_url: document_url.to_string(),
    }
}

/// What the bands are expanded against.
///
/// Everything here comes from the command line rather than from the document.
/// `[title]` is `--title` and not the page's own `<title>`, which needs the
/// document to have been loaded and is #29's business; `[webpage]` is the input
/// as it was written, which is what wkhtmltopdf prints too.
pub fn context(global: &GlobalSettings, object: &ObjectSettings, clock: Clock) -> Context {
    Context {
        webpage: object
            .input
            .as_ref()
            .map(|input| input.as_written())
            .unwrap_or_default(),
        title: global.title.clone().unwrap_or_default(),
        replacements: object.replacements.clone(),
        clock,
    }
}

/// Which local files the document may read.
pub fn file_access(web: &WebSettings) -> Policy {
    Policy {
        enabled: web.local_file_access,
        allowed: web.allowed_paths.clone(),
    }
}

/// What the interception handler is being asked to do.
pub fn rules(object: &ObjectSettings, document_url: &str) -> Rules {
    let web = &object.web;

    // `--encoding` can only be applied to a document we can read ourselves. A
    // document fetched over http or https is read as its server said, and there
    // is nowhere to intervene.
    let serve_as = web.encoding.as_ref().and_then(|name| {
        file_access::local_path(document_url).map(|path| Charset {
            url: document_url.to_string(),
            path,
            name: name.clone(),
        })
    });

    Rules {
        files: file_access(web).about(document_url),
        document: document_url.to_string(),
        serve_as,
        // With propagation the headers go on every request instead, through
        // `Network.setExtraHTTPHeaders` in `prepare`, and there is nothing left
        // for the handler to add. Without it, only the document's own request
        // carries them -- which is wkhtmltopdf's asymmetry, not ours.
        document_headers: match web.propagate_custom_headers {
            true => Vec::new(),
            false => web.custom_headers.clone(),
        },
        credentials: (web.username.is_some() || web.password.is_some()).then(|| Credentials {
            username: web.username.clone().unwrap_or_default(),
            password: web.password.clone().unwrap_or_default(),
        }),
    }
}

/// Whether a document is one a cookie can be set for.
///
/// A cookie on a `file://` document is meaningless -- there is no origin to
/// scope it to -- so it is dropped and the binary says so rather than failing.
pub fn takes_cookies(document_url: &str) -> bool {
    document_url.starts_with("http://") || document_url.starts_with("https://")
}

/// The browser to start.
pub fn launch(global: &GlobalSettings, object: &ObjectSettings) -> LaunchOptions {
    LaunchOptions {
        no_sandbox: global.browser.no_sandbox,
        extra_args: global.browser.extra_args.clone(),
        allow_slow_scripts: !object.load.stop_slow_scripts,
        proxy: object.web.proxy.clone(),
        // Blink settings rather than protocol commands, so they have to be
        // decided before the browser starts rather than before the page loads.
        minimum_font_size: object.web.minimum_font_size,
        no_images: !object.web.images,
        ..LaunchOptions::default()
    }
}

/// What to put in place before the document arrives.
pub fn prepare(object: &ObjectSettings, document_url: &str) -> Vec<Command> {
    let web = &object.web;
    // The two domains have to be enabled before anything is expected from them:
    // the load event for a small document can fire before the navigate call has
    // returned.
    let mut commands = vec![
        Command::new("Page.enable", Value::Null),
        Command::new("Network.enable", Value::Null),
        emulate_media(web),
        viewport(web),
    ];

    // Before the document is asked for, so the first request carries them.
    if !web.cookies.is_empty() && takes_cookies(document_url) {
        commands.push(Command::new(
            "Network.setCookies",
            json!({
                "cookies": web
                    .cookies
                    .iter()
                    .map(|cookie| json!({
                        "name": cookie.name,
                        // wkhtmltopdf's own help says the value arrives url
                        // encoded, so it is decoded before the browser sees it.
                        "value": file_access::percent_decode(&cookie.value),
                        // Scoped to the document's own origin, which is what
                        // wkhtmltopdf does and the only sane default.
                        "url": document_url,
                    }))
                    .collect::<Vec<_>>(),
            }),
        ));
    }

    // Every request, not just the document's: that is what propagation means,
    // and without it the handler puts them on the document alone.
    if web.propagate_custom_headers && !web.custom_headers.is_empty() {
        let headers: serde_json::Map<String, Value> = web
            .custom_headers
            .iter()
            .map(|header| (header.name.clone(), Value::String(header.value.clone())))
            .collect();
        commands.push(Command::new(
            "Network.setExtraHTTPHeaders",
            json!({ "headers": headers }),
        ));
    }

    // Only sent when it is being turned off. Chromium runs scripts unless told
    // not to, so the absence of this command is the default, and sending it with
    // `false` would be a command that means nothing.
    if !web.javascript {
        commands.push(Command::new(
            "Emulation.setScriptExecutionDisabled",
            json!({ "value": true }),
        ));
    }

    commands
}

/// Choose which stylesheets apply.
///
/// **This is the easiest thing in the whole project to get wrong.** Chromium
/// prints with `print` stylesheets unless told otherwise; wkhtmltopdf renders
/// with `screen` ones. A migrated document with `@media print` rules it never
/// used before silently changes layout if this is skipped, and so does every
/// fixture calibrated against it (D03).
pub fn emulate_media(web: &WebSettings) -> Command {
    let media = match web.media_type {
        MediaType::Screen => "screen",
        MediaType::Print => "print",
    };
    Command::new("Emulation.setEmulatedMedia", json!({ "media": media }))
}

/// Emulate the window wkhtmltopdf had.
///
/// Sent on every conversion and not only when `--viewport-size` was written,
/// because Chromium's own window is not wkhtmltopdf's 1024 by 768 and a migrated
/// document's media queries were written against that one (D03).
///
/// It does **not** decide the printed layout width. Chromium lays a printed page
/// out at the content width — about 718 CSS pixels for A4 less 10mm margins — so
/// a design built for 1024 reflows however this is set. `docs/migration.md`
/// carries that, because it is the second biggest surprise in a migration.
pub fn viewport(web: &WebSettings) -> Command {
    let (width, height) = web.viewport;
    Command::new(
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width": width,
            "height": height,
            // wkhtmltopdf rendered at one device pixel per CSS pixel, and a
            // desktop browser, not a phone.
            "deviceScaleFactor": 1,
            "mobile": false,
        }),
    )
}

/// The print call for one document.
///
/// Orientation is resolved here rather than passed on, so the paper handed over
/// is already the right way round. Letting the protocol rotate it as well would
/// apply the swap twice. Paper geometry is global in wkhtmltopdf; backgrounds
/// and zoom belong to the object. Both are needed, and they come from different
/// places.
///
/// No bands: since D38 they are printed afterwards as a document of their own
/// and stamped onto the pages, so the browser is told to draw none and the
/// margins are all that is decided here.
pub fn print(page: &PageSetup, object: &ObjectSettings, outline: &OutlineSettings) -> Command {
    let web = &object.web;

    // A band is anchored to the paper edge and reaches towards the content, so
    // it cannot open a gap below itself: the print margin is the only thing
    // that decides where the content starts. `--header-spacing` therefore lands
    // here rather than in the band, and only for a band that draws something,
    // because spacing under nothing is nothing. Measured, not assumed --
    // `crates/browser/src/band.rs` carries the evidence.
    let gap = |band: &Band| match band.is_empty() {
        true => 0.0,
        false => Length::mm(band.spacing.unwrap_or(0.0)).to_inches(),
    };

    // A band that is a document follows wkhtmltopdf's other rule (D39): its
    // own height is the margin unless the margin was written. The height is
    // not known here -- the document has to be loaded to measure it -- so
    // what is decided is the rest, and `reserve` adds the measurement.
    let margin = |band: &Band, named: bool, written: Length| match band.html.is_some() && !named {
        true => 0.0,
        false => written.to_inches(),
    };

    Command::new(
        "Page.printToPDF",
        json!({
            "paperWidth": page.width_inches(),
            "paperHeight": page.height_inches(),
            "marginTop": margin(&object.header, page.named.top, page.margins.top)
                + gap(&object.header),
            "marginBottom": margin(&object.footer, page.named.bottom, page.margins.bottom)
                + gap(&object.footer),
            "marginLeft": page.margins.left.to_inches(),
            "marginRight": page.margins.right.to_inches(),
            "printBackground": web.background,
            "scale": web.zoom,
            // Orientation is already in the paper dimensions above.
            "landscape": false,
            // Leave this off. Turning it on lets a document's own @page rule
            // override --page-size, and wkhtmltopdf does not do that, so neither
            // do we.
            "preferCSSPageSize": false,
            // Nothing: the bands are drawn afterwards (D38). Chromium would
            // otherwise draw its own footer, a page number nobody asked for.
            "displayHeaderFooter": false,
            // Chromium derives an outline from the headings, nested by level,
            // with a destination on each. All or nothing per document: the
            // depth is cut afterwards (`Finishing`), and a document kept out
            // of the outline is one that was never asked for it.
            "generateDocumentOutline": outline_wanted(outline, object),
            // Not the default. The default returns the whole document
            // base64-encoded inside one protocol message, and a document of any
            // size exceeds the message limit.
            "transferMode": "ReturnAsStream",
        }),
    )
}

/// Whether a band's document decides the margin on its side.
///
/// wkhtmltopdf's rule: an HTML band replaces a margin that was defaulted and
/// is fitted into one that was written. Only the first kind is added to the
/// print margin by [`reserve`]; both kinds are measured, because the sheet
/// needs the frame's height either way.
pub fn sized_by_its_document(band: &Band, named: bool) -> bool {
    band.html.is_some() && !named
}

/// The print call with the measured band documents added to the margins.
///
/// The one thing [`print()`] cannot decide on its own: how tall a document is.
/// Everything else about the margins is already in the command, and this adds
/// the two measurements and nothing more, so the plan still describes the
/// conversion (D27). An amount is only added for a band whose document sizes
/// its margin; pass zero for the rest.
pub fn reserve(print: &Command, header_inches: f64, footer_inches: f64) -> Command {
    let mut params = print.params.clone();
    for (key, add) in [
        ("marginTop", header_inches),
        ("marginBottom", footer_inches),
    ] {
        let current = params.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        params[key] = json!(current + add);
    }
    Command::new(print.method, params)
}

/// What to put in place before a band document is opened to be measured.
///
/// Laid out at the content width, in CSS pixels, which is the width the
/// frame on the sheet will give it, with the stylesheets the pages get. The
/// height is the paper's: tall enough that nothing scrolls, so the body is
/// measured as it will be framed.
pub fn measure_prepare(page: &PageSetup, web: &WebSettings) -> Vec<Command> {
    vec![
        Command::new("Page.enable", Value::Null),
        Command::new("Network.enable", Value::Null),
        emulate_media(web),
        Command::new(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": (page.content_width_inches() * CSS_PIXELS_PER_INCH).round() as u64,
                "height": (page.height_inches() * CSS_PIXELS_PER_INCH).round() as u64,
                "deviceScaleFactor": 1,
                "mobile": false,
            }),
        ),
    ]
}

/// Chromium lays a printed page out at this many CSS pixels to the inch, so a
/// document measured at this density measures as it will print.
pub const CSS_PIXELS_PER_INCH: f64 = 96.0;

/// What to put in place before the document of band sheets is opened.
///
/// Only the two domains the wait needs. The sheets are ours: no cookies, no
/// headers, and the window is irrelevant to absolutely positioned boxes. When
/// a sheet frames a band document, the stylesheets that document gets are the
/// ones the pages got — `web` is the settings of the object it was written on.
pub fn band_prepare(web: Option<&WebSettings>) -> Vec<Command> {
    let mut commands = vec![
        Command::new("Page.enable", Value::Null),
        Command::new("Network.enable", Value::Null),
    ];
    commands.extend(web.map(emulate_media));
    commands
}

/// How to wait for the document of band sheets: the load event, the network
/// and the fonts, then `delay`.
///
/// Zero for sheets of text, which have no script to give time to. A band
/// document is loaded with its object's `--javascript-delay`, as wkhtmltopdf
/// loaded it; with several objects the longest is used, because the sheets
/// are one document.
pub fn band_load(delay: Duration) -> LoadSettings {
    LoadSettings {
        javascript_delay: delay,
        ..LoadSettings::default()
    }
}

/// The local file rule for the sheets and the band documents they frame.
///
/// One page carries every object's bands, so it gets the union of their
/// policies: on if any object turned it on, and every directory any of them
/// allowed. The documents named on the command line are readable regardless
/// (`also`), as the input is.
pub fn band_policy<'a>(objects: impl IntoIterator<Item = &'a ObjectSettings>) -> Policy {
    let mut policy = Policy::default();
    for object in objects {
        policy.enabled |= object.web.local_file_access;
        policy
            .allowed
            .extend(object.web.allowed_paths.iter().cloned());
    }
    policy
}

/// What the interception handler does on a page that loads band documents:
/// the file rule and nothing else. No charset, no document headers, no
/// credentials — those are the input's options, and `document` here is the
/// sheet or the document being measured.
pub fn band_rules<'a>(
    policy: &Policy,
    document: &str,
    also: impl IntoIterator<Item = &'a str>,
) -> Rules {
    let mut files = policy.about(document);
    for url in also {
        files = files.also(url);
    }
    Rules {
        files,
        document: document.to_string(),
        ..Rules::default()
    }
}

/// The print call for the document of band sheets: the same paper, no
/// margins, backgrounds on so a rule is drawn, and nothing of the browser's
/// own.
pub fn band_print(page: &PageSetup) -> Command {
    Command::new(
        "Page.printToPDF",
        json!({
            "paperWidth": page.width_inches(),
            "paperHeight": page.height_inches(),
            "marginTop": 0.0,
            "marginBottom": 0.0,
            "marginLeft": 0.0,
            "marginRight": 0.0,
            "printBackground": true,
            "scale": 1.0,
            "landscape": false,
            "preferCSSPageSize": false,
            "displayHeaderFooter": false,
            "generateDocumentOutline": false,
            "transferMode": "ReturnAsStream",
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchtmltopdf_core::Input;
    use rchtmltopdf_core::settings::Pair;

    fn page_object() -> ObjectSettings {
        ObjectSettings::page(Input::Stdin)
    }

    /// The prepare commands for a page with these web settings, against a real
    /// origin: cookies are dropped for anything else.
    fn prepared(web: WebSettings) -> Vec<Command> {
        let object = ObjectSettings {
            web,
            ..page_object()
        };
        prepare(&object, "https://example.com/doc")
    }

    fn methods(commands: &[Command]) -> Vec<&str> {
        commands.iter().map(|command| command.method).collect()
    }

    #[test]
    fn the_two_domains_are_enabled_before_anything_is_expected_of_them() {
        let commands = prepared(WebSettings::default());
        let methods: Vec<&str> = commands.iter().map(|command| command.method).collect();
        assert_eq!(
            methods,
            [
                "Page.enable",
                "Network.enable",
                "Emulation.setEmulatedMedia",
                "Emulation.setDeviceMetricsOverride",
            ]
        );
    }

    /// Sent on every conversion, not only when `--viewport-size` was written:
    /// Chromium's own window is not the one a migrated document was written
    /// against (D03).
    #[test]
    fn the_window_is_the_one_wkhtmltopdf_emulated_unless_asked_otherwise() {
        let default = viewport(&WebSettings::default());
        assert_eq!(default.params["width"], json!(1024));
        assert_eq!(default.params["height"], json!(768));

        let asked = viewport(&WebSettings {
            viewport: (1280, 1024),
            ..WebSettings::default()
        });
        assert_eq!(asked.params["width"], json!(1280));
        // One device pixel per CSS pixel, and not a phone.
        assert_eq!(asked.params["deviceScaleFactor"], json!(1));
        assert_eq!(asked.params["mobile"], json!(false));
    }

    /// Both are Blink settings, so they are decided before the browser starts
    /// rather than before the page loads.
    #[test]
    fn the_font_floor_and_images_are_launch_decisions() {
        let global = GlobalSettings::default();
        let mut object = page_object();
        assert_eq!(launch(&global, &object).minimum_font_size, None);
        assert!(!launch(&global, &object).no_images);

        object.web.minimum_font_size = Some(9);
        object.web.images = false;
        let options = launch(&global, &object);
        assert_eq!(options.minimum_font_size, Some(9));
        assert!(options.no_images);
    }

    /// D03: Chromium's default is the other one, so this is sent on every print
    /// and not only when the option was written.
    #[test]
    fn screen_stylesheets_are_asked_for_by_default() {
        assert_eq!(
            emulate_media(&WebSettings::default()).params,
            json!({ "media": "screen" })
        );

        let print_media = WebSettings {
            media_type: MediaType::Print,
            ..WebSettings::default()
        };
        assert_eq!(
            emulate_media(&print_media).params,
            json!({ "media": "print" })
        );
    }

    /// Sending it with `false` would be a command that means nothing, and would
    /// make the plan for a default conversion longer than the conversion.
    #[test]
    fn scripting_is_only_mentioned_when_it_is_being_turned_off() {
        let allowed = prepared(WebSettings::default());
        assert!(
            !allowed
                .iter()
                .any(|command| command.method == "Emulation.setScriptExecutionDisabled")
        );

        let refused = prepared(WebSettings {
            javascript: false,
            ..WebSettings::default()
        });
        assert_eq!(
            refused.last().unwrap(),
            &Command::new(
                "Emulation.setScriptExecutionDisabled",
                json!({ "value": true })
            )
        );
    }

    fn with_header_document() -> ObjectSettings {
        let mut object = page_object();
        object.header.html = Some("header.html".into());
        object.header.spacing = Some(5.0);
        object
    }

    fn margin_top(command: &Command) -> f64 {
        command.params["marginTop"].as_f64().unwrap() * 25.4
    }

    /// wkhtmltopdf's rule for a band that is a document (D39): with no
    /// `--margin-top`, the document's height is the margin. That height is
    /// not known until the document is loaded, so the print call carries the
    /// spacing alone and `reserve` adds the measurement.
    #[test]
    fn a_band_document_replaces_a_margin_that_was_not_written() {
        let command = print(
            &PageSetup::default(),
            &with_header_document(),
            &OutlineSettings::default(),
        );
        assert!((margin_top(&command) - 5.0).abs() < 1e-9, "{command:?}");
        // Untouched on the side that has no document.
        let bottom = command.params["marginBottom"].as_f64().unwrap() * 25.4;
        assert!((bottom - 10.0).abs() < 1e-9, "{command:?}");

        let reserved = reserve(&command, 20.0 / 25.4, 0.0);
        assert!((margin_top(&reserved) - 25.0).abs() < 1e-9, "{reserved:?}");
        assert_eq!(reserved.method, "Page.printToPDF");
        // Only the two margins move.
        assert_eq!(reserved.params["paperWidth"], command.params["paperWidth"]);
    }

    /// `--margin-top 15mm` written: the document is fitted into it, and the
    /// content is pushed in by the spacing alone, as with a text band.
    #[test]
    fn a_band_document_is_fitted_into_a_margin_that_was_written() {
        let named = PageSetup {
            margins: rchtmltopdf_core::settings::Margins {
                top: Length::mm(15.0),
                ..Default::default()
            },
            named: rchtmltopdf_core::settings::NamedMargins {
                top: true,
                bottom: false,
            },
            ..PageSetup::default()
        };
        let command = print(&named, &with_header_document(), &OutlineSettings::default());
        assert!((margin_top(&command) - 20.0).abs() < 1e-9, "{command:?}");
        assert!(!sized_by_its_document(&with_header_document().header, true));
        assert!(sized_by_its_document(&with_header_document().header, false));
    }

    /// A band document is handed every placeholder, so it needs the outline
    /// the sections are read from, whatever its text says.
    #[test]
    fn a_band_document_asks_for_the_outline() {
        let none = OutlineSettings {
            enabled: false,
            ..OutlineSettings::default()
        };
        assert!(!outline_wanted(&none, &page_object()));
        assert!(outline_wanted(&none, &with_header_document()));
    }

    /// The document is laid out at the width the frame will give it.
    #[test]
    fn a_band_document_is_measured_at_the_content_width() {
        let commands = measure_prepare(&PageSetup::default(), &WebSettings::default());
        let metrics = commands
            .iter()
            .find(|command| command.method == "Emulation.setDeviceMetricsOverride")
            .expect("a viewport");
        // A4 less two 10mm margins is 190mm, which is 718 CSS pixels.
        assert_eq!(metrics.params["width"], json!(718));
        assert!(methods(&commands).contains(&"Emulation.setEmulatedMedia"));
    }

    /// One page carries every object's bands, so the rule is the union.
    #[test]
    fn the_sheets_get_the_union_of_the_objects_file_rules() {
        let mut opened = page_object();
        opened.web.local_file_access = true;
        let mut allowed = page_object();
        allowed.web.allowed_paths.push(PathBuf::from("/srv/assets"));
        let policy = band_policy([&opened, &allowed]);
        assert!(policy.enabled);
        assert_eq!(policy.allowed, vec![PathBuf::from("/srv/assets")]);
        assert!(!band_policy([&page_object()]).enabled);
    }

    /// The swap belongs in the paper dimensions, once. Asking the protocol for
    /// landscape as well would turn A4 landscape back into portrait.
    #[test]
    fn orientation_is_in_the_paper_and_not_in_the_flag() {
        use rchtmltopdf_core::page_size::Orientation;

        let landscape = PageSetup {
            orientation: Orientation::Landscape,
            ..PageSetup::default()
        };
        let command = print(&landscape, &page_object(), &OutlineSettings::default());
        assert_eq!(command.params["landscape"], json!(false));
        // 297mm, the long edge, is now the width.
        let width = command.params["paperWidth"].as_f64().unwrap();
        assert!((width - 297.0 / 25.4).abs() < 1e-9, "{width}");
    }

    #[test]
    fn a_stylesheet_is_a_file_to_read_or_a_url_to_fetch() {
        let mut load = LoadSettings::default();
        assert_eq!(LoadPlan::new(&load).inject, None);

        load.user_style_sheet = Some("/srv/print.css".into());
        assert_eq!(
            LoadPlan::new(&load).inject,
            Some(Injection::File(PathBuf::from("/srv/print.css")))
        );

        load.user_style_sheet = Some("https://example.com/print.css".into());
        assert_eq!(
            LoadPlan::new(&load).inject,
            Some(Injection::Link("https://example.com/print.css".into()))
        );

        // A drive letter is not a scheme, here as everywhere else.
        load.user_style_sheet = Some(r"C:\styles\print.css".into());
        assert!(matches!(
            LoadPlan::new(&load).inject,
            Some(Injection::File(_))
        ));
    }

    #[test]
    fn scripts_keep_the_order_they_were_written_in() {
        let load = LoadSettings {
            run_scripts: vec!["first()".into(), "second()".into()],
            ..LoadSettings::default()
        };
        assert_eq!(LoadPlan::new(&load).scripts, ["first()", "second()"]);
    }

    #[test]
    fn a_named_status_replaces_the_delay_rather_than_adding_to_it() {
        let mut load = LoadSettings::default();
        assert_eq!(
            LoadPlan::new(&load).settle,
            Settle::Delay(Duration::from_millis(200))
        );

        load.window_status = Some("ready".into());
        assert_eq!(
            LoadPlan::new(&load).settle,
            Settle::WindowStatus("ready".into())
        );
    }

    #[test]
    fn cookies_are_set_before_the_document_is_asked_for_and_decoded() {
        let web = WebSettings {
            cookies: vec![Pair {
                name: "session".into(),
                value: "abc%20def".into(),
            }],
            ..WebSettings::default()
        };
        let commands = prepared(web.clone());
        let cookies = commands
            .iter()
            .find(|command| command.method == "Network.setCookies")
            .expect("a cookie should be set");

        let first = &cookies.params["cookies"][0];
        // wkhtmltopdf's own help says the value arrives url encoded.
        assert_eq!(first["value"], json!("abc def"));
        assert_eq!(first["url"], json!("https://example.com/doc"));

        // A cookie on a local document has no origin to be scoped to.
        let object = ObjectSettings {
            web,
            ..page_object()
        };
        assert!(
            !methods(&prepare(&object, "file:///doc.html")).contains(&"Network.setCookies"),
            "a file:// document takes no cookies"
        );
    }

    /// The asymmetry is wkhtmltopdf's: without propagation a header is on the
    /// document's own request and on nothing else, which needs the interception
    /// handler; with it, on every request, which is one protocol call.
    #[test]
    fn propagation_decides_which_layer_carries_the_header() {
        let header = Pair {
            name: "X-Tenant".into(),
            value: "acme".into(),
        };
        let mut web = WebSettings {
            custom_headers: vec![header.clone()],
            ..WebSettings::default()
        };

        let object = ObjectSettings {
            web: web.clone(),
            ..page_object()
        };
        assert!(!methods(&prepared(web.clone())).contains(&"Network.setExtraHTTPHeaders"));
        assert_eq!(
            rules(&object, "https://example.com/doc").document_headers,
            [header]
        );

        web.propagate_custom_headers = true;
        let object = ObjectSettings {
            web: web.clone(),
            ..page_object()
        };
        assert!(methods(&prepared(web)).contains(&"Network.setExtraHTTPHeaders"));
        assert!(
            rules(&object, "https://example.com/doc")
                .document_headers
                .is_empty(),
            "the handler has nothing left to add"
        );
    }

    /// Either half is enough to need an answer to a challenge: a server may want
    /// only a username, and a password with no username is a mistake worth
    /// sending rather than silently dropping.
    #[test]
    fn either_half_of_the_credentials_arms_the_handler() {
        let object = |web| ObjectSettings {
            web,
            ..page_object()
        };
        let url = "https://example.com/doc";

        assert!(
            rules(&object(WebSettings::default()), url)
                .credentials
                .is_none()
        );
        for web in [
            WebSettings {
                username: Some("bob".into()),
                ..WebSettings::default()
            },
            WebSettings {
                password: Some("hunter2".into()),
                ..WebSettings::default()
            },
        ] {
            assert!(rules(&object(web), url).credentials.is_some());
        }
    }

    #[test]
    fn the_file_policy_carries_what_the_command_line_asked_for() {
        let open = file_access(&WebSettings {
            local_file_access: true,
            allowed_paths: vec!["/srv/assets".into()],
            ..WebSettings::default()
        });
        assert!(open.enabled);
        assert_eq!(open.allowed, [std::path::PathBuf::from("/srv/assets")]);

        // Off is the default, and D10 is the reason.
        assert!(!file_access(&WebSettings::default()).enabled);
    }

    /// `--no-stop-slow-scripts` is an object option that is honoured by a launch
    /// flag, which is the one place the plan crosses from the page to the
    /// process.
    #[test]
    fn letting_slow_scripts_run_is_a_launch_decision() {
        let global = GlobalSettings::default();
        let mut object = page_object();
        assert!(!launch(&global, &object).allow_slow_scripts);

        object.load.stop_slow_scripts = false;
        assert!(launch(&global, &object).allow_slow_scripts);
    }
}
