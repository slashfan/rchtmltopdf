//! Turning headers and footers into a document of sheets, one per page.
//!
//! # Why the bands are a document of their own
//!
//! Until D38 the bands were Chromium's print templates: two fragments of HTML
//! the print call draws into the margins. A template cannot run a script,
//! cannot load anything, and above all cannot count: it knows the page number
//! within the document being printed and nothing else, so `[page]` restarted
//! at one for every document of a conversion and `--page-offset` could not be
//! honoured at all (#38, #39).
//!
//! So the bands are printed separately, after everything else has been. One
//! HTML document is built with one **sheet** per page of the merged output,
//! each sheet the size of the paper, carrying the header and footer for its
//! page with every placeholder already expanded — the numbers are known by
//! then. Chromium prints that document, and `rchtmltopdf_pdf::stamp` draws
//! each sheet onto its page. Typography stays Chromium's: fonts, shaping,
//! measurement, and the text extraction the compatibility matrix relies on.
//!
//! # Where a band lands
//!
//! A band is anchored to the **paper edge** and reaches towards the content,
//! and its box is at least as tall as the margin on that side. Three
//! consequences, all of them held by `conformance/tests/bands.rs`:
//!
//! - **A band never moves the content.** The content box of a document printed
//!   with a header sits exactly where the same document without one does.
//!   Adding a header to a migrated command line cannot silently repaginate it.
//! - **The rule sits on the margin line.** The band's row is aligned to the
//!   content side of its box, so with a header the rule is exactly the top
//!   margin down from the paper edge, right above the content.
//! - **A band taller than the margin grows into the content.** The box has a
//!   minimum height, not a fixed one, so a 40pt band in a 10mm margin runs over
//!   the text rather than being clipped at the paper edge. That is
//!   wkhtmltopdf's arrangement too, and why its `--default-header` is
//!   documented as needing room: the margin is what has to accommodate the
//!   band.
//!
//! **Spacing is not in here.** `--header-spacing` opens a gap between the band
//! and the content, and the only thing that can move the content is the print
//! margin, so it is added there (see `plan::print`). The band's own box is
//! sized by the margin alone, which keeps the rule where it was.
//!
//! # Exactly one sheet per page
//!
//! Every sheet is the paper's exact size, with `overflow: hidden` and a page
//! break after it, so nothing a band does can spill a sheet onto a second
//! page. `stamp` checks the count and refuses a mismatch rather than drawing
//! page three's footer on page four.
//!
//! # Placeholders
//!
//! `[page]`, `[date]` and the rest are expanded by [`crate::placeholder`], which
//! also does the escaping: a band's text is somebody's document title.
//!
//! # A band that is a document
//!
//! `--header-html` names a document rather than three cells, and the sheet
//! carries it in a frame: the document is loaded once per page, with the
//! placeholders appended as a query string the way wkhtmltopdf passed them,
//! so its own script can read `document.location.search` (D39). Where the
//! frame lands follows wkhtmltopdf's rule for it, which is not the text
//! band's:
//!
//! - **The margin was not written.** The document's height *is* the margin:
//!   the frame starts at the paper edge, and the content starts its measured
//!   height plus `--header-spacing` further in. `plan::print` reserves that,
//!   from a measurement taken before the pages are printed.
//! - **The margin was written.** The document is fitted into the margin asked
//!   for, its edge nearest the content on the margin line, and the content is
//!   pushed in by the spacing alone. A document taller than the margin runs
//!   off the paper, which is wkhtmltopdf's arrangement too.
//!
//! The frame is as tall as the document's body reaches, so nothing a band
//! draws is clipped at its own edge; what is reserved is the body's height,
//! which is what wkhtmltopdf measured.

use crate::placeholder::{self, Context, Numbers};
use rchtmltopdf_core::settings::{Band, PageSetup};

/// wkhtmltopdf's default band font, which is not Chromium's.
const FONT: &str = "Arial";

/// wkhtmltopdf's default band size, in points. Chromium's is about 10px, which
/// is two thirds of this.
const SIZE_PT: f64 = 12.0;

/// Which end of the page a band is at.
///
/// The only difference is which side the rule goes on, and which way the band
/// reaches — but both differences are invisible until you see them the wrong
/// way round on a printed page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Header,
    Footer,
}

/// The header and footer of one page, as rows of markup ready to place.
///
/// Empty strings for a page that has none: the sheet is still there, blank, so
/// the sheets and the pages stay in step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sheet {
    pub header: String,
    pub footer: String,
}

/// One band's row: three cells, the font, and the rule if asked.
///
/// `left` and `right` are the page's side margins in millimetres: the row spans
/// the full width of the paper, where wkhtmltopdf's band spans the content
/// width, so the difference is padded out here. Empty for an empty band.
pub fn row(
    band: &Band,
    edge: Edge,
    left_mm: f64,
    right_mm: f64,
    context: &Context,
    numbers: &Numbers,
) -> String {
    if band.is_empty() {
        return String::new();
    }

    let cell = |text: Option<&str>| -> String {
        text.map(|text| placeholder::expand(text, context, numbers))
            .unwrap_or_default()
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

    format!(
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
    )
}

/// What a band document measures, from a page that loaded it at the
/// content width.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Measured {
    /// The body's height, in millimetres: what wkhtmltopdf reserved for it.
    pub height_mm: f64,
    /// How far down the body reaches, in millimetres: the frame's height, so
    /// a body with a margin above it is not cut off below.
    pub extent_mm: f64,
}

/// One band that is a document, framed for one page.
///
/// `url` already carries the page's query string. The frame spans the content
/// width, inset by the side margins like a text band, and sits where
/// wkhtmltopdf drew it: from the paper edge when the margin was not written,
/// fitted into the margin when it was (see the module documentation).
///
/// **`--zoom` reaches the band** (D61). Measured on wkhtmltopdf 0.12.6.1, the
/// same word in a header document is 114.2 pt wide at `--zoom 1` and 55.0 at
/// `--zoom 0.5`: a band is scaled with the document it frames. So the frame is
/// laid out at the content width **divided** by the zoom — the width the band
/// was measured at — and painted back down by `scale`, which is what a zoom is.
/// At `--zoom 1` that is `scale(1)` on a frame of the content width, and every
/// number below is the one it was before.
pub fn frame(url: &str, edge: Edge, paper: &PageSetup, measured: &Measured, zoom: f64) -> String {
    let (left, right) = (paper.margins.left.to_mm(), paper.margins.right.to_mm());
    let width = (paper.effective_size().width.to_mm() - left - right).max(0.0);
    // The band was measured in a viewport the zoom widened, so its measurements
    // are in that layout's millimetres. What lands on the paper is those times
    // the zoom; what the frame is laid out at is the layout's own numbers.
    let laid_out = (width / zoom).max(0.0);
    let painted = measured.height_mm * zoom;

    let (named, margin) = match edge {
        Edge::Header => (paper.named.top, paper.margins.top.to_mm()),
        Edge::Footer => (paper.named.bottom, paper.margins.bottom.to_mm()),
    };
    // The box the band gets: the margin if one was asked for, its own height
    // if not.
    let box_mm = if named { margin } else { painted };

    // The box is anchored to the paper edge whatever the container does with
    // its free space, and the frame within the box is aligned to the content
    // side: a header's bottom on the margin line, a footer's top on it. For a
    // header that is an offset from the box's top, negative when the document
    // is taller than the margin it was fitted into.
    let (anchor, offset) = match edge {
        Edge::Header => ("margin-bottom:auto;", box_mm - painted),
        Edge::Footer => ("margin-top:auto;", 0.0),
    };

    format!(
        "<div style=\"box-sizing:border-box;width:100%;height:{box_mm}mm;\
         padding-left:{left}mm;padding-right:{right}mm;{anchor}\">\
         <iframe src=\"{}\" style=\"display:block;border:0;margin:{offset}mm 0 0 0;\
         padding:0;width:{laid_out}mm;height:{}mm;\
         transform:scale({zoom});transform-origin:top left\"></iframe>\
         </div>",
        placeholder::escape(url),
        measured.extent_mm,
    )
}

/// The document of sheets, one per page, ready to print with no margins.
pub fn document(paper: &PageSetup, sheets: &[Sheet]) -> String {
    let size = paper.effective_size();
    let (width, height) = (size.width.to_mm(), size.height.to_mm());
    let (top, bottom) = (paper.margins.top.to_mm(), paper.margins.bottom.to_mm());

    let mut html = format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>bands</title><style>\n\
         @page {{ size: {width}mm {height}mm; margin: 0; }}\n\
         html, body {{ margin: 0; padding: 0; }}\n\
         .sheet {{ position: relative; width: {width}mm; height: {height}mm; \
         overflow: hidden; page-break-after: always; }}\n\
         .sheet:last-child {{ page-break-after: auto; }}\n\
         .header, .footer {{ position: absolute; left: 0; right: 0; \
         display: flex; flex-direction: column; }}\n\
         .header {{ top: 0; min-height: {top}mm; justify-content: flex-end; }}\n\
         .footer {{ bottom: 0; min-height: {bottom}mm; justify-content: flex-start; }}\n\
         </style></head><body>\n"
    );
    for sheet in sheets {
        html.push_str("<div class=\"sheet\">");
        if !sheet.header.is_empty() {
            html.push_str("<div class=\"header\">");
            html.push_str(&sheet.header);
            html.push_str("</div>");
        }
        if !sheet.footer.is_empty() {
            html.push_str("<div class=\"footer\">");
            html.push_str(&sheet.footer);
            html.push_str("</div>");
        }
        html.push_str("</div>\n");
    }
    html.push_str("</body></html>\n");
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchtmltopdf_core::settings::{Margins, NamedMargins};
    use rchtmltopdf_core::units::Length;

    /// A row's markup, expanded against an empty context: these tests are about
    /// layout and escaping, and `placeholder.rs` covers expansion.
    fn markup(band: &Band, edge: Edge, left: f64, right: f64) -> String {
        row(
            band,
            edge,
            left,
            right,
            &Context::default(),
            &Numbers::default(),
        )
    }

    fn band(center: &str) -> Band {
        Band {
            center: Some(center.to_string()),
            ..Band::default()
        }
    }

    #[test]
    fn an_empty_band_is_no_row_at_all() {
        assert_eq!(markup(&Band::default(), Edge::Footer, 10.0, 10.0), "");
    }

    /// Both are written on every row. Chromium's own defaults are about 10px
    /// and grey, so a migrated band that said nothing about its font would
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
    fn spacing_does_not_appear_in_the_row_at_all() {
        let spaced = Band {
            spacing: Some(5.0),
            ..band("x")
        };
        let html = markup(&spaced, Edge::Header, 10.0, 10.0);
        assert!(!html.contains("5mm"), "{html}");
        assert_eq!(html, markup(&band("x"), Edge::Header, 10.0, 10.0));
    }

    /// A row spans the paper; wkhtmltopdf's band spans the content. The
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

    /// A row that paints anything comes out white without it.
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

    // --- a band that is a document ------------------------------------------------

    fn measured() -> Measured {
        Measured {
            height_mm: 20.0,
            extent_mm: 22.0,
        }
    }

    /// **The zoom reaches the frame** (D61): the band is laid out at the
    /// content width divided by it and painted back down, and the box it
    /// takes is its measured height times it. At 1 every number is the one
    /// the tests above measure.
    #[test]
    fn the_frame_is_laid_out_wide_and_painted_down_by_the_zoom() {
        let whole = frame("file:///h.html", Edge::Header, &paper(), &measured(), 1.0);
        assert!(whole.contains("transform:scale(1)"), "{whole}");

        let half = frame("file:///h.html", Edge::Header, &paper(), &measured(), 0.5);
        assert!(half.contains("transform:scale(0.5)"), "{half}");
        // The paper is 210mm wide with 10mm margins, so the content is 190mm
        // and the frame twice that.
        assert!(half.contains("width:380mm"), "{half}");
        // The box is the measured 20mm painted at a half.
        assert!(half.contains("height:10mm;"), "{half}");
    }

    /// The URL is somebody's path, and it lands in an attribute.
    #[test]
    fn the_frame_names_the_document_escaped() {
        let html = frame(
            "file:///h.html?title=a%20%26%20b",
            Edge::Header,
            &paper(),
            &measured(),
            1.0,
        );
        assert!(
            html.contains("<iframe src=\"file:///h.html?title=a%20%26%20b\""),
            "{html}"
        );
        let nasty = frame("x\" onload=\"y", Edge::Header, &paper(), &measured(), 1.0);
        assert!(!nasty.contains("onload=\""), "{nasty}");
    }

    /// Like a text band, the frame spans the content and not the paper.
    #[test]
    fn the_frame_is_inset_to_the_content_width() {
        let html = frame("file:///h.html", Edge::Footer, &paper(), &measured(), 1.0);
        assert!(
            html.contains("padding-left:10mm;padding-right:10mm"),
            "{html}"
        );
        assert!(html.contains("width:190mm"), "{html}");
    }

    /// Nothing written: the document's own height is the box, the frame
    /// starts at the paper edge, and is as tall as the body reaches.
    #[test]
    fn an_unnamed_margin_is_replaced_by_the_documents_height() {
        let html = frame("file:///h.html", Edge::Header, &paper(), &measured(), 1.0);
        assert!(html.contains("height:20mm;"), "{html}");
        assert!(html.contains("margin:0mm 0 0 0"), "{html}");
        // The extent, which the transform now follows rather than ends.
        assert!(html.contains("height:22mm;"), "{html}");
        assert!(html.contains("margin-bottom:auto"), "{html}");
    }

    /// `--margin-top 15mm` written: the box is the margin, and the document's
    /// bottom sits on the margin line, so a 20mm document starts 5mm above
    /// the paper.
    #[test]
    fn a_named_margin_fits_the_document_against_the_content() {
        let named = PageSetup {
            named: NamedMargins {
                top: true,
                bottom: true,
            },
            ..paper()
        };
        let header = frame("file:///h.html", Edge::Header, &named, &measured(), 1.0);
        assert!(header.contains("height:15mm;"), "{header}");
        assert!(header.contains("margin:-5mm 0 0 0"), "{header}");

        // A footer's top is on the margin line either way, so it needs no
        // offset: the box, 20mm, is anchored to the paper's bottom.
        let footer = frame("file:///h.html", Edge::Footer, &named, &measured(), 1.0);
        assert!(footer.contains("height:20mm;"), "{footer}");
        assert!(footer.contains("margin:0mm 0 0 0"), "{footer}");
        assert!(footer.contains("margin-top:auto"), "{footer}");
    }

    // --- the document of sheets --------------------------------------------------

    fn paper() -> PageSetup {
        PageSetup {
            margins: Margins {
                top: Length::mm(15.0),
                bottom: Length::mm(20.0),
                ..Margins::default()
            },
            ..PageSetup::default()
        }
    }

    /// One sheet per page, blank ones included, so the sheets and the pages
    /// stay in step.
    #[test]
    fn there_is_one_sheet_per_page_blank_or_not() {
        let sheets = [
            Sheet {
                header: "<div>H1</div>".into(),
                footer: String::new(),
            },
            Sheet::default(),
            Sheet {
                header: String::new(),
                footer: "<div>F3</div>".into(),
            },
        ];
        let html = document(&paper(), &sheets);
        assert_eq!(html.matches("class=\"sheet\"").count(), 3, "{html}");
        assert_eq!(html.matches("class=\"header\"").count(), 1, "{html}");
        assert_eq!(html.matches("class=\"footer\"").count(), 1, "{html}");
        assert!(html.contains("H1") && html.contains("F3"));
    }

    /// The sheet is the paper, exactly, and cannot spill.
    #[test]
    fn a_sheet_is_the_papers_size_and_clips() {
        let html = document(&paper(), &[Sheet::default()]);
        assert!(
            html.contains("@page { size: 210mm 297mm; margin: 0; }"),
            "{html}"
        );
        assert!(html.contains("width: 210mm; height: 297mm"), "{html}");
        assert!(
            html.contains("overflow: hidden; page-break-after: always"),
            "{html}"
        );
        assert!(html.contains("page-break-after: auto"), "{html}");
    }

    /// The box is at least the margin, and the row sits on the content side of
    /// it: the header's at its bottom, the footer's at its top.
    #[test]
    fn a_band_fills_its_margin_from_the_paper_edge() {
        let html = document(&paper(), &[Sheet::default()]);
        assert!(
            html.contains(".header { top: 0; min-height: 15mm; justify-content: flex-end; }"),
            "{html}"
        );
        assert!(
            html.contains(".footer { bottom: 0; min-height: 20mm; justify-content: flex-start; }"),
            "{html}"
        );
    }

    /// Landscape swaps the sheet, as it swaps the paper.
    #[test]
    fn the_sheet_follows_the_orientation() {
        let landscape = PageSetup {
            orientation: rchtmltopdf_core::page_size::Orientation::Landscape,
            ..PageSetup::default()
        };
        let html = document(&landscape, &[Sheet::default()]);
        assert!(
            html.contains("@page { size: 297mm 210mm; margin: 0; }"),
            "{html}"
        );
    }
}
