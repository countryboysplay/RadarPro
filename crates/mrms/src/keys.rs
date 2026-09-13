//! MRMS S3 object key construction and parsing -- pure, no-network
//! functions mapping a (product, snapshot timestamp) pair to the object
//! keys `noaa-mrms-pds` actually uses, and back.
//!
//! # Verified empirically (2026-09-13, live bucket)
//!
//! `<REGION>/<Product>_<height>/<YYYYMMDD>/MRMS_<Product>_<height>_<YYYYMMDD>-<HHMMSS>.grib2.gz`,
//! e.g.
//! `CONUS/MergedReflectivityQCComposite_00.50/20260913/MRMS_MergedReflectivityQCComposite_00.50_20260913-050438.grib2.gz`.
//! Anonymous, unsigned S3 access -- same trust model as `provider-gefs`/
//! `provider-hrrr`.
//!
//! Unlike GEFS/HRRR (which publish discrete runs at fixed hours, with a
//! predictable `.idx`-listed set of forecast-hour files), MRMS publishes a
//! plain, unbroken stream of ~2-minute snapshots with **no run/lead-time/
//! ensemble concept at all** -- confirmed empirically: consecutive real
//! object timestamps for `MergedReflectivityQCComposite_00.50` on
//! 2026-09-13 were `00:00:42`, `00:02:42`, `00:04:41`, ... -- close to but
//! not exactly on a fixed 2-minute-past-the-hour grid (the seconds field
//! drifts by up to a couple of seconds run to run). This means a snapshot's
//! exact key **cannot be reliably constructed from a guessed timestamp**
//! the way HRRR's `object_exists`-per-candidate-hour check can -- see
//! `crate::client::MrmsClient::discover_latest_snapshot`'s module doc for
//! how this crate's discovery loop adapts to that reality instead of
//! guessing exact seconds.
//!
//! # Discipline-209 category/parameter numbers are center-local
//!
//! MRMS GRIB2 messages declare discipline byte `209` (WMO Table 0.0
//! "local use" range), not a standard meteorological discipline --
//! confirmed empirically (see `crate::decode`'s module doc). This module
//! never interprets the decoded category/parameter numbers as if they were
//! standard WMO Table 4.2 codes; a snapshot's physical identity comes
//! entirely from which [`MrmsProduct`]/key was requested, matching
//! `DATA_SOURCES.md`'s own explicit guidance.

use std::fmt;

/// The current public NOAA MRMS bucket on AWS S3 (anonymous, unsigned
/// `ListObjectsV2`/`GetObject` -- confirmed live, no credentials, same
/// trust model as `noaa-gefs-pds`/`noaa-hrrr-bdp-pds`).
pub const MRMS_BUCKET_URL: &str = "https://noaa-mrms-pds.s3.amazonaws.com";

/// The two national gridded MRMS products this crate implements (S09 Phase
/// 1's "national mosaic and one precipitation-oriented product"). Both are
/// plain periodic observation snapshots -- see this module's doc comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MrmsProduct {
    /// `CONUS/MergedReflectivityQCComposite_00.50/` -- national column-max
    /// composite reflectivity, dBZ. The same physical quantity NEXRAD REF
    /// already has a color table for.
    ReflectivityQcComposite,
    /// `CONUS/PrecipRate_00.00/` -- instantaneous precipitation rate, mm/hr.
    PrecipRate,
}

impl MrmsProduct {
    /// The `<Product>_<height>` component of this product's key path and
    /// filename, e.g. `"MergedReflectivityQCComposite_00.50"` -- verified
    /// empirically against the real live bucket.
    pub const fn product_dir(&self) -> &'static str {
        match self {
            MrmsProduct::ReflectivityQcComposite => "MergedReflectivityQCComposite_00.50",
            MrmsProduct::PrecipRate => "PrecipRate_00.00",
        }
    }

    /// Native physical unit every decoded value (before missing/no-coverage
    /// classification) is expressed in.
    pub const fn native_unit(&self) -> &'static str {
        match self {
            MrmsProduct::ReflectivityQcComposite => "dBZ",
            MrmsProduct::PrecipRate => "mm/hr",
        }
    }

    /// Short, human-readable display name, for logging/harness output.
    pub const fn display_name(&self) -> &'static str {
        match self {
            MrmsProduct::ReflectivityQcComposite => "MRMS national reflectivity mosaic",
            MrmsProduct::PrecipRate => "MRMS precipitation rate",
        }
    }
}

impl fmt::Display for MrmsProduct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.product_dir())
    }
}

/// A specific MRMS snapshot: one product at one real-world instant, exactly
/// as published (year/month/day/hour/minute/second all from the object
/// key's own `YYYYMMDD-HHMMSS` filename component). This is deliberately
/// *not* a `forecast_core::grid::ForecastGrid`-style run/lead-time pair --
/// MRMS has no such concept (see this module's doc comment); a snapshot is
/// identified by exactly one real-world timestamp, full stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapshotReference {
    pub product: MrmsProduct,
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl SnapshotReference {
    /// The `<YYYYMMDD>` date component of this snapshot's key.
    pub fn date_component(&self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    /// The `<HHMMSS>` time component of this snapshot's key.
    pub fn time_component(&self) -> String {
        format!("{:02}{:02}{:02}", self.hour, self.minute, self.second)
    }

    /// The `CONUS/<product_dir>/<YYYYMMDD>/` prefix every key for this
    /// snapshot's calendar day falls under -- what
    /// [`crate::client::MrmsClient::list_objects`] lists to discover every
    /// snapshot published that day.
    pub fn day_prefix(product: MrmsProduct, year: u16, month: u8, day: u8) -> String {
        format!(
            "CONUS/{}/{:04}{:02}{:02}/",
            product.product_dir(),
            year,
            month,
            day
        )
    }

    /// The full object key for this exact snapshot, e.g.
    /// `"CONUS/PrecipRate_00.00/20260913/MRMS_PrecipRate_00.00_20260913-050400.grib2.gz"`.
    pub fn object_key(&self) -> String {
        format!(
            "CONUS/{}/{}/MRMS_{}_{}-{}.grib2.gz",
            self.product.product_dir(),
            self.date_component(),
            self.product.product_dir(),
            self.date_component(),
            self.time_component(),
        )
    }
}

impl fmt::Display for SnapshotReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.product, self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// Parse an object key (or bare file name) back into a [`SnapshotReference`]
/// for `product`, e.g.
/// `"CONUS/PrecipRate_00.00/20260913/MRMS_PrecipRate_00.00_20260913-050400.grib2.gz"`
/// -> `2026-09-13T05:04:00Z`. Returns `None` (never panics) for anything
/// that does not match this product's exact expected filename shape --
/// `crate::client::MrmsClient::discover_latest_snapshot`'s listing may
/// contain unrelated keys under the same prefix (there should not be any,
/// but this crate never assumes a remote listing is exactly what it
/// expects) and simply skips them rather than erroring the whole listing.
pub fn parse_snapshot_key(product: MrmsProduct, key: &str) -> Option<SnapshotReference> {
    let file_name = key.rsplit('/').next()?;
    let prefix = format!("MRMS_{}_", product.product_dir());
    let rest = file_name.strip_prefix(&prefix)?;
    let rest = rest.strip_suffix(".grib2.gz")?;
    let (date_part, time_part) = rest.split_once('-')?;
    if date_part.len() != 8 || time_part.len() != 6 {
        return None;
    }
    let year: u16 = date_part[0..4].parse().ok()?;
    let month: u8 = date_part[4..6].parse().ok()?;
    let day: u8 = date_part[6..8].parse().ok()?;
    let hour: u8 = time_part[0..2].parse().ok()?;
    let minute: u8 = time_part[2..4].parse().ok()?;
    let second: u8 = time_part[4..6].parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    Some(SnapshotReference {
        product,
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

/// Join a bucket base URL and an object key into a full URL.
pub fn object_url(bucket_url: &str, key: &str) -> String {
    format!("{}/{}", bucket_url.trim_end_matches('/'), key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_key_matches_the_empirically_verified_real_layout() {
        let snapshot = SnapshotReference {
            product: MrmsProduct::ReflectivityQcComposite,
            year: 2026,
            month: 9,
            day: 13,
            hour: 5,
            minute: 4,
            second: 38,
        };
        assert_eq!(
            snapshot.object_key(),
            "CONUS/MergedReflectivityQCComposite_00.50/20260913/\
             MRMS_MergedReflectivityQCComposite_00.50_20260913-050438.grib2.gz"
        );
    }

    #[test]
    fn precip_rate_object_key_matches_the_real_layout() {
        let snapshot = SnapshotReference {
            product: MrmsProduct::PrecipRate,
            year: 2026,
            month: 9,
            day: 13,
            hour: 5,
            minute: 4,
            second: 0,
        };
        assert_eq!(
            snapshot.object_key(),
            "CONUS/PrecipRate_00.00/20260913/MRMS_PrecipRate_00.00_20260913-050400.grib2.gz"
        );
    }

    #[test]
    fn day_prefix_matches_the_real_layout() {
        assert_eq!(
            SnapshotReference::day_prefix(MrmsProduct::PrecipRate, 2026, 9, 13),
            "CONUS/PrecipRate_00.00/20260913/"
        );
    }

    #[test]
    fn parse_snapshot_key_round_trips_object_key() {
        let snapshot = SnapshotReference {
            product: MrmsProduct::ReflectivityQcComposite,
            year: 2026,
            month: 9,
            day: 13,
            hour: 5,
            minute: 4,
            second: 38,
        };
        let key = snapshot.object_key();
        let parsed = parse_snapshot_key(MrmsProduct::ReflectivityQcComposite, &key).unwrap();
        assert_eq!(parsed, snapshot);
    }

    #[test]
    fn parse_snapshot_key_rejects_wrong_product() {
        let key = "CONUS/PrecipRate_00.00/20260913/MRMS_PrecipRate_00.00_20260913-050400.grib2.gz";
        assert!(parse_snapshot_key(MrmsProduct::ReflectivityQcComposite, key).is_none());
    }

    #[test]
    fn parse_snapshot_key_rejects_garbage() {
        assert!(parse_snapshot_key(MrmsProduct::PrecipRate, "not a key").is_none());
        assert!(parse_snapshot_key(MrmsProduct::PrecipRate, "").is_none());
        assert!(parse_snapshot_key(
            MrmsProduct::PrecipRate,
            "CONUS/PrecipRate_00.00/20260913/MRMS_PrecipRate_00.00_2026091x-050400.grib2.gz"
        )
        .is_none());
    }

    #[test]
    fn object_url_joins_cleanly_regardless_of_trailing_slash() {
        assert_eq!(
            object_url("https://example.com", "a/b"),
            "https://example.com/a/b"
        );
        assert_eq!(
            object_url("https://example.com/", "a/b"),
            "https://example.com/a/b"
        );
    }
}
