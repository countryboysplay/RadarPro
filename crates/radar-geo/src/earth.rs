//! Earth-model constants shared by every geometry calculation in this
//! crate.
//!
//! # Earth model choice
//!
//! This crate uses a **spherical Earth** (mean radius) model for all
//! horizontal (latitude/longitude) geometry: great-circle distance,
//! initial bearing, the spherical direct geodetic problem ("destination
//! point"), and range rings. It uses the standard **4/3 effective-Earth-
//! radius model** (a sphere with radius `(4/3) * mean_earth_radius_km`)
//! for the beam-height and ground-range/slant-range approximations, which
//! is the standard way weather-radar meteorology accounts for the
//! curvature of a normally-refracting atmosphere.
//!
//! This is a deliberate accuracy/complexity trade-off, not an oversight:
//! see `docs/adr/0006-earth-model-for-radar-geometry.md` for the full
//! rationale, the alternatives considered (WGS84 ellipsoidal geodesy /
//! Vincenty's formulae), and the conditions under which it should be
//! revisited.
//!
//! ## References
//! - Mean Earth radius: IUGG (International Union of Geodesy and
//!   Geophysics) mean radius `R1 = (2a + b) / 3` evaluated on the WGS84
//!   ellipsoid, which yields the commonly cited value of 6371.0088 km.
//! - 4/3 effective-Earth-radius model for standard atmospheric
//!   refraction: Doviak, R. J., and Zrnić, D. S., *Doppler Radar and
//!   Weather Observations*, 2nd ed. (1993), Section 2.2.1.

/// Mean Earth radius in kilometers (IUGG value, `R1 = (2a + b) / 3` on the
/// WGS84 ellipsoid), used for all spherical-Earth horizontal geometry
/// (great-circle distance, bearing, destination point, range rings).
pub const MEAN_EARTH_RADIUS_KM: f64 = 6371.0088;

/// Effective Earth radius in kilometers under the standard "4/3 Earth"
/// model for normal atmospheric refraction, used for beam-height and
/// ground-range/slant-range approximations.
///
/// Per Doviak & Zrnić (1993), Section 2.2.1: a normally-refracting
/// atmosphere bends a radar beam back toward the Earth just enough that
/// beam geometry over a *flat* Earth of radius `(4/3) * a` (where `a` is
/// the true/mean Earth radius) matches beam geometry over the *curved*
/// true Earth under standard refraction. Using this enlarged radius lets
/// the beam-height and ground-range formulas treat the Earth as locally
/// flat, which is what makes the small-angle approximations in
/// [`crate::beam`] valid.
pub const EFFECTIVE_EARTH_RADIUS_KM: f64 = (4.0 / 3.0) * MEAN_EARTH_RADIUS_KM;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_radius_is_four_thirds_mean_radius() {
        assert!((EFFECTIVE_EARTH_RADIUS_KM - (4.0 / 3.0) * 6371.0088).abs() < 1e-9);
    }
}
