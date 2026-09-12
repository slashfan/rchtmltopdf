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

/// What the wait ladder has been asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadPlan {
    pub settle: Settle,
}

impl LoadPlan {
    pub fn new(load: &LoadSettings) -> Self {
        Self {
            // Naming a status replaces the delay rather than adding to it, which
            // is what wkhtmltopdf does: the delay is the fallback for having no
            // signal, and a page that publishes one does not need it.
            settle: match &load.window_status {
                Some(wanted) => Settle::WindowStatus(wanted.clone()),
                None => Settle::Delay(load.javascript_delay),
            },
        }
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
    /// Which local files the document may read (D10).
    ///
    /// The only half of a conversion the settings do not finish deciding. What a
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
    fn the_two_domains_are_enabled_before_the_media_is_chosen() {
        let commands = prepare(&WebSettings::default());
        let methods: Vec<&str> = commands.iter().map(|command| command.method).collect();
        assert_eq!(
            methods,
            [
                "Page.enable",
                "Network.enable",
                "Emulation.setEmulatedMedia"
            ]
        );
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
