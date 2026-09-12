//! A moment, broken down.
//!
//! The shape of one is model: the band placeholders render it, and the PDF's
//! Info dictionary is dated with it, and neither layer should have its own idea
//! of what a date is. Reading the machine's clock needs libc and therefore lives
//! in the browser crate; everything here is pure formatting and is tested
//! without asking what time it is.

/// The wall clock, broken down, with the offset from UTC.
///
/// Read once per conversion and carried, so two bands cannot disagree about what
/// time it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Clock {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// Seconds east of UTC, which is what `tm_gmtoff` gives.
    pub utc_offset_seconds: i32,
}

impl Clock {
    /// `[date]`, as the local calendar date.
    ///
    /// **Not byte-identical to wkhtmltopdf.** Qt renders this through the
    /// system locale, so the same binary prints a different string on two
    /// machines and there is no format to match. An unambiguous one is more use
    /// than a guess at somebody's locale, and `docs/migration.md` says so.
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// `[time]`, as the local wall clock.
    pub fn time(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    /// `[isodate]`, ISO 8601 with the offset, which is the one format that does
    /// not depend on knowing where the reader is.
    pub fn iso(&self) -> String {
        let offset = self.utc_offset_seconds;
        let sign = if offset < 0 { '-' } else { '+' };
        let (hours, minutes) = (offset.abs() / 3600, (offset.abs() % 3600) / 60);
        format!(
            "{}T{}{sign}{hours:02}:{minutes:02}",
            self.date(),
            self.time()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn afternoon() -> Clock {
        Clock {
            year: 2026,
            month: 9,
            day: 12,
            hour: 14,
            minute: 5,
            second: 9,
            utc_offset_seconds: 2 * 3600,
        }
    }

    #[test]
    fn the_three_shapes_a_band_can_ask_for() {
        assert_eq!(afternoon().date(), "2026-09-12");
        assert_eq!(afternoon().time(), "14:05:09");
        assert_eq!(afternoon().iso(), "2026-09-12T14:05:09+02:00");
    }

    #[test]
    fn a_western_offset_is_signed_the_other_way() {
        let behind = Clock {
            utc_offset_seconds: -(5 * 3600 + 30 * 60),
            ..afternoon()
        };
        assert!(behind.iso().ends_with("-05:30"), "{}", behind.iso());
    }

    /// Every field zero is what a failed clock read leaves behind, and it prints
    /// as an obviously wrong date rather than a plausibly wrong one.
    #[test]
    fn the_default_is_visibly_not_a_real_date() {
        assert_eq!(Clock::default().date(), "0000-00-00");
    }
}
