//! Spherical-Earth great-circle geometry: distance, initial bearing, the
//! direct geodetic problem ("destination point"), and range rings built
//! from repeated destination-point calls.
//!
//! Formulas follow the standard spherical-trigonometry derivations
//! collected at Chris Veness's "Calculate distance, bearing and more
//! between Latitude/Longitude points"
//! (<https://www.movable-type.co.uk/scripts/latlong.html>); the
//! haversine distance formula itself traces to Sinnott, R. W., "Virtues
//! of the Haversine", *Sky and Telescope*, vol. 68, no. 2 (1984), p. 159.
//! See `crate::earth` for the mean-Earth-radius value used here.

use crate::earth::MEAN_EARTH_RADIUS_KM;
use radar_types::Site;

/// A point on the Earth's surface in decimal degrees.
///
/// Latitude/longitude values are ordinary WGS84 coordinates (matching how
/// [`radar_types::Site`] and NEXRAD metadata express position), but all
/// distance/bearing/destination math in this crate treats the Earth as a
/// sphere of [`crate::earth::MEAN_EARTH_RADIUS_KM`] — see the `earth`
/// module docs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLon {
    /// Latitude in decimal degrees, positive north.
    pub lat_deg: f64,
    /// Longitude in decimal degrees, positive east.
    pub lon_deg: f64,
}

impl LatLon {
    /// Construct a new [`LatLon`] from decimal-degree latitude/longitude.
    pub const fn new(lat_deg: f64, lon_deg: f64) -> Self {
        Self { lat_deg, lon_deg }
    }
}

impl From<&Site> for LatLon {
    /// A radar site's horizontal position as a [`LatLon`] (site height is
    /// not part of this horizontal-only type; see [`crate::beam`] for
    /// height math).
    fn from(site: &Site) -> Self {
        LatLon::new(site.latitude_deg, site.longitude_deg)
    }
}

/// The great-circle distance and initial bearing from one point to
/// another, on the spherical-Earth model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GreatCircle {
    /// Great-circle distance in kilometers.
    pub distance_km: f64,
    /// Initial bearing in degrees, clockwise from true north, in
    /// `[0.0, 360.0)`.
    pub bearing_deg: f64,
}

/// Compute the great-circle distance and initial bearing from `from` to
/// `to`, using the haversine distance formula and the standard spherical
/// initial-bearing formula (spherical-Earth model; see the module docs
/// for the source).
///
/// Returns `bearing_deg` of `0.0` when `from` and `to` are coincident (or
/// antipodal-through-the-pole in a way that leaves bearing undefined);
/// `atan2`'s well-defined behavior at the origin makes this safe without
/// a special case.
pub fn distance_bearing(from: LatLon, to: LatLon) -> GreatCircle {
    let (phi1, phi2) = (from.lat_deg.to_radians(), to.lat_deg.to_radians());
    let delta_phi = (to.lat_deg - from.lat_deg).to_radians();
    let delta_lambda = (to.lon_deg - from.lon_deg).to_radians();

    // Haversine distance (Sinnott 1984).
    let a = (delta_phi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (delta_lambda / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    let distance_km = MEAN_EARTH_RADIUS_KM * c;

    // Standard spherical initial-bearing formula.
    let y = delta_lambda.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * delta_lambda.cos();
    let bearing_deg = y.atan2(x).to_degrees().rem_euclid(360.0);

    GreatCircle {
        distance_km,
        bearing_deg,
    }
}

/// Compute the destination point reached by traveling `distance_km` along
/// the great circle starting at `origin` on initial bearing
/// `bearing_deg` (degrees clockwise from true north).
///
/// This is the spherical "direct geodetic problem", solved with the
/// standard spherical-trigonometry formula (see module docs).
pub fn destination_point(origin: LatLon, bearing_deg: f64, distance_km: f64) -> LatLon {
    let angular_distance = distance_km / MEAN_EARTH_RADIUS_KM;
    let bearing = bearing_deg.to_radians();
    let phi1 = origin.lat_deg.to_radians();
    let lambda1 = origin.lon_deg.to_radians();

    let phi2 = (phi1.sin() * angular_distance.cos()
        + phi1.cos() * angular_distance.sin() * bearing.cos())
    .asin();
    let lambda2 = lambda1
        + (bearing.sin() * angular_distance.sin() * phi1.cos())
            .atan2(angular_distance.cos() - phi1.sin() * phi2.sin());

    LatLon::new(phi2.to_degrees(), lambda2.to_degrees())
}

/// Generate the points forming a range ring of radius `radius_km` around
/// `center`: `num_points` points evenly spaced by bearing (0..360 degrees)
/// and connected in order, computed on demand via [`destination_point`].
///
/// This is a rendering/UI helper, independent of any decoded radar
/// volume; it does not persist or cache geometry for decoded data (see
/// the crate-level docs).
///
/// Returns an empty `Vec` if `num_points` is `0`.
pub fn range_ring(center: LatLon, radius_km: f64, num_points: usize) -> Vec<LatLon> {
    (0..num_points)
        .map(|i| {
            let bearing_deg = 360.0 * (i as f64) / (num_points as f64);
            destination_point(center, bearing_deg, radius_km)
        })
        .collect()
}

/// Generate one range ring (see [`range_ring`]) per radius in `radii_km`,
/// all centered on `center` and using the same point count.
pub fn range_rings(center: LatLon, radii_km: &[f64], num_points: usize) -> Vec<Vec<LatLon>> {
    radii_km
        .iter()
        .map(|&radius_km| range_ring(center, radius_km, num_points))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KTLX: LatLon = LatLon::new(35.3334, -97.2778);

    fn assert_close(actual: f64, expected: f64, tolerance: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: expected {expected}, got {actual} (tolerance {tolerance})"
        );
    }

    #[test]
    fn destination_then_distance_bearing_round_trips_due_north_short() {
        let dest = destination_point(KTLX, 0.0, 1.0);
        let gc = distance_bearing(KTLX, dest);
        assert_close(gc.distance_km, 1.0, 1e-6, "distance");
        assert_close(gc.bearing_deg, 0.0, 1e-6, "bearing");
    }

    #[test]
    fn destination_then_distance_bearing_round_trips_due_east_typical_range() {
        let dest = destination_point(KTLX, 90.0, 100.0);
        let gc = distance_bearing(KTLX, dest);
        assert_close(gc.distance_km, 100.0, 1e-6, "distance");
        assert_close(gc.bearing_deg, 90.0, 1e-6, "bearing");
    }

    #[test]
    fn destination_then_distance_bearing_round_trips_due_south_near_max_range() {
        let dest = destination_point(KTLX, 180.0, 460.0);
        let gc = distance_bearing(KTLX, dest);
        assert_close(gc.distance_km, 460.0, 1e-6, "distance");
        assert_close(gc.bearing_deg, 180.0, 1e-6, "bearing");
    }

    #[test]
    fn destination_then_distance_bearing_round_trips_due_west() {
        let dest = destination_point(KTLX, 270.0, 230.0);
        let gc = distance_bearing(KTLX, dest);
        assert_close(gc.distance_km, 230.0, 1e-6, "distance");
        assert_close(gc.bearing_deg, 270.0, 1e-6, "bearing");
    }

    #[test]
    fn destination_then_distance_bearing_round_trips_oblique_bearing() {
        let dest = destination_point(KTLX, 37.5, 150.0);
        let gc = distance_bearing(KTLX, dest);
        assert_close(gc.distance_km, 150.0, 1e-6, "distance");
        assert_close(gc.bearing_deg, 37.5, 1e-6, "bearing");
    }

    /// One degree of latitude along a meridian is a well-known,
    /// independently citable approximation of ~111.2 km (e.g. per
    /// standard geodesy references). Traveling due north by exactly that
    /// distance should land almost exactly 1 degree of latitude away,
    /// independent of this crate's own formulas being self-consistent.
    #[test]
    fn one_degree_of_latitude_is_about_111_2_km() {
        let start = LatLon::new(0.0, 0.0);
        let dest = destination_point(start, 0.0, 111.2);
        assert_close(dest.lat_deg, 1.0, 0.01, "latitude after 111.2 km north");
    }

    /// Independently-known reference distance: the great-circle distance
    /// between the equator/prime-meridian origin (0,0) and the point 90
    /// degrees away along the equator (0, 90) is one quarter of the
    /// Earth's circumference, `(pi/2) * R`.
    #[test]
    fn quarter_circumference_along_equator_matches_known_value() {
        let gc = distance_bearing(LatLon::new(0.0, 0.0), LatLon::new(0.0, 90.0));
        let expected = (std::f64::consts::PI / 2.0) * MEAN_EARTH_RADIUS_KM;
        assert_close(
            gc.distance_km,
            expected,
            1e-6,
            "quarter-circumference distance",
        );
        assert_close(gc.bearing_deg, 90.0, 1e-6, "bearing due east along equator");
    }

    #[test]
    fn distance_bearing_coincident_points_is_zero_distance_and_does_not_panic() {
        let gc = distance_bearing(KTLX, KTLX);
        assert_close(gc.distance_km, 0.0, 1e-9, "coincident distance");
        assert!(gc.bearing_deg.is_finite());
    }

    #[test]
    fn range_ring_has_requested_point_count_and_correct_radius() {
        let ring = range_ring(KTLX, 50.0, 8);
        assert_eq!(ring.len(), 8);
        for point in &ring {
            let gc = distance_bearing(KTLX, *point);
            assert_close(gc.distance_km, 50.0, 1e-6, "range ring point distance");
        }
    }

    #[test]
    fn range_ring_zero_points_is_empty() {
        assert!(range_ring(KTLX, 50.0, 0).is_empty());
    }

    #[test]
    fn range_rings_produces_one_ring_per_radius() {
        let rings = range_rings(KTLX, &[50.0, 100.0, 230.0], 16);
        assert_eq!(rings.len(), 3);
        for (ring, &radius_km) in rings.iter().zip([50.0, 100.0, 230.0].iter()) {
            assert_eq!(ring.len(), 16);
            let gc = distance_bearing(KTLX, ring[0]);
            assert_close(gc.distance_km, radius_km, 1e-6, "ring radius");
        }
    }

    #[test]
    fn latlon_from_site_uses_horizontal_position_only() {
        let site = Site::new("KTLX", 35.3334, -97.2778, 370.0);
        let latlon: LatLon = (&site).into();
        assert_eq!(latlon.lat_deg, 35.3334);
        assert_eq!(latlon.lon_deg, -97.2778);
    }
}
