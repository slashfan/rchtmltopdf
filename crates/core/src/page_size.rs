//! Named page sizes.
//!
//! wkhtmltopdf accepts the names of Qt's `QPrinter::PageSize` enum, so this
//! table follows Qt rather than any PDF or CSS standard. Dimensions are the
//! portrait extent in millimetres.

use crate::units::Length;
use std::fmt;

/// Portrait dimensions of a page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageDimensions {
    pub width: Length,
    pub height: Length,
}

impl PageDimensions {
    pub const fn mm(width: f64, height: f64) -> Self {
        Self {
            width: Length::mm(width),
            height: Length::mm(height),
        }
    }

    /// Swap width and height.
    pub fn landscape(self) -> Self {
        Self {
            width: self.height,
            height: self.width,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
}

impl Orientation {
    /// wkhtmltopdf matches orientation names case-insensitively.
    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "portrait" => Some(Orientation::Portrait),
            "landscape" => Some(Orientation::Landscape),
            _ => None,
        }
    }
}

impl fmt::Display for Orientation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Orientation::Portrait => write!(f, "Portrait"),
            Orientation::Landscape => write!(f, "Landscape"),
        }
    }
}

/// Every page size name wkhtmltopdf accepts, with its portrait size in mm.
///
/// Taken from Qt 4's `QPrinter::PageSize`. The B series is ISO B, not JIS B.
/// Ledger is listed at its natural landscape extent, matching Qt, and is the one
/// entry whose width exceeds its height.
pub const PAGE_SIZES: &[(&str, PageDimensions)] = &[
    ("A0", PageDimensions::mm(841.0, 1189.0)),
    ("A1", PageDimensions::mm(594.0, 841.0)),
    ("A2", PageDimensions::mm(420.0, 594.0)),
    ("A3", PageDimensions::mm(297.0, 420.0)),
    ("A4", PageDimensions::mm(210.0, 297.0)),
    ("A5", PageDimensions::mm(148.0, 210.0)),
    ("A6", PageDimensions::mm(105.0, 148.0)),
    ("A7", PageDimensions::mm(74.0, 105.0)),
    ("A8", PageDimensions::mm(52.0, 74.0)),
    ("A9", PageDimensions::mm(37.0, 52.0)),
    ("B0", PageDimensions::mm(1000.0, 1414.0)),
    ("B1", PageDimensions::mm(707.0, 1000.0)),
    ("B2", PageDimensions::mm(500.0, 707.0)),
    ("B3", PageDimensions::mm(353.0, 500.0)),
    ("B4", PageDimensions::mm(250.0, 353.0)),
    ("B5", PageDimensions::mm(176.0, 250.0)),
    ("B6", PageDimensions::mm(125.0, 176.0)),
    ("B7", PageDimensions::mm(88.0, 125.0)),
    ("B8", PageDimensions::mm(62.0, 88.0)),
    ("B9", PageDimensions::mm(44.0, 62.0)),
    ("B10", PageDimensions::mm(31.0, 44.0)),
    ("C5E", PageDimensions::mm(163.0, 229.0)),
    ("Comm10E", PageDimensions::mm(105.0, 241.0)),
    ("DLE", PageDimensions::mm(110.0, 220.0)),
    ("Executive", PageDimensions::mm(190.5, 254.0)),
    ("Folio", PageDimensions::mm(210.0, 330.0)),
    ("Ledger", PageDimensions::mm(431.8, 279.4)),
    ("Legal", PageDimensions::mm(215.9, 355.6)),
    ("Letter", PageDimensions::mm(215.9, 279.4)),
    ("Tabloid", PageDimensions::mm(279.4, 431.8)),
];

/// Look up a page size by name, case-insensitively.
pub fn lookup(name: &str) -> Option<PageDimensions> {
    let wanted = name.trim();
    PAGE_SIZES
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(wanted))
        .map(|(_, dimensions)| *dimensions)
}

/// The canonical spelling of a page size name, for echoing back to the user.
pub fn canonical_name(name: &str) -> Option<&'static str> {
    let wanted = name.trim();
    PAGE_SIZES
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(wanted))
        .map(|(candidate, _)| *candidate)
}

/// Every accepted name, for error messages.
pub fn all_names() -> impl Iterator<Item = &'static str> {
    PAGE_SIZES.iter().map(|(name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_is_210_by_297() {
        let a4 = lookup("A4").unwrap();
        assert_eq!(a4.width.to_mm(), 210.0);
        assert_eq!(a4.height.to_mm(), 297.0);
    }

    #[test]
    fn lookup_ignores_case() {
        assert_eq!(lookup("a4"), lookup("A4"));
        assert_eq!(lookup("letter"), lookup("Letter"));
        assert_eq!(canonical_name("comm10e"), Some("Comm10E"));
    }

    #[test]
    fn letter_is_8_5_by_11_inches() {
        let letter = lookup("Letter").unwrap();
        assert!((letter.width.to_inches() - 8.5).abs() < 1e-6);
        assert!((letter.height.to_inches() - 11.0).abs() < 1e-6);
    }

    #[test]
    fn unknown_size_is_none() {
        assert!(lookup("A11").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn landscape_swaps_axes() {
        let a4 = lookup("A4").unwrap().landscape();
        assert_eq!(a4.width.to_mm(), 297.0);
        assert_eq!(a4.height.to_mm(), 210.0);
    }

    #[test]
    fn names_are_unique() {
        let mut seen: Vec<String> = PAGE_SIZES
            .iter()
            .map(|(n, _)| n.to_ascii_lowercase())
            .collect();
        let count = seen.len();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), count, "duplicate page size name in table");
    }

    #[test]
    fn orientation_parses() {
        assert_eq!(
            Orientation::parse("Landscape"),
            Some(Orientation::Landscape)
        );
        assert_eq!(Orientation::parse("portrait"), Some(Orientation::Portrait));
        assert_eq!(Orientation::parse("sideways"), None);
    }
}
