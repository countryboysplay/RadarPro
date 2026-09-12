//! S3 object key parsing/filtering and UTC calendar-date handling for
//! discovery.
//!
//! Object keys look like
//! `{year}/{month:02}/{day:02}/{ICAO}/{ICAO}{yyyyMMdd}_{HHmmss}_V0{2-7}`.
//! [`parse_object_key`] is a pure, non-panicking function that recognizes
//! exactly that filename shape and rejects everything else (including the
//! `_MDM` supplemental-metadata companion objects and any other
//! unrecognized suffix), per the discovery contract: keys are untrusted,
//! remote-sourced strings, and a key that doesn't match is skipped, not an
//! error.

use radar_types::Timestamp;

/// One discovered Level II volume-scan object in the bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredVolume {
    /// Four-letter ICAO site identifier, from the object key itself (not
    /// looked up/cross-checked against the site directory here).
    pub icao: String,
    /// The full S3 object key, e.g.
    /// `"2026/09/12/KTLX/KTLX20260912_000110_V06"`.
    pub key: String,
    /// Archive II format version number parsed from the key's `_V0N` suffix
    /// (2-7).
    pub format_version: u8,
    /// Volume start time (UTC), parsed from the key's
    /// `{yyyyMMdd}_{HHmmss}` timestamp component.
    pub start_time: Timestamp,
}

impl DiscoveredVolume {
    /// The key's final path segment (the bare filename), e.g.
    /// `"KTLX20260912_000110_V06"`.
    pub fn file_name(&self) -> &str {
        self.key.rsplit('/').next().unwrap_or(&self.key)
    }
}

// Ord/PartialOrd by (start_time, key): chronological order first, with the
// full key as a tiebreaker for a total, stable order even in the
// (implausible, but not impossible for untrusted input) case of duplicate
// timestamps.
impl PartialOrd for DiscoveredVolume {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DiscoveredVolume {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.start_time
            .cmp(&other.start_time)
            .then_with(|| self.key.cmp(&other.key))
    }
}

/// Parse one S3 object key into a [`DiscoveredVolume`], or return `None` if
/// it is not a recognized Level II volume-scan object (a `_MDM`
/// supplemental-metadata companion, an unsupported/garbled format-version
/// suffix, or anything else that doesn't match the expected shape exactly).
///
/// Never panics on malformed/adversarial input: every byte access here is
/// bounds-checked via slicing on a pre-validated total length, and every
/// numeric field is validated to be all-ASCII-digit before parsing and
/// range-checked afterward.
pub fn parse_object_key(key: &str) -> Option<DiscoveredVolume> {
    let file_name = key.rsplit('/').next().unwrap_or(key);

    // Exact expected length: ICAO(4) + yyyyMMdd(8) + '_'(1) + HHmmss(6) +
    // '_'(1) + "V0N"(3) = 23. Anything shorter or longer (including a
    // `_MDM` suffix, which adds 4 more bytes) is rejected outright, before
    // any indexing.
    const EXPECTED_LEN: usize = 23;
    if file_name.len() != EXPECTED_LEN || !file_name.is_ascii() {
        return None;
    }

    let bytes = file_name.as_bytes();
    let icao = &file_name[0..4];
    let date_digits = &file_name[4..12];
    let sep1 = bytes[12];
    let time_digits = &file_name[13..19];
    let sep2 = bytes[19];
    let version_tag = &file_name[20..23];

    if !icao.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    if sep1 != b'_' || sep2 != b'_' {
        return None;
    }
    if !all_ascii_digits(date_digits) || !all_ascii_digits(time_digits) {
        return None;
    }
    if version_tag.as_bytes()[0] != b'V' || version_tag.as_bytes()[1] != b'0' {
        return None;
    }
    let version_digit = version_tag.as_bytes()[2];
    if !(b'2'..=b'7').contains(&version_digit) {
        return None;
    }
    let format_version = version_digit - b'0';

    let year: i64 = date_digits[0..4].parse().ok()?;
    let month: u32 = date_digits[4..6].parse().ok()?;
    let day: u32 = date_digits[6..8].parse().ok()?;
    let hour: u32 = time_digits[0..2].parse().ok()?;
    let minute: u32 = time_digits[2..4].parse().ok()?;
    let second: u32 = time_digits[4..6].parse().ok()?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        // `second > 60` (not 59) tolerates a leap second, per the same
        // spirit as the rest of this codebase treating time fields
        // permissively where the ICD allows it; NEXRAD does not encode
        // leap seconds directly, but this costs nothing and avoids
        // rejecting a technically-out-of-range-by-one value outright.
        return None;
    }

    let start_time = Timestamp::from_epoch_millis(
        days_from_civil(year, month, day)
            .checked_mul(86_400_000)?
            .checked_add(i64::from(hour) * 3_600_000)?
            .checked_add(i64::from(minute) * 60_000)?
            .checked_add(i64::from(second) * 1_000)?,
    );

    Some(DiscoveredVolume {
        icao: icao.to_string(),
        key: key.to_string(),
        format_version,
        start_time,
    })
}

fn all_ascii_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Convert a proleptic-Gregorian civil (year, month, day) date into a day
/// count since the Unix epoch (1970-01-01 = day 0).
///
/// This is Howard Hinnant's `days_from_civil` algorithm
/// (<http://howardhinnant.github.io/date_algorithms.html>), the public-domain
/// inverse of the `civil_from_days` function `radar_types::Timestamp`
/// already uses for the opposite conversion. `radar_types` does not expose
/// `civil_from_days`'s inverse (it only ever needs date -> civil, not
/// civil -> date), so it is reimplemented here rather than reused, using
/// the same well-tested public-domain source rather than inventing new
/// calendar arithmetic.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * i64::from(if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// A UTC calendar date (no time-of-day), used to select which day's Level
/// II objects to discover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheDate {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

impl CacheDate {
    pub const fn new(year: i64, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    /// Today's date in UTC, per the system clock.
    ///
    /// Reuses `radar_types::Timestamp::to_civil_utc` (rather than adding a
    /// calendar/date-time dependency) to break the current wall-clock time
    /// down into civil date components.
    pub fn today_utc() -> Self {
        let now_millis = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(0);
        let civil = Timestamp::from_epoch_millis(now_millis).to_civil_utc();
        Self {
            year: civil.year,
            month: civil.month,
            day: civil.day,
        }
    }

    /// The S3 key prefix for this date/site, e.g. `"2026/09/12/KTLX/"`.
    pub fn key_prefix(&self, icao: &str) -> String {
        format!(
            "{:04}/{:02}/{:02}/{}/",
            self.year, self.month, self.day, icao
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_valid_volume_key() {
        let v = parse_object_key("2026/09/12/KTLX/KTLX20260912_000110_V06").unwrap();
        assert_eq!(v.icao, "KTLX");
        assert_eq!(v.format_version, 6);
        assert_eq!(v.file_name(), "KTLX20260912_000110_V06");
        let civil = v.start_time.to_civil_utc();
        assert_eq!(civil.year, 2026);
        assert_eq!(civil.month, 9);
        assert_eq!(civil.day, 12);
        assert_eq!(civil.hour, 0);
        assert_eq!(civil.minute, 1);
        assert_eq!(civil.second, 10);
    }

    #[test]
    fn rejects_mdm_companion_objects() {
        assert!(parse_object_key("2026/09/12/KTLX/KTLX20260912_000110_V06_MDM").is_none());
    }

    #[test]
    fn rejects_unsupported_format_version() {
        assert!(parse_object_key("2026/09/12/KTLX/KTLX20260912_000110_V01").is_none());
        assert!(parse_object_key("2026/09/12/KTLX/KTLX20260912_000110_V08").is_none());
        assert!(parse_object_key("2026/09/12/KTLX/KTLX20260912_000110_V19").is_none());
    }

    #[test]
    fn accepts_full_supported_version_range() {
        for v in b'2'..=b'7' {
            let key = format!("2026/09/12/KTLX/KTLX20260912_000110_V0{}", v as char);
            assert!(
                parse_object_key(&key).is_some(),
                "V0{} should be accepted",
                v as char
            );
        }
    }

    #[test]
    fn rejects_garbage_and_truncated_keys() {
        assert!(parse_object_key("").is_none());
        assert!(parse_object_key("not-a-key-at-all").is_none());
        assert!(parse_object_key("2026/09/12/KTLX/KTLX20260912_000110").is_none());
        assert!(parse_object_key("2026/09/12/KTLX/KTLX2026091_000110_V06").is_none());
        assert!(parse_object_key("2026/09/12/KTLX/ktlx20260912_000110_v06").is_none());
        assert!(parse_object_key("2026/09/12/KTLX/KTLX2026091X_000110_V06").is_none());
    }

    #[test]
    fn rejects_out_of_range_calendar_fields_without_panicking() {
        assert!(parse_object_key("2026/09/12/KTLX/KTLX20269912_999999_V06").is_none());
        assert!(parse_object_key("2026/09/12/KTLX/KTLX20260000_000110_V06").is_none());
    }

    #[test]
    fn key_prefix_is_zero_padded() {
        let date = CacheDate::new(2026, 1, 5);
        assert_eq!(date.key_prefix("KTLX"), "2026/01/05/KTLX/");
    }

    #[test]
    fn today_utc_is_plausible() {
        // Loose sanity check: today's UTC year should not have regressed to
        // the epoch or run away to something absurd.
        let today = CacheDate::today_utc();
        assert!(today.year >= 2024 && today.year < 3000);
        assert!((1..=12).contains(&today.month));
        assert!((1..=31).contains(&today.day));
    }

    #[test]
    fn sorts_chronologically() {
        let a = parse_object_key("2026/09/12/KTLX/KTLX20260912_000110_V06").unwrap();
        let b = parse_object_key("2026/09/12/KTLX/KTLX20260912_000440_V06").unwrap();
        let mut v = vec![b.clone(), a.clone()];
        v.sort();
        assert_eq!(v, vec![a, b]);
    }
}
