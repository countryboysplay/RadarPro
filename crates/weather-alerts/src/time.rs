//! RFC 3339 timestamp parsing for CAP's `sent`/`effective`/`onset`/
//! `expires`/`ends` fields.
//!
//! `radar_types::Timestamp` (reused here as this crate's UTC point-in-time
//! type -- see the crate root docs for why) intentionally carries no
//! calendar/string parsing of its own: it exists to hold a NEXRAD
//! "modified Julian date + milliseconds of day" pair, which is unrelated
//! wire encoding. The NWS alerts feed instead gives every timestamp as an
//! RFC 3339 string with an explicit, already-resolved numeric UTC offset
//! (e.g. `"2026-09-12T18:09:00-05:00"`, `"2026-09-12T23:13:09+00:00"`) --
//! confirmed against a real `https://api.weather.gov/alerts/active`
//! response fetched during development. Because the offset is always a
//! literal `+HH:MM`/`-HH:MM`/`Z` in the string itself, converting to a UTC
//! epoch-millisecond count needs no timezone database (no DST rules, no
//! named zones) -- just calendar arithmetic, so this module hand-rolls a
//! small parser rather than pulling in a full date/time crate (e.g.
//! `chrono`), matching `radar_types::Timestamp`'s own documented reasoning
//! for avoiding that dependency.
//!
//! [`days_from_civil`] is the inverse of `radar_types`'s private
//! `civil_from_days` (Howard Hinnant's algorithm,
//! <http://howardhinnant.github.io/date_algorithms.html>, public domain);
//! it is reimplemented here (rather than exposed from `radar_types`, which
//! this task does not modify) since converting a *civil calendar date back
//! to* a day count is exactly the inverse operation `radar_types` does not
//! currently need for its own (NEXRAD-native) use case.

use radar_types::Timestamp;
use thiserror::Error;

/// Errors parsing an RFC 3339 timestamp string.
///
/// Every variant names the byte-level expectation that failed; malformed
/// timestamps are untrusted external input (CAP feed content) and must
/// never panic this crate.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TimeParseError {
    #[error("timestamp {value:?} is too short to be a valid RFC 3339 timestamp")]
    TooShort { value: String },

    #[error(
        "timestamp {value:?} has an unexpected character {expected:?} at byte offset {offset}"
    )]
    UnexpectedChar {
        value: String,
        offset: usize,
        expected: char,
    },

    #[error("timestamp {value:?} has a non-digit character in its {field} field")]
    NonDigit { value: String, field: &'static str },

    #[error("timestamp {value:?} has an out-of-range {field} value {found}")]
    OutOfRange {
        value: String,
        field: &'static str,
        found: i64,
    },

    #[error("timestamp {value:?} has an unrecognized UTC offset suffix")]
    InvalidOffset { value: String },
}

/// Parse an RFC 3339 timestamp (as used throughout the NWS CAP/GeoJSON
/// feed) into a UTC [`Timestamp`].
///
/// Accepts a literal `Z`/`z` suffix or a numeric `+HH:MM`/`-HH:MM` offset,
/// and an optional fractional-seconds component (`.` followed by one or
/// more digits; only the first three significant digits are kept, i.e.
/// truncated to milliseconds -- CAP timestamps observed in practice never
/// carry a fractional-seconds component at all, but the CAP/RFC 3339 grammar
/// permits one).
pub fn parse_rfc3339(value: &str) -> Result<Timestamp, TimeParseError> {
    let bytes = value.as_bytes();
    // Minimum: "YYYY-MM-DDTHH:MM:SSZ" (20 bytes).
    if bytes.len() < 20 {
        return Err(TimeParseError::TooShort {
            value: value.to_string(),
        });
    }

    let digits = |start: usize, len: usize, field: &'static str| -> Result<i64, TimeParseError> {
        let slice = &bytes[start..start + len];
        if !slice.iter().all(u8::is_ascii_digit) {
            return Err(TimeParseError::NonDigit {
                value: value.to_string(),
                field,
            });
        }
        // Safe: just validated every byte is an ASCII digit.
        let s = std::str::from_utf8(slice).expect("ASCII digits are valid UTF-8");
        Ok(s.parse::<i64>().expect("bounded digit run parses as i64"))
    };

    let expect_char = |offset: usize, expected: char| -> Result<(), TimeParseError> {
        if bytes.get(offset).copied() != Some(expected as u8) {
            return Err(TimeParseError::UnexpectedChar {
                value: value.to_string(),
                offset,
                expected,
            });
        }
        Ok(())
    };

    let year = digits(0, 4, "year")?;
    expect_char(4, '-')?;
    let month = digits(5, 2, "month")?;
    expect_char(7, '-')?;
    let day = digits(8, 2, "day")?;
    expect_char(10, 'T')?;
    let hour = digits(11, 2, "hour")?;
    expect_char(13, ':')?;
    let minute = digits(14, 2, "minute")?;
    expect_char(16, ':')?;
    let second = digits(17, 2, "second")?;

    if !(1..=12).contains(&month) {
        return Err(TimeParseError::OutOfRange {
            value: value.to_string(),
            field: "month",
            found: month,
        });
    }
    if !(1..=31).contains(&day) {
        return Err(TimeParseError::OutOfRange {
            value: value.to_string(),
            field: "day",
            found: day,
        });
    }
    if !(0..=23).contains(&hour) {
        return Err(TimeParseError::OutOfRange {
            value: value.to_string(),
            field: "hour",
            found: hour,
        });
    }
    if !(0..=59).contains(&minute) {
        return Err(TimeParseError::OutOfRange {
            value: value.to_string(),
            field: "minute",
            found: minute,
        });
    }
    // Allow 60 for a leap second (never actually observed in this feed,
    // but rejecting it outright would be an unforced, untested assumption
    // about a real-world CAP producer's clock behavior).
    if !(0..=60).contains(&second) {
        return Err(TimeParseError::OutOfRange {
            value: value.to_string(),
            field: "second",
            found: second,
        });
    }

    let mut pos = 19;

    // Optional fractional seconds: '.' followed by 1+ digits.
    let mut millisecond: i64 = 0;
    if bytes.get(pos).copied() == Some(b'.') {
        let start = pos + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == start {
            return Err(TimeParseError::NonDigit {
                value: value.to_string(),
                field: "fractional seconds",
            });
        }
        let frac_str = std::str::from_utf8(&bytes[start..end]).expect("digits are valid UTF-8");
        // Keep only the first 3 digits (milliseconds); pad if shorter.
        let mut padded = frac_str.to_string();
        padded.truncate(3);
        while padded.len() < 3 {
            padded.push('0');
        }
        millisecond = padded.parse::<i64>().expect("3-digit run parses as i64");
        pos = end;
    }

    // Offset: 'Z'/'z', or "+HH:MM"/"-HH:MM".
    let offset_minutes: i64 = match bytes.get(pos).copied() {
        Some(b'Z') | Some(b'z') => {
            if pos + 1 != bytes.len() {
                return Err(TimeParseError::InvalidOffset {
                    value: value.to_string(),
                });
            }
            0
        }
        Some(sign @ (b'+' | b'-')) => {
            if bytes.len() != pos + 6 || bytes[pos + 3] != b':' {
                return Err(TimeParseError::InvalidOffset {
                    value: value.to_string(),
                });
            }
            let offset_hour = digits(pos + 1, 2, "offset hour")?;
            let offset_minute = digits(pos + 4, 2, "offset minute")?;
            if !(0..=23).contains(&offset_hour) || !(0..=59).contains(&offset_minute) {
                return Err(TimeParseError::InvalidOffset {
                    value: value.to_string(),
                });
            }
            let magnitude = offset_hour * 60 + offset_minute;
            if sign == b'-' {
                -magnitude
            } else {
                magnitude
            }
        }
        _ => {
            return Err(TimeParseError::InvalidOffset {
                value: value.to_string(),
            })
        }
    };

    let days = days_from_civil(year, month as u32, day as u32);
    let local_millis_of_day =
        (hour * 3_600_000) + (minute * 60_000) + (second * 1_000) + millisecond;
    let local_millis = days * 86_400_000 + local_millis_of_day;
    // The string's local clock reading is `offset` ahead of UTC, so
    // UTC = local - offset.
    let utc_millis = local_millis - offset_minutes * 60_000;

    Ok(Timestamp::from_epoch_millis(utc_millis))
}

/// Convert a proleptic-Gregorian civil (year, month, day) date into a day
/// count since the Unix epoch (1970-01-01 = day 0).
///
/// The exact inverse of `radar_types`'s private `civil_from_days`; see this
/// module's docs for why it is reimplemented here rather than exposed from
/// `radar_types`. No validation of `m`/`d` ranges is performed here --
/// callers ([`parse_rfc3339`]) validate `1..=12`/`1..=31` beforehand.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m as i64 - 3 } else { m as i64 + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ground truth independently computed via `new Date(s).getTime()`
    /// (Node.js, a correct RFC 3339 implementation) for each string.
    #[test]
    fn parses_real_nws_timestamps_matching_independent_ground_truth() {
        let cases: &[(&str, i64)] = &[
            ("2026-09-12T18:09:00-05:00", 1_789_254_540_000),
            ("2026-09-12T23:13:09+00:00", 1_789_254_789_000),
            ("1970-01-01T00:00:00Z", 0),
            ("2024-06-01T00:03:53.941Z", 1_717_200_233_941),
            ("2026-09-13T03:30:00-08:00", 1_789_299_000_000),
        ];
        for (input, expected_millis) in cases {
            let ts = parse_rfc3339(input).unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(ts.epoch_millis(), *expected_millis, "input: {input}");
        }
    }

    #[test]
    fn rejects_too_short_input_without_panicking() {
        assert!(matches!(
            parse_rfc3339("2026-09-12"),
            Err(TimeParseError::TooShort { .. })
        ));
    }

    #[test]
    fn rejects_non_digit_fields() {
        assert!(matches!(
            parse_rfc3339("202X-09-12T18:09:00Z"),
            Err(TimeParseError::NonDigit { .. })
        ));
    }

    #[test]
    fn rejects_out_of_range_month() {
        assert!(matches!(
            parse_rfc3339("2026-13-12T18:09:00Z"),
            Err(TimeParseError::OutOfRange { field: "month", .. })
        ));
    }

    #[test]
    fn rejects_missing_offset() {
        // 20 bytes (passes the minimum-length check) but ends in a
        // character that is neither 'Z' nor '+'/'-'.
        assert!(matches!(
            parse_rfc3339("2026-09-12T18:09:00X"),
            Err(TimeParseError::InvalidOffset { .. })
        ));
    }

    #[test]
    fn rejects_malformed_offset_sign_only() {
        assert!(matches!(
            parse_rfc3339("2026-09-12T18:09:00+05"),
            Err(TimeParseError::InvalidOffset { .. })
        ));
    }

    #[test]
    fn never_panics_on_arbitrary_short_garbage() {
        for s in [
            "",
            "z",
            "\u{0}",
            "----------------T::Z",
            "🎉🎉🎉🎉🎉🎉🎉🎉🎉🎉",
        ] {
            let _ = parse_rfc3339(s);
        }
    }
}
