//! WSR-88D radar site directory.
//!
//! Embeds the 159 real WSR-88D sites (ICAO, name, WGS84 lat/lon, antenna
//! height) shipped in this crate's own `fixtures/wsr88d-sites.json`, which
//! is a vendored copy of the ground-truthed data in the repo-root
//! `fixtures/nexrad-sites/wsr88d-sites.json` (sourced from the official NWS
//! API, `https://api.weather.gov/radar/stations`; see that directory's
//! `README.md` for full provenance). The copy here is vendored rather than
//! reached via a `../../fixtures/...` relative path so this crate does not
//! depend on files outside its own directory tree.
//!
//! No coordinates are hand-transcribed or invented: this module only
//! parses the embedded JSON.

use radar_types::Site;
use std::sync::OnceLock;

/// The vendored copy of `fixtures/nexrad-sites/wsr88d-sites.json` (159 WSR-88D
/// sites), embedded at compile time.
const SITES_JSON: &str = include_str!("../fixtures/wsr88d-sites.json");

#[derive(Debug, serde::Deserialize)]
struct RawSite {
    icao: String,
    name: String,
    lat: f64,
    lon: f64,
    height_m: f64,
}

/// A WSR-88D radar site's identity, human-readable name, and location.
///
/// Wraps [`radar_types::Site`] (reused rather than reinvented) with the
/// station name, which is not part of that shared domain type.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteInfo {
    /// Site identity/location, in the shared domain model.
    pub site: Site,
    /// Human-readable station name (e.g. `"Norman"` for `KTLX`), from the
    /// NWS site directory.
    pub name: String,
}

impl SiteInfo {
    /// Four-letter NEXRAD/ICAO site identifier, e.g. `"KTLX"`.
    pub fn icao(&self) -> &str {
        &self.site.icao
    }
}

static SITES: OnceLock<Vec<SiteInfo>> = OnceLock::new();

/// All 159 WSR-88D sites, sorted by ICAO identifier.
///
/// # Panics
///
/// Panics if the embedded `fixtures/wsr88d-sites.json` fails to parse. This
/// is compile-time-fixed data controlled by this crate, not remote or
/// user-supplied input, so a parse failure here means the vendored fixture
/// itself is corrupt -- a build-time defect to fix, not a runtime condition
/// to recover from. (This is distinct from the rest of this crate, which
/// treats all *network*-sourced data as untrusted and never panics on it.)
pub fn all_sites() -> &'static [SiteInfo] {
    SITES
        .get_or_init(|| {
            let raw: Vec<RawSite> = serde_json::from_str(SITES_JSON)
                .expect("embedded fixtures/wsr88d-sites.json must be valid JSON");
            raw.into_iter()
                .map(|r| SiteInfo {
                    site: Site::new(r.icao, r.lat, r.lon, r.height_m),
                    name: r.name,
                })
                .collect()
        })
        .as_slice()
}

/// Look up a WSR-88D site by its ICAO identifier (case-insensitive).
pub fn find_site(icao: &str) -> Option<&'static SiteInfo> {
    all_sites()
        .iter()
        .find(|s| s.icao().eq_ignore_ascii_case(icao))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_all_159_wsr88d_sites() {
        assert_eq!(all_sites().len(), 159);
    }

    #[test]
    fn sites_are_sorted_by_icao() {
        let icaos: Vec<&str> = all_sites().iter().map(|s| s.icao()).collect();
        let mut sorted = icaos.clone();
        sorted.sort();
        assert_eq!(
            icaos, sorted,
            "wsr88d-sites.json should be pre-sorted by ICAO"
        );
    }

    /// Ground truth from `fixtures/nexrad-sites/wsr88d-sites.json` (and
    /// cross-checked against this repo's real S01 fixture volume
    /// `KTLX20240601_000353_V06`, whose own decoded VOL block reports
    /// (35.3333, -97.2778) -- matching to 4 decimal places).
    #[test]
    fn ktlx_matches_known_ground_truth() {
        let ktlx = find_site("KTLX").expect("KTLX must be present");
        assert_eq!(ktlx.name, "Norman");
        assert!((ktlx.site.latitude_deg - 35.33305).abs() < 1e-6);
        assert!((ktlx.site.longitude_deg - (-97.27775)).abs() < 1e-6);
        assert!((ktlx.site.height_m - 369.72).abs() < 1e-6);
    }

    /// Ground truth for the other S01 fixture site, `KFTG20240601_000116_V06`.
    #[test]
    fn kftg_matches_known_ground_truth() {
        let kftg = find_site("KFTG").expect("KFTG must be present");
        assert_eq!(kftg.name, "Denver");
        assert!((kftg.site.latitude_deg - 39.78663).abs() < 1e-6);
        assert!((kftg.site.longitude_deg - (-104.5458)).abs() < 1e-6);
        assert!((kftg.site.height_m - 1675.49).abs() < 1e-6);
    }

    #[test]
    fn find_site_is_case_insensitive() {
        assert!(find_site("ktlx").is_some());
        assert!(find_site("KTLX").is_some());
    }

    #[test]
    fn find_site_returns_none_for_unknown_icao() {
        assert!(find_site("ZZZZ").is_none());
    }
}
