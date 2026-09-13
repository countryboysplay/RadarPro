//! GEFS S3 object key construction -- pure, no-network functions mapping a
//! (run, member, product group, forecast hour) tuple to the object keys
//! `noaa-gefs-pds` actually uses.
//!
//! Verified empirically (2026-09-12, live bucket) against the 2026-09-12
//! 12Z run's `atmos/pgrb2sp25` product group:
//! `gefs.<YYYYMMDD>/<HH>/atmos/pgrb2sp25/<member>.t<HH>z.pgrb2s.0p25.f<FFF>`
//! (plus a `.idx` sidecar at the same key with `.idx` appended), `<HH>` one
//! of `00`/`06`/`12`/`18`, `<FFF>` a zero-padded 3-digit forecast hour.
//! Members confirmed present: `gec00` (control), `gep01..gep30` (30
//! perturbed members -- `gep30` confirmed present, `gep31` confirmed
//! **absent**, HTTP 404), `geavg` (ensemble mean).

use std::fmt;

/// The current public NOAA GEFS bucket on AWS S3 (anonymous, unsigned
/// `ListObjectsV2`/`GetObject`, same trust model as `unidata-nexrad-level2`
/// -- confirmed live, no credentials).
pub const GEFS_BUCKET_URL: &str = "https://noaa-gefs-pds.s3.amazonaws.com";

/// One of GEFS's four daily model runs, identified by UTC run hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunHour {
    H00,
    H06,
    H12,
    H18,
}

impl RunHour {
    pub const ALL: [RunHour; 4] = [RunHour::H00, RunHour::H06, RunHour::H12, RunHour::H18];

    pub const fn as_str(&self) -> &'static str {
        match self {
            RunHour::H00 => "00",
            RunHour::H06 => "06",
            RunHour::H12 => "12",
            RunHour::H18 => "18",
        }
    }

    pub const fn hour(&self) -> u8 {
        match self {
            RunHour::H00 => 0,
            RunHour::H06 => 6,
            RunHour::H12 => 12,
            RunHour::H18 => 18,
        }
    }
}

impl fmt::Display for RunHour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A specific GEFS model run: a UTC calendar date plus one of the four
/// daily run hours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RunReference {
    /// UTC calendar year, e.g. `2026`.
    pub year: u16,
    /// UTC calendar month, `1..=12`.
    pub month: u8,
    /// UTC calendar day, `1..=31`.
    pub day: u8,
    pub run_hour: RunHour,
}

impl RunReference {
    pub const fn new(year: u16, month: u8, day: u8, run_hour: RunHour) -> Self {
        Self {
            year,
            month,
            day,
            run_hour,
        }
    }

    /// The `gefs.<YYYYMMDD>` date component of every key under this run.
    pub fn date_component(&self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    /// The `gefs.<YYYYMMDD>/<HH>/` prefix common to every object under this
    /// run.
    pub fn run_prefix(&self) -> String {
        format!("gefs.{}/{}/", self.date_component(), self.run_hour)
    }
}

impl fmt::Display for RunReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}Z {}", self.run_hour, self.date_component())
    }
}

/// A GEFS ensemble member identity, as encoded in the object key's filename
/// (`gec00`/`gep01`.../`geavg`). This is the *file-naming* identity; the
/// physical "is this a control forecast, a positively/negatively perturbed
/// member, or an ensemble statistic" identity carried inside the decoded
/// GRIB2 message itself is [`crate::ensemble::EnsembleIdentity`] --
/// [`decode`][crate::decode::decode_field] cross-checks the two are
/// consistent rather than trusting the filename alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemberKey {
    /// `gec00` -- the unperturbed control forecast.
    Control,
    /// `gep01..gep30` -- a perturbed ensemble member, 1-indexed.
    Perturbed(u8),
    /// `geavg` -- the ensemble mean.
    Mean,
}

impl MemberKey {
    /// GEFS's own perturbed-member count as of this stage's empirical
    /// verification (`gep30` present, `gep31` absent, confirmed live).
    pub const MAX_PERTURBED_MEMBER: u8 = 30;

    pub fn file_stem(&self) -> String {
        match self {
            MemberKey::Control => "gec00".to_string(),
            MemberKey::Perturbed(n) => format!("gep{n:02}"),
            MemberKey::Mean => "geavg".to_string(),
        }
    }
}

impl fmt::Display for MemberKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file_stem())
    }
}

/// A GEFS product group (this crate only implements `pgrb2sp25`, the
/// 0.25-degree global surface/single-level product group the S07 stage
/// file names explicitly -- other groups such as `pgrb2ap5` (0.5 degree)
/// or `pgrb2bp25` (a different variable set) are out of scope for this PoC
/// but share the same key-naming convention if added later).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProductGroup(&'static str);

impl ProductGroup {
    pub const PGRB2S_P25: ProductGroup = ProductGroup("pgrb2sp25");

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

/// A forecast lead time, in whole hours -- always a multiple of 3 for
/// `pgrb2sp25` (`f000`, `f003`, `f006`, ... confirmed present through at
/// least `f240` in the live bucket for the run this crate's tests were
/// verified against).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForecastHour(pub u16);

impl ForecastHour {
    pub fn file_suffix(&self) -> String {
        format!("f{:03}", self.0)
    }
}

/// The GRIB2 object key (no bucket URL, no leading slash) for one
/// (run, member, product group, forecast hour) tuple, e.g.
/// `"gefs.20260912/12/atmos/pgrb2sp25/gep01.t12z.pgrb2s.0p25.f000"`.
pub fn object_key(
    run: RunReference,
    member: MemberKey,
    group: ProductGroup,
    forecast_hour: ForecastHour,
) -> String {
    format!(
        "{}atmos/{}/{}.t{}z.pgrb2s.0p25.{}",
        run.run_prefix(),
        group.as_str(),
        member.file_stem(),
        run.run_hour,
        forecast_hour.file_suffix()
    )
}

/// The `.idx` sidecar key for the same tuple.
pub fn idx_key(
    run: RunReference,
    member: MemberKey,
    group: ProductGroup,
    forecast_hour: ForecastHour,
) -> String {
    format!("{}.idx", object_key(run, member, group, forecast_hour))
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
        let run = RunReference::new(2026, 9, 12, RunHour::H12);
        let key = object_key(
            run,
            MemberKey::Perturbed(1),
            ProductGroup::PGRB2S_P25,
            ForecastHour(0),
        );
        assert_eq!(
            key,
            "gefs.20260912/12/atmos/pgrb2sp25/gep01.t12z.pgrb2s.0p25.f000"
        );
    }

    #[test]
    fn idx_key_appends_dot_idx() {
        let run = RunReference::new(2026, 9, 12, RunHour::H12);
        let key = idx_key(
            run,
            MemberKey::Control,
            ProductGroup::PGRB2S_P25,
            ForecastHour(3),
        );
        assert_eq!(
            key,
            "gefs.20260912/12/atmos/pgrb2sp25/gec00.t12z.pgrb2s.0p25.f003.idx"
        );
    }

    #[test]
    fn mean_member_file_stem_is_geavg() {
        assert_eq!(MemberKey::Mean.file_stem(), "geavg");
    }

    #[test]
    fn forecast_hour_is_zero_padded_to_three_digits() {
        assert_eq!(ForecastHour(0).file_suffix(), "f000");
        assert_eq!(ForecastHour(9).file_suffix(), "f009");
        assert_eq!(ForecastHour(240).file_suffix(), "f240");
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
