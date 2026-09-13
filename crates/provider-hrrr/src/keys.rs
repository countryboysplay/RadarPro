//! HRRR S3 object key construction -- pure, no-network functions mapping a
//! (run, forecast hour) pair to the object keys `noaa-hrrr-bdp-pds` actually
//! uses.
//!
//! Verified empirically (2026-09-12, live bucket) against the 2026-09-12
//! 12Z run's `conus` surface product:
//! `hrrr.<YYYYMMDD>/conus/hrrr.t<HH>z.wrfsfcf<FF>.grib2` (plus a `.idx`
//! sidecar at the same key with `.idx` appended), `<HH>` any of `00..23`
//! (HRRR runs hourly, unlike GEFS's four daily runs) and `<FF>` a
//! zero-padded **2**-digit forecast hour (`f00`, `f01`, ... -- not GEFS's
//! 3-digit `f000`).
//!
//! # Corrections to this stage's starting brief
//! The brief's suggested path shape guessed `wrfsfcf<FF>.grib2` directly
//! under `conus/` with no other qualifier -- confirmed correct. It also
//! guessed the `.idx` sidecar shares GEFS's `N:offset:d=...:VAR:LEVEL:...`
//! shape -- confirmed correct (`crate::idx`), except HRRR's 2m-temperature
//! line has no trailing `ENS=...` annotation at all (HRRR is deterministic,
//! confirmed live: `71:34722112:d=2026091212:TMP:2 m above ground:anl:`),
//! and the `VAR:LEVEL` naming for 2m temperature is spelled **identically**
//! to GEFS's (`TMP` / `2 m above ground`), not differently as the brief
//! flagged as a possibility to verify.

use std::fmt;

/// The current public NOAA HRRR bucket on AWS S3 (anonymous, unsigned
/// `ListObjectsV2`/`GetObject`, same trust model as `noaa-gefs-pds` and
/// `unidata-nexrad-level2` -- confirmed live, no credentials).
pub const HRRR_BUCKET_URL: &str = "https://noaa-hrrr-bdp-pds.s3.amazonaws.com";

/// The `conus` surface ("wrfsfc") product this crate implements -- HRRR
/// also publishes `nat` (native model levels), `prs` (pressure levels),
/// `subh` (sub-hourly), and an `alaska` domain, all out of scope for this
/// stage (2m temperature, matching GEFS, for an apples-to-apples
/// abstraction proof -- see the S08 stage file).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Product(&'static str);

impl Product {
    pub const CONUS_SURFACE: Product = Product("wrfsfc");

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// A specific HRRR model run: a UTC calendar date plus one of the 24
/// hourly run hours (HRRR runs every hour, unlike GEFS's four daily runs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunReference {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    /// UTC run hour, `0..=23`.
    pub run_hour: u8,
}

impl RunReference {
    pub const fn new(year: u16, month: u8, day: u8, run_hour: u8) -> Self {
        Self {
            year,
            month,
            day,
            run_hour,
        }
    }

    /// The `hrrr.<YYYYMMDD>` date component of every key under this run.
    pub fn date_component(&self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    /// The `hrrr.<YYYYMMDD>/conus/` prefix common to every object under
    /// this run's CONUS domain.
    pub fn run_prefix(&self) -> String {
        format!("hrrr.{}/conus/", self.date_component())
    }
}

impl fmt::Display for RunReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}Z {}", self.run_hour, self.date_component())
    }
}

/// A forecast lead time, in whole hours. Zero-padded to **2** digits in
/// HRRR's own file naming (`f00`, `f01`, ..., unlike GEFS's 3-digit
/// `f000`) -- confirmed empirically against the live bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForecastHour(pub u16);

impl ForecastHour {
    pub fn file_suffix(&self) -> String {
        format!("f{:02}", self.0)
    }
}

/// The GRIB2 object key (no bucket URL, no leading slash) for one
/// (run, product, forecast hour) tuple, e.g.
/// `"hrrr.20260912/conus/hrrr.t12z.wrfsfcf00.grib2"`.
pub fn object_key(run: RunReference, product: Product, forecast_hour: ForecastHour) -> String {
    format!(
        "{}hrrr.t{:02}z.{}{}.grib2",
        run.run_prefix(),
        run.run_hour,
        product.as_str(),
        forecast_hour.file_suffix()
    )
}

/// The `.idx` sidecar key for the same tuple.
pub fn idx_key(run: RunReference, product: Product, forecast_hour: ForecastHour) -> String {
    format!("{}.idx", object_key(run, product, forecast_hour))
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
        let run = RunReference::new(2026, 9, 12, 12);
        let key = object_key(run, Product::CONUS_SURFACE, ForecastHour(0));
        assert_eq!(key, "hrrr.20260912/conus/hrrr.t12z.wrfsfcf00.grib2");
    }

    #[test]
    fn idx_key_appends_dot_idx() {
        let run = RunReference::new(2026, 9, 12, 12);
        let key = idx_key(run, Product::CONUS_SURFACE, ForecastHour(3));
        assert_eq!(key, "hrrr.20260912/conus/hrrr.t12z.wrfsfcf03.grib2.idx");
    }

    #[test]
    fn forecast_hour_is_zero_padded_to_two_digits_not_three() {
        assert_eq!(ForecastHour(0).file_suffix(), "f00");
        assert_eq!(ForecastHour(9).file_suffix(), "f09");
        assert_eq!(ForecastHour(18).file_suffix(), "f18");
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

    #[test]
    fn run_reference_displays_run_hour_and_date() {
        let run = RunReference::new(2026, 9, 12, 12);
        assert_eq!(run.to_string(), "12Z 20260912");
    }
}
