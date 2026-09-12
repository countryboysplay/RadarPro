//! Beam-height and ground-range/slant-range approximations under the
//! standard 4/3 effective-Earth-radius model.
//!
//! Both formulas here are the standard *paired* small-angle
//! approximations used throughout operational radar meteorology for beam
//! geometry over the effective (4/3) Earth — not independently invented:
//!
//! - Beam height above the site: `h ≈ r·sin(θ) + r²/(2·ae)`
//! - Ground range: `s ≈ r·cos(θ)`
//!
//! where `r` is slant range, `θ` is elevation angle, and `ae` is the
//! effective Earth radius ([`crate::earth::EFFECTIVE_EARTH_RADIUS_KM`]).
//! See Doviak, R. J., and Zrnić, D. S., *Doppler Radar and Weather
//! Observations*, 2nd ed. (1993), Section 2.2.1.
//!
//! # Limitations
//!
//! These are small-angle approximations: they are standard and accurate
//! enough for the elevation angles WSR-88D VCPs actually use (0-19.5
//! degrees for the operational cuts this project targets; the RDA can cut
//! elevations up to 70 degrees but the approximation is typically applied
//! throughout radar meteorology software regardless). Accuracy degrades
//! as `θ` approaches 90 degrees, which VCPs approach but essentially
//! never reach exactly. This crate does not implement full ray tracing or
//! non-standard-refraction correction (out of scope for this stage); it
//! only guards the exact `θ = ±90°` case where the ground-range inverse
//! would otherwise divide by (near) zero.

use crate::earth::EFFECTIVE_EARTH_RADIUS_KM;

/// How close `cos(elevation)` may get to zero before
/// [`slant_range_from_ground_range_km`] refuses to divide by it and
/// returns `None` instead of producing `inf`/`NaN`. `1e-6` corresponds to
/// an elevation angle within roughly 0.00006 degrees of ±90 degrees,
/// tight enough to never reject any real WSR-88D elevation angle while
/// still catching the exact degenerate case.
const COS_NEAR_ZERO_EPSILON: f64 = 1e-6;

/// Approximate height above the radar site of a point at `slant_range_km`
/// along a beam at `elevation_deg`, under the standard 4/3-Earth-radius
/// small-angle approximation: `h ≈ r·sin(θ) + r²/(2·ae)`, plus
/// `site_height_km` to give height above mean sea level.
///
/// See the module docs for the formula's source and limitations. This
/// function never panics: `sin`/`cos` are total over all finite inputs,
/// so no division-by-zero guard is needed here (unlike
/// [`slant_range_from_ground_range_km`]).
pub fn beam_height_km(slant_range_km: f64, elevation_deg: f64, site_height_km: f64) -> f64 {
    let theta = elevation_deg.to_radians();
    let curvature_term = (slant_range_km * slant_range_km) / (2.0 * EFFECTIVE_EARTH_RADIUS_KM);
    site_height_km + slant_range_km * theta.sin() + curvature_term
}

/// Approximate ground (great-circle) range for a point at
/// `slant_range_km` along a beam at `elevation_deg`, under the standard
/// small-angle approximation `s ≈ r·cos(θ)`, paired with
/// [`beam_height_km`] above.
pub fn ground_range_km(slant_range_km: f64, elevation_deg: f64) -> f64 {
    slant_range_km * elevation_deg.to_radians().cos()
}

/// Invert [`ground_range_km`]: recover slant range from ground range and
/// elevation angle, `r = s / cos(θ)`.
///
/// Returns `None` rather than an infinite or `NaN` value when
/// `cos(elevation_deg)` is within `COS_NEAR_ZERO_EPSILON` of zero (i.e.
/// `elevation_deg` is at or extremely near ±90 degrees), since the
/// ground-range approximation is degenerate there (a beam pointing
/// straight up/down has no meaningful ground range to invert).
pub fn slant_range_from_ground_range_km(ground_range_km: f64, elevation_deg: f64) -> Option<f64> {
    let cos_theta = elevation_deg.to_radians().cos();
    if cos_theta.abs() < COS_NEAR_ZERO_EPSILON {
        None
    } else {
        Some(ground_range_km / cos_theta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64, tolerance: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: expected {expected}, got {actual} (tolerance {tolerance})"
        );
    }

    #[test]
    fn ground_range_at_zero_elevation_equals_slant_range() {
        assert_close(
            ground_range_km(230.0, 0.0),
            230.0,
            1e-9,
            "ground range at 0 deg",
        );
    }

    #[test]
    fn beam_height_at_zero_elevation_is_small_but_positive_above_site() {
        // At 0 degrees elevation, only the curvature term contributes
        // (sin(0) = 0), so height above the site should be small but
        // strictly positive at WSR-88D-typical range.
        let height_above_site = beam_height_km(230.0, 0.0, 0.0);
        assert!(height_above_site > 0.0);
        assert!(
            height_above_site < 10.0,
            "curvature term should stay modest at 230 km"
        );
    }

    #[test]
    fn beam_height_includes_site_height_offset() {
        let h0 = beam_height_km(100.0, 5.0, 0.0);
        let h_with_site = beam_height_km(100.0, 5.0, 0.370);
        assert_close(h_with_site - h0, 0.370, 1e-9, "site height offset");
    }

    /// Higher elevation angles should increase beam height and decrease
    /// ground range relative to slant range, at real WSR-88D VCP
    /// elevations and near-max-range slant ranges (up to ~460 km, per the
    /// S01 KTLX fixture scale).
    #[test]
    fn higher_elevation_increases_height_and_decreases_ground_range() {
        let slant_range_km = 200.0;
        let site_height_km = 0.370; // KTLX site height, in km.

        let h_low = beam_height_km(slant_range_km, 0.5, site_height_km);
        let h_5 = beam_height_km(slant_range_km, 5.0, site_height_km);
        let h_19_5 = beam_height_km(slant_range_km, 19.5, site_height_km);
        assert!(
            h_low < h_5,
            "height should increase from 0.5 to 5.0 degrees"
        );
        assert!(
            h_5 < h_19_5,
            "height should increase from 5.0 to 19.5 degrees"
        );

        let g_low = ground_range_km(slant_range_km, 0.5);
        let g_5 = ground_range_km(slant_range_km, 5.0);
        let g_19_5 = ground_range_km(slant_range_km, 19.5);
        assert!(
            g_low > g_5,
            "ground range should decrease from 0.5 to 5.0 degrees"
        );
        assert!(
            g_5 > g_19_5,
            "ground range should decrease from 5.0 to 19.5 degrees"
        );
        assert!(
            g_19_5 < slant_range_km,
            "ground range must stay below slant range"
        );
    }

    #[test]
    fn slant_range_from_ground_range_inverts_ground_range_km() {
        let elevation_deg = 19.5;
        let slant_range_km = 460.0;
        let ground = ground_range_km(slant_range_km, elevation_deg);
        let recovered =
            slant_range_from_ground_range_km(ground, elevation_deg).expect("valid elevation");
        assert_close(recovered, slant_range_km, 1e-6, "slant range round trip");
    }

    #[test]
    fn slant_range_from_ground_range_none_at_exactly_90_degrees() {
        assert!(slant_range_from_ground_range_km(100.0, 90.0).is_none());
        assert!(slant_range_from_ground_range_km(100.0, -90.0).is_none());
    }

    #[test]
    fn slant_range_from_ground_range_never_panics_near_90_degrees() {
        for bump in [-0.001, -0.0001, 0.0, 0.0001, 0.001] {
            let elevation_deg = 90.0 + bump;
            // Must not panic; result is either a finite value or `None`,
            // never `inf`/`NaN` leaking out unchecked.
            if let Some(r) = slant_range_from_ground_range_km(100.0, elevation_deg) {
                assert!(r.is_finite());
            }
        }
    }
}
