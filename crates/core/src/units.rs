//! Length values with wkhtmltopdf's `<unitreal>` syntax.
//!
//! wkhtmltopdf accepts a bare number (interpreted as millimetres) or a number
//! followed by a unit suffix. Everything is normalised to inches internally
//! because that is what `Page.printToPDF` expects.

use std::fmt;
use std::str::FromStr;

/// Units accepted as a suffix on a `<unitreal>` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Millimeter,
    Centimeter,
    Inch,
    /// PostScript point, 1/72 inch.
    Point,
    /// CSS pixel, 1/96 inch. Not accepted by wkhtmltopdf; see `Length::parse`.
    Pixel,
}

impl Unit {
    /// How many inches one of this unit is worth.
    pub const fn inches_per_unit(self) -> f64 {
        match self {
            Unit::Millimeter => 1.0 / 25.4,
            Unit::Centimeter => 10.0 / 25.4,
            Unit::Inch => 1.0,
            Unit::Point => 1.0 / 72.0,
            Unit::Pixel => 1.0 / 96.0,
        }
    }

    pub const fn suffix(self) -> &'static str {
        match self {
            Unit::Millimeter => "mm",
            Unit::Centimeter => "cm",
            Unit::Inch => "in",
            Unit::Point => "pt",
            Unit::Pixel => "px",
        }
    }
}

/// A length: a magnitude plus the unit it was written in.
///
/// The original unit is kept so error messages and `Display` can echo back what
/// the user typed rather than a converted value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    pub value: f64,
    pub unit: Unit,
}

impl Length {
    pub const fn new(value: f64, unit: Unit) -> Self {
        Self { value, unit }
    }

    pub const fn mm(value: f64) -> Self {
        Self::new(value, Unit::Millimeter)
    }

    pub const fn inches(value: f64) -> Self {
        Self::new(value, Unit::Inch)
    }

    /// The length in inches, which is what `Page.printToPDF` takes.
    ///
    /// A length already written in inches is returned unchanged rather than
    /// multiplied by one, so no precision is lost on the way through.
    pub fn to_inches(self) -> f64 {
        match self.unit {
            Unit::Inch => self.value,
            _ => self.value * self.unit.inches_per_unit(),
        }
    }

    /// The length in millimetres.
    ///
    /// Millimetres and centimetres are converted directly. Going via inches
    /// would round-trip through a non-representable factor, so `15mm` would not
    /// come back as exactly `15.0`, and the error differs between architectures
    /// depending on whether the compiler contracts the two operations into a
    /// fused multiply-add.
    pub fn to_mm(self) -> f64 {
        match self.unit {
            Unit::Millimeter => self.value,
            Unit::Centimeter => self.value * 10.0,
            _ => self.to_inches() * 25.4,
        }
    }

    /// Parse a wkhtmltopdf `<unitreal>`.
    ///
    /// A bare number means millimetres, matching wkhtmltopdf. Whitespace between
    /// the number and the unit is tolerated, as is any letter case.
    ///
    /// `px` and `pt` are accepted as an extension: wkhtmltopdf rejects them, so
    /// accepting them can only turn an error into a sensible result.
    pub fn parse(input: &str) -> Result<Self, LengthParseError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(LengthParseError::Empty);
        }
        let lower = trimmed.to_ascii_lowercase();

        let (number_part, unit) = match Self::split_suffix(&lower) {
            Some((head, unit)) => (head, unit),
            None => (lower.as_str(), Unit::Millimeter),
        };

        let number_part = number_part.trim_end();
        if number_part.is_empty() {
            return Err(LengthParseError::NoNumber {
                input: trimmed.to_string(),
            });
        }

        let value: f64 = number_part
            .parse()
            .map_err(|_| LengthParseError::NotANumber {
                input: trimmed.to_string(),
            })?;

        if !value.is_finite() {
            return Err(LengthParseError::NotFinite {
                input: trimmed.to_string(),
            });
        }

        Ok(Self { value, unit })
    }

    /// Split a known unit suffix off an already-lowercased string.
    fn split_suffix(lower: &str) -> Option<(&str, Unit)> {
        const SUFFIXES: [(&str, Unit); 5] = [
            ("mm", Unit::Millimeter),
            ("cm", Unit::Centimeter),
            ("in", Unit::Inch),
            ("pt", Unit::Point),
            ("px", Unit::Pixel),
        ];
        for (suffix, unit) in SUFFIXES {
            if let Some(head) = lower.strip_suffix(suffix) {
                return Some((head, unit));
            }
        }
        None
    }
}

impl FromStr for Length {
    type Err = LengthParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.value, self.unit.suffix())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LengthParseError {
    Empty,
    NoNumber { input: String },
    NotANumber { input: String },
    NotFinite { input: String },
}

impl fmt::Display for LengthParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LengthParseError::Empty => write!(f, "expected a length, got an empty value"),
            LengthParseError::NoNumber { input } => {
                write!(f, "`{input}` has a unit but no number")
            }
            LengthParseError::NotANumber { input } => {
                write!(
                    f,
                    "`{input}` is not a length; expected a number optionally followed by mm, cm, in, pt or px"
                )
            }
            LengthParseError::NotFinite { input } => {
                write!(f, "`{input}` is not a finite length")
            }
        }
    }
}

impl std::error::Error for LengthParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn bare_number_is_millimeters() {
        let l = Length::parse("10").unwrap();
        assert_eq!(l.unit, Unit::Millimeter);
        approx(l.to_mm(), 10.0);
    }

    #[test]
    fn parses_every_suffix() {
        approx(Length::parse("25.4mm").unwrap().to_inches(), 1.0);
        approx(Length::parse("2.54cm").unwrap().to_inches(), 1.0);
        approx(Length::parse("1in").unwrap().to_inches(), 1.0);
        approx(Length::parse("72pt").unwrap().to_inches(), 1.0);
        approx(Length::parse("96px").unwrap().to_inches(), 1.0);
    }

    #[test]
    fn tolerates_case_and_whitespace() {
        approx(Length::parse("  1 IN ").unwrap().to_inches(), 1.0);
        approx(Length::parse("15MM").unwrap().to_mm(), 15.0);
    }

    #[test]
    fn accepts_negative_and_fractional() {
        approx(Length::parse("-5mm").unwrap().to_mm(), -5.0);
        approx(Length::parse("0.5in").unwrap().to_inches(), 0.5);
        approx(Length::parse(".5in").unwrap().to_inches(), 0.5);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Length::parse("").is_err());
        assert!(Length::parse("mm").is_err());
        assert!(Length::parse("abc").is_err());
        assert!(Length::parse("10 furlongs").is_err());
        assert!(Length::parse("inf").is_err());
    }

    #[test]
    fn display_round_trips() {
        let l = Length::parse("15mm").unwrap();
        assert_eq!(l.to_string(), "15mm");
        assert_eq!(Length::parse(&l.to_string()).unwrap(), l);
    }
}
