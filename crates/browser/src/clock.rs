//! Reading the machine's clock.
//!
//! The shape of a moment is model and lives in `core`; getting one is not, and
//! needs libc. This crate already links it for the pipe transport, so the OS
//! half lives here rather than pulling a date library into a workspace that has
//! none.

use rchtmltopdf_core::Clock;

/// Local time, through libc rather than a dependency.
///
/// `localtime_r` is what knows about the machine's zone and its daylight
/// saving rules, and this crate already links libc for the pipe transport.
/// A failure leaves every field at zero, which prints as an obviously wrong
/// date rather than as a silently plausible one.
pub fn now() -> Clock {
    let Ok(since_epoch) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return Clock::default();
    };
    let seconds = since_epoch.as_secs() as libc::time_t;

    // SAFETY: `localtime_r` writes into the caller's `tm`, which is the
    // whole reason to prefer it over `localtime`. The pointer it returns is
    // either that same `tm` or null.
    let mut broken_down: libc::tm = unsafe { std::mem::zeroed() };
    let filled = unsafe { libc::localtime_r(&seconds, &mut broken_down) };
    if filled.is_null() {
        return Clock::default();
    }

    Clock {
        year: broken_down.tm_year + 1900,
        month: (broken_down.tm_mon + 1) as u32,
        day: broken_down.tm_mday as u32,
        hour: broken_down.tm_hour as u32,
        minute: broken_down.tm_min as u32,
        second: broken_down.tm_sec as u32,
        utc_offset_seconds: broken_down.tm_gmtoff as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not what the time is — that is the machine's business — but that a time
    /// was read at all. Every field zero means `localtime_r` refused, and a
    /// document dated year zero is better than one dated plausibly wrong.
    #[test]
    fn the_machine_has_a_clock_and_it_is_this_century() {
        let now = now();
        assert!(now.year >= 2024, "{now:?}");
        assert!((1..=12).contains(&now.month), "{now:?}");
        assert!((1..=31).contains(&now.day), "{now:?}");
        assert!(
            now.hour < 24 && now.minute < 60 && now.second < 61,
            "{now:?}"
        );
    }
}
