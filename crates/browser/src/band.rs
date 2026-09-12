//! Turning a header or footer into a Chromium print template.
//!
//! # What a template is, and what it is not
//!
//! `Page.printToPDF` takes two fragments of HTML and draws them into the top and
//! bottom margins. They are not ordinary documents, and four things about them
//! catch people out:
//!
//! - **A template cannot load anything.** No external stylesheet, no web font,
//!   no image, and no JavaScript at all. Everything has to be inline, which is
//!   why the markup below carries its styles as attributes. Those limits are
//!   exactly the ones D04 names as the reason `--header-html` waits for the PDF
//!   overlay in V2: an arbitrary document cannot be squeezed through here.
//! - **The default font is about 10px and grey.** A migrated footer that said
//!   nothing about its font would silently shrink and fade, so size, family and
//!   colour are always written out, whether or not the user asked.
//! - **Backgrounds need the colour adjustment property**, or a template that
//!   paints anything comes out white.
//! - **Asking for one band gets you Chromium's other one.** Turning
//!   `displayHeaderFooter` on with only a `headerTemplate` leaves the footer at
//!   Chromium's default, which is a page number nobody asked for. The empty band
//!   has to be sent explicitly, which is what [`EMPTY`] is for.
//!
//! # Where a band lands, and why spacing is not in here
//!
//! Measured against the pinned Chromium rather than assumed. A template is
//! anchored to the **paper edge** and grows towards the content; its height is
//! whatever its content needs. Three consequences, all of them visible in
//! `conformance/tests/bands.rs`:
//!
//! - **A band never moves the content.** The content box of a document printed
//!   with a header sits exactly where the same document without one does. Adding
//!   a header to a migrated command line cannot silently repaginate it.
//! - **The margin is what has to accommodate the band.** A 12pt band is about
//!   28.5pt tall and a 10mm margin is 28.3pt, so the default only just fits. At
//!   `--header-font-size 40` the band runs into the content, and the answer is a
//!   bigger `--margin-top`. That is wkhtmltopdf's arrangement too, and it is why
//!   its `--default-header` is documented as needing room.
//! - **Spacing cannot be padding.** The band is anchored at the top, so padding
//!   below the rule moves the rule *towards* the content, which is backwards,
//!   and padding under an inner rule is invisible because nothing is drawn below
//!   it. The only thing that can open a gap between a band and the content is
//!   the print margin, so `--header-spacing` is added there (see `plan::print`)
//!   and this module knows nothing about it.
//!
//! # Placeholders
//!
//! `[page]`, `[date]` and the rest are expanded by [`crate::placeholder`], which
//! also does the escaping: a band's text is somebody's document title, and
//! `[page]` has to survive as a `<span>` while everything around it does not.

use crate::placeholder::{self, Context};
use rchtmltopdf_core::settings::Band;

/// wkhtmltopdf's default band font, which is not Chromium's.
const FONT: &str = "Arial";

/// wkhtmltopdf's default band size, in points. Chromium's is about 10px, which
/// is two thirds of this.
const SIZE_PT: f64 = 12.0;

/// An explicitly empty band.
///
/// Not an empty string: an empty `footerTemplate` is treated as "no template
/// given", and Chromium falls back to its own. A div that draws nothing is the
/// way to ask for nothing.
pub const EMPTY: &str = "<div></div>";

/// A band's markup, and what it could not expand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Template {
    pub html: String,
    /// Placeholders that are real in wkhtmltopdf and empty here. The binary says
    /// so once per name, however many bands and cells used it.
    pub unsupported: Vec<&'static str>,
}

/// Which end of the page a band is at.
///
/// The only difference is which side the rule goes on, and which way the spacing
/// pushes — but both differences are invisible until you see them the wrong way
/// round on a printed page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Header,
    Footer,
}

/// The template for one band, ready to hand to the print call.
///
/// `left` and `right` are the page's side margins in millimetres: a template
/// spans the full width of the paper, where wkhtmltopdf's band spans the content
/// width, so the difference is padded out here.
pub fn template(
    band: &Band,
    edge: Edge,
    left_mm: f64,
    right_mm: f64,
    context: &Context,
) -> Template {
    if band.is_empty() {
        return Template {
            html: EMPTY.to_string(),
            unsupported: Vec::new(),
        };
    }

    let mut unsupported: Vec<&'static str> = Vec::new();
    let mut cell = |text: Option<&str>| -> String {
        let Some(text) = text else {
            return String::new();
        };
        let expansion = placeholder::expand(text, context);
        for name in expansion.unsupported {
            if !unsupported.contains(&name) {
                unsupported.push(name);
            }
        }
        expansion.html
    };
    let (left, centre, right) = (
        cell(band.left.as_deref()),
        cell(band.center.as_deref()),
        cell(band.right.as_deref()),
    );

    let size = band.font_size.unwrap_or(SIZE_PT);
    let family = band.font_name.as_deref().unwrap_or(FONT);

    // The rule goes between the band and the content, so which side depends on
    // which end of the page this is.
    let rule = match (band.line, edge) {
        (false, _) => String::new(),
        (true, Edge::Header) => "border-bottom:0.5pt solid #000;".to_string(),
        (true, Edge::Footer) => "border-top:0.5pt solid #000;".to_string(),
    };

    let html = format!(
        "<div style=\"\
         -webkit-print-color-adjust:exact;print-color-adjust:exact;\
         box-sizing:border-box;width:100%;margin:0;\
         padding-left:{left_mm}mm;padding-right:{right_mm}mm;\
         font-family:{family};font-size:{size}pt;color:#000;\
         display:flex;align-items:baseline;{rule}\">\
         <div style=\"flex:1 1 0;text-align:left;white-space:pre\">{}</div>\
         <div style=\"flex:1 1 0;text-align:center;white-space:pre\">{}</div>\
         <div style=\"flex:1 1 0;text-align:right;white-space:pre\">{}</div>\
         </div>",
        left,
        centre,
        right,
        family = placeholder::escape(family),
    );

    Template { html, unsupported }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template's markup, expanded against an empty context: these tests are
    /// about layout and escaping, and `placeholder.rs` covers expansion.
    fn markup(band: &Band, edge: Edge, left: f64, right: f64) -> String {
        template(band, edge, left, right, &Context::default()).html
    }

    fn band(center: &str) -> Band {
        Band {
            center: Some(center.to_string()),
            ..Band::default()
        }
    }

    /// An empty string would leave Chromium drawing its own footer, which is a
    /// page number the user did not ask for.
    #[test]
    fn an_empty_band_is_still_a_template() {
        assert_eq!(markup(&Band::default(), Edge::Footer, 10.0, 10.0), EMPTY);
        assert!(!EMPTY.is_empty());
    }

    /// Both are written on every template. Chromium's own defaults are about
    /// 10px and grey, so a migrated band that said nothing about its font would
    /// silently shrink and fade.
    #[test]
    fn the_font_is_wkhtmltopdfs_default_rather_than_chromiums() {
        let html = markup(&band("x"), Edge::Header, 10.0, 10.0);
        assert!(html.contains("font-family:Arial"), "{html}");
        assert!(html.contains("font-size:12pt"), "{html}");
        assert!(html.contains("color:#000"), "{html}");
    }

    #[test]
    fn the_font_can_be_overridden() {
        let asked = Band {
            font_name: Some("Times New Roman".into()),
            font_size: Some(8.0),
            ..band("x")
        };
        let html = markup(&asked, Edge::Header, 10.0, 10.0);
        assert!(html.contains("font-family:Times New Roman"), "{html}");
        assert!(html.contains("font-size:8pt"), "{html}");
    }

    /// The rule goes between the band and the content, so a header's is below it
    /// and a footer's above. Getting this the wrong way round is invisible until
    /// you look at a printed page.
    #[test]
    fn the_rule_is_on_the_side_facing_the_content() {
        let ruled = Band {
            line: true,
            ..band("x")
        };
        assert!(markup(&ruled, Edge::Header, 10.0, 10.0).contains("border-bottom"));
        assert!(markup(&ruled, Edge::Footer, 10.0, 10.0).contains("border-top"));
        // `box-sizing:border-box` is always there, so this looks for the rule
        // itself rather than for the word.
        let plain = markup(&band("x"), Edge::Header, 10.0, 10.0);
        assert!(!plain.contains("border-bottom") && !plain.contains("border-top"));
    }

    /// Spacing is a print margin and not a padding, because the band is anchored
    /// to the paper edge: padding below the rule moves the rule towards the
    /// content, which is the wrong way round. `plan::print` is where it lands.
    #[test]
    fn spacing_does_not_appear_in_the_template_at_all() {
        let spaced = Band {
            spacing: Some(5.0),
            ..band("x")
        };
        let html = markup(&spaced, Edge::Header, 10.0, 10.0);
        assert!(!html.contains("5mm"), "{html}");
        assert_eq!(html, markup(&band("x"), Edge::Header, 10.0, 10.0));
    }

    /// A template spans the paper; wkhtmltopdf's band spans the content. The
    /// difference is the side margins, and without this a centred footer is not
    /// centred on the text above it whenever the two margins differ.
    #[test]
    fn the_band_is_inset_to_the_content_width() {
        let html = markup(&band("x"), Edge::Footer, 15.0, 25.0);
        assert!(html.contains("padding-left:15mm"), "{html}");
        assert!(html.contains("padding-right:25mm"), "{html}");
    }

    /// The text is somebody's document title, and the styles around it are
    /// attributes.
    #[test]
    fn text_is_escaped_so_a_title_cannot_break_the_markup() {
        let nasty = Band {
            left: Some("Tom & Jerry".into()),
            center: Some("<script>alert(1)</script>".into()),
            right: Some("a \"quoted\" thing".into()),
            ..Band::default()
        };
        let html = markup(&nasty, Edge::Header, 10.0, 10.0);
        assert!(html.contains("Tom &amp; Jerry"), "{html}");
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(html.contains("&quot;quoted&quot;"), "{html}");
    }

    /// A font name arrives from the command line and lands inside a style
    /// attribute, so it is escaped like any other text.
    #[test]
    fn a_font_name_cannot_end_the_style_attribute() {
        let injected = Band {
            font_name: Some("Arial\" onload=\"x".into()),
            ..band("x")
        };
        let html = markup(&injected, Edge::Header, 10.0, 10.0);
        assert!(!html.contains("onload=\""), "{html}");
        assert!(html.contains("&quot;"), "{html}");
    }

    /// A template that paints anything comes out white without it.
    #[test]
    fn colours_are_asked_to_print() {
        let html = markup(&band("x"), Edge::Header, 10.0, 10.0);
        assert!(html.contains("print-color-adjust:exact"), "{html}");
    }

    /// The three cells share the width evenly, so the centre one is centred on
    /// the page rather than on whatever is left after the other two.
    #[test]
    fn the_three_cells_divide_the_width_evenly() {
        let html = markup(&band("x"), Edge::Header, 10.0, 10.0);
        assert_eq!(html.matches("flex:1 1 0").count(), 3, "{html}");
    }
}
