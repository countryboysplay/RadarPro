//! `radar-types` — shared domain types for RadarPro.
//!
//! # Status: S00 placeholder
//!
//! This crate is a deliberately minimal, compilable skeleton created during
//! the S00 "foundation" stage. It does **not** yet contain the canonical
//! polar radar domain model.
//!
//! The real model — preserving NEXRAD polar geometry as
//! `Volume -> Sweep -> Radial -> Moment`, with missing values, range
//! folding, units, timestamps, and source metadata preserved per
//! `GLOBAL_CONTRACT.md` — arrives in stage S01. Nothing here should be
//! treated as a scientifically meaningful representation of radar data.
//!
//! Types in this crate are provisional stubs that exist only so that the
//! Cargo workspace has a shared crate to depend on (see `apps/radar-cli`)
//! while later stages build out the real domain model.

/// A provisional, minimal description of a radar site's location.
///
/// This is intentionally trivial: an identifier plus a WGS84 latitude and
/// longitude in decimal degrees. It does not yet model elevation, site
/// metadata, or coordinate reference system nuances that `radar-geo` will
/// own in a later stage.
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    /// Short site identifier, e.g. a four-letter NEXRAD site code such as
    /// `"KTLX"`. Not validated yet — validation arrives with real ICAO/NEXRAD
    /// site handling in a later stage.
    pub id: String,
    /// Latitude in decimal degrees, WGS84.
    pub lat: f64,
    /// Longitude in decimal degrees, WGS84.
    pub lon: f64,
}

impl Site {
    /// Construct a new [`Site`].
    ///
    /// No range validation is performed on `lat`/`lon` yet; this is a
    /// placeholder constructor for the S00 stage.
    pub fn new(id: impl Into<String>, lat: f64, lon: f64) -> Self {
        Self {
            id: id.into(),
            lat,
            lon,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_new_stores_fields_unchanged() {
        let site = Site::new("KTLX", 35.3333, -97.2778);

        assert_eq!(site.id, "KTLX");
        assert_eq!(site.lat, 35.3333);
        assert_eq!(site.lon, -97.2778);
    }
}
