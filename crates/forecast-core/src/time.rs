//! A small, dependency-free UTC timestamp shared by every forecast provider.
//!
//! Originally written for `provider-gefs` alone (S07), which documented it
//! as "a shared timestamp utility crate can be extracted later if a third
//! caller ever needs one" -- S08's `provider-hrrr` is that third caller
//! (`forecast-core` itself is the second), so this is moved here verbatim
//! rather than duplicated again. `radar-types::Timestamp` does something
//! similar for NEXRAD but is documented as part of that crate's polar radar
//! domain model, and GLOBAL_CONTRACT's "provider-specific names and formats
//! stop at provider boundaries" argues for keeping the forecast domain's own
//! timestamp type separate from the polar radar one regardless.
//!
//! Every real GRIB2 time field this crate's providers decode (GEFS's
//! `Section1::ref_time_unchecked` reference/run time, `ProdDefinition::
//! forecast_time` lead time; HRRR's identical shape) is already whole
//! calendar fields (year/month/day/hour/minute/second) plus a whole-hour
//! lead time -- there is never a sub-second or fractional-day quantity to
//! represent, so this type stores civil UTC fields directly rather than
//! round-tripping through epoch milliseconds the way NEXRAD's millisecond-
//! of-day encoding needs to.

use std::fmt;

/// A UTC point in time, as whole calendar fields. No time zone is ever
/// implied or applied -- every value in this crate is UTC, per
/// GLOBAL_CONTRACT.md ("Internal time is UTC").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTimestamp {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl UtcTimestamp {
    pub const fn new(year: i64, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    /// Add a whole number of hours, correctly rolling over day/month/year
    /// boundaries -- used to compute a forecast's valid time (run/init
    /// time plus forecast lead) per GLOBAL_CONTRACT's "forecasts keep
    /// initialization time, lead time, and valid time separately" (all
    /// three are kept; this is how `valid_time` is *derived*, not a
    /// replacement for storing the other two).
    pub fn plus_hours(&self, hours: i64) -> Self {
        let total_seconds_of_day = i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
            + hours * 3600;
        let day_offset = total_seconds_of_day.div_euclid(86_400);
        let seconds_of_day = total_seconds_of_day.rem_euclid(86_400);

        let days_since_epoch = days_from_civil(self.year, self.month, self.day) + day_offset;
        let (year, month, day) = civil_from_days(days_since_epoch);

        Self {
            year,
            month,
            day,
            hour: (seconds_of_day / 3600) as u32,
            minute: ((seconds_of_day / 60) % 60) as u32,
            second: (seconds_of_day % 60) as u32,
        }
    }

    /// ISO-8601-ish UTC string, e.g. `"2026-09-12T12:00:00Z"`.
    pub fn to_iso8601(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Today's UTC calendar date (year, month, day), from the system clock.
/// Used only by a provider's own "find the most recently published run"
/// discovery logic -- never used to compute or validate a decoded field's
/// own metadata, which always comes from the decoded message itself.
pub fn today_utc_date() -> (u16, u8, u8) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let days_since_epoch = (now.as_secs() / 86_400) as i64;
    let (year, month, day) = civil_from_days(days_since_epoch);
    (year as u16, month as u8, day as u8)
}

/// Subtract `days` whole days from a (year, month, day) civil date --
/// used alongside [`today_utc_date`] to walk backward over recent UTC
/// calendar days.
pub fn civil_date_minus_days(year: u16, month: u8, day: u8, days: u32) -> (u16, u8, u8) {
    let base = days_from_civil(i64::from(year), u32::from(month), u32::from(day));
    let (y, m, d) = civil_from_days(base - i64::from(days));
    (y as u16, m as u8, d as u8)
}

impl fmt::Display for UtcTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_iso8601())
    }
}

/// Howard Hinnant's `days_from_civil`/`civil_from_days` algorithms
/// (<http://howardhinnant.github.io/date_algorithms.html>), public-domain,
/// well-tested proleptic-Gregorian calendar arithmetic.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 }); // [0, 11]
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plus_hours_within_same_day() {
        let t = UtcTimestamp::new(2026, 9, 12, 12, 0, 0);
        assert_eq!(t.plus_hours(3), UtcTimestamp::new(2026, 9, 12, 15, 0, 0));
    }

    #[test]
    fn plus_hours_rolls_over_midnight() {
        let t = UtcTimestamp::new(2026, 9, 12, 18, 0, 0);
        assert_eq!(t.plus_hours(12), UtcTimestamp::new(2026, 9, 13, 6, 0, 0));
    }

    #[test]
    fn plus_hours_rolls_over_month_boundary() {
        let t = UtcTimestamp::new(2026, 9, 30, 18, 0, 0);
        assert_eq!(t.plus_hours(12), UtcTimestamp::new(2026, 10, 1, 6, 0, 0));
    }

    #[test]
    fn plus_hours_rolls_over_year_boundary() {
        let t = UtcTimestamp::new(2026, 12, 31, 18, 0, 0);
        assert_eq!(t.plus_hours(12), UtcTimestamp::new(2027, 1, 1, 6, 0, 0));
    }

    #[test]
    fn plus_hours_handles_a_real_gefs_extended_lead_time() {
        // A real forecast hour seen in the live GEFS bucket (f240 = 10 days).
        let run = UtcTimestamp::new(2026, 9, 12, 12, 0, 0);
        let valid = run.plus_hours(240);
        assert_eq!(valid, UtcTimestamp::new(2026, 9, 22, 12, 0, 0));
    }

    #[test]
    fn plus_zero_hours_is_identity() {
        let t = UtcTimestamp::new(2026, 2, 28, 23, 59, 59);
        assert_eq!(t.plus_hours(0), t);
    }

    #[test]
    fn civil_date_minus_days_rolls_back_across_month_and_year_boundaries() {
        assert_eq!(civil_date_minus_days(2026, 9, 1, 1), (2026, 8, 31));
        assert_eq!(civil_date_minus_days(2026, 1, 1, 1), (2025, 12, 31));
        assert_eq!(civil_date_minus_days(2026, 9, 13, 1), (2026, 9, 12));
    }

    #[test]
    fn today_utc_date_returns_a_plausible_date() {
        // Not a precise assertion (depends on the system clock) -- just
        // confirms this returns *something* sane rather than a garbage
        // epoch-adjacent default, catching a gross overflow/logic bug.
        let (year, month, day) = today_utc_date();
        assert!((2020..2100).contains(&year));
        assert!((1..=12).contains(&month));
        assert!((1..=31).contains(&day));
    }

    #[test]
    fn to_iso8601_formats_with_zero_padding() {
        let t = UtcTimestamp::new(2026, 1, 2, 3, 4, 5);
        assert_eq!(t.to_iso8601(), "2026-01-02T03:04:05Z");
    }
}
