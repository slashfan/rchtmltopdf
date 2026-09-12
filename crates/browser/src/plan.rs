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
//! The one thing a plan cannot finish deciding is the local file policy, because
//! what it permits depends on the document it is bound to and the document is
//! resolved after the settings are read. The policy itself is here; binding it
//! is [`crate::file_access::Policy::about`].
//!
//! [`Page::prepare`]: crate::launch::Page::prepare
//! [`Page::print_to_pdf`]: crate::launch::Page::print_to_pdf
//! [`Page::load`]: crate::launch::Page::load

use crate::file_access::Policy;
use crate::launch::LaunchOptions;
use rchtmltopdf_core::settings::{
    GlobalSettings, LoadSettings, MediaType, ObjectSettings, PageSetup, WebSettings,
};
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
    /// The bound over the whole of the above (D16). `None` is `--timeout 0`.
    pub deadline: Option<Duration>,
    /// What to read a local document as, from `--encoding`.
    ///
    /// Like the file policy below, only half decided here: a document fetched
    /// over http or https is read as its server said and this does not apply.
    pub encoding: Option<String>,
    /// Which local files the document may read (D10).
    ///
    /// One of the two halves of a conversion the settings do not finish
    /// deciding. What a
    /// policy does depends on the document it is bound to — whether that
    /// document is itself local, and where it sits — and the document is
    /// resolved after the settings are read, because standard input becomes a
    /// file whose name nothing could predict. `Policy::about` is where the two
    /// meet.
    pub file_access: Policy,
}

impl Plan {
    pub fn new(global: &GlobalSettings, object: &ObjectSettings) -> Self {
        Self {
            launch: launch(global, object),
            prepare: prepare(&object.web),
            load: LoadPlan::new(&object.load),
            print: print(&global.page, &object.web),
            deadline: global.timeout,
            encoding: object.web.encoding.clone(),
            file_access: file_access(&object.web),
        }
    }
}

/// Which local files the document may read.
pub fn file_access(web: &WebSettings) -> Policy {
    Policy {
        enabled: web.local_file_access,
        allowed: web.allowed_paths.clone(),
    }
}

/// The browser to start.
pub fn launch(global: &GlobalSettings, object: &ObjectSettings) -> LaunchOptions {
    LaunchOptions {
        no_sandbox: global.browser.no_sandbox,
        extra_args: global.browser.extra_args.clone(),
        allow_slow_scripts: !object.load.stop_slow_scripts,
        // Blink settings rather than protocol commands, so they have to be
        // decided before the browser starts rather than before the page loads.
        minimum_font_size: object.web.minimum_font_size,
        no_images: !object.web.images,
        ..LaunchOptions::default()
    }
}

/// What to put in place before the document arrives.
pub fn prepare(web: &WebSettings) -> Vec<Command> {
    // The two domains have to be enabled before anything is expected from them:
    // the load event for a small document can fire before the navigate call has
    // returned.
    let mut commands = vec![
        Command::new("Page.enable", Value::Null),
        Command::new("Network.enable", Value::Null),
        emulate_media(web),
        viewport(web),
    ];

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

/// The print call.
///
/// Orientation is resolved here rather than passed on, so the paper handed over
/// is already the right way round. Letting the protocol rotate it as well would
/// apply the swap twice. Paper geometry is global in wkhtmltopdf; backgrounds
/// and zoom belong to the object. Both are needed, and they come from different
/// places.
pub fn print(page: &PageSetup, web: &WebSettings) -> Command {
    Command::new(
        "Page.printToPDF",
        json!({
            "paperWidth": page.width_inches(),
            "paperHeight": page.height_inches(),
            "marginTop": page.margins.top.to_inches(),
            "marginBottom": page.margins.bottom.to_inches(),
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
            // Not the default. The default returns the whole document
            // base64-encoded inside one protocol message, and a document of any
            // size exceeds the message limit.
            "transferMode": "ReturnAsStream",
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchtmltopdf_core::Input;

    fn page_object() -> ObjectSettings {
        ObjectSettings::page(Input::Stdin)
    }

    #[test]
    fn the_two_domains_are_enabled_before_anything_is_expected_of_them() {
        let commands = prepare(&WebSettings::default());
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
        let allowed = prepare(&WebSettings::default());
        assert!(
            !allowed
                .iter()
                .any(|command| command.method == "Emulation.setScriptExecutionDisabled")
        );

        let refused = prepare(&WebSettings {
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

    /// The swap belongs in the paper dimensions, once. Asking the protocol for
    /// landscape as well would turn A4 landscape back into portrait.
    #[test]
    fn orientation_is_in_the_paper_and_not_in_the_flag() {
        use rchtmltopdf_core::page_size::Orientation;

        let landscape = PageSetup {
            orientation: Orientation::Landscape,
            ..PageSetup::default()
        };
        let command = print(&landscape, &WebSettings::default());
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
