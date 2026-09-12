//! Mapping between a cursor position and a decoded sweep's polar bins:
//! azimuth -> radial index, slant range -> gate index, and the composed
//! cursor-lat/lon -> radial/gate convenience this stage's exit criteria
//! is built around.
//!
//! All lookups operate directly on the polar metadata already carried by
//! [`radar_types::Sweep`]/[`radar_types::Radial`]/[`radar_types::Moment`]
//! on demand; nothing here converts a sweep into persistent lat/lon
//! geometry (see the crate-level docs and `GLOBAL_CONTRACT.md`).

use crate::beam::slant_range_from_ground_range_km;
use crate::spherical::{distance_bearing, LatLon};
use radar_types::{GateValue, Moment, MomentKind, Radial, Sweep};

/// The result of resolving a cursor position to a sweep's polar bins: a
/// zero-based radial index into the sweep's `radials`, and a zero-based
/// gate index into a moment's `gates`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RadialGate {
    pub radial_index: usize,
    pub gate_index: usize,
}

/// A cursor position resolved to polar coordinates relative to a radar
/// site: azimuth (degrees, clockwise from true north) and slant range
/// (kilometers, along the beam).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolarCoord {
    pub azimuth_deg: f64,
    pub slant_range_km: f64,
}

/// Find the radial in `radials` whose azimuth is closest to
/// `target_azimuth_deg`, treating azimuth as circular (`0.0 == 360.0`),
/// and reject the match if it is farther than `tolerance_deg` away.
///
/// This never assumes uniform spacing (`index = azimuth / nominal
/// spacing`): it compares the target against every radial's actual
/// azimuth. `tolerance_deg` is caller-supplied rather than inferred from
/// the data, because radial spacing can be irregular (dropped/degraded
/// radials) and a sparse or partial radial slice is not necessarily
/// representative of the sweep's real nominal spacing; see
/// [`find_radial_index_for_sweep`] for a convenience that derives a
/// tolerance from a whole sweep's documented azimuth resolution.
///
/// Returns `None` if `radials` is empty or if the closest match exceeds
/// `tolerance_deg` (e.g. a partial scan that does not cover the target
/// azimuth at all, or a gap between radials wider than the tolerance).
pub fn find_radial_index(
    radials: &[Radial],
    target_azimuth_deg: f64,
    tolerance_deg: f64,
) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, radial) in radials.iter().enumerate() {
        let diff = circular_diff_deg(f64::from(radial.azimuth_angle_deg), target_azimuth_deg);
        if best.is_none_or(|(_, best_diff)| diff < best_diff) {
            best = Some((index, diff));
        }
    }
    best.filter(|&(_, diff)| diff <= tolerance_deg)
        .map(|(index, _)| index)
}

/// Convenience over [`find_radial_index`] that derives `tolerance_deg`
/// from the sweep's own nominal azimuth resolution (the first radial's
/// [`radar_types::AzimuthResolution`], per Message 31 semantics — a
/// sweep's radials share one nominal resolution): `tolerance_deg = 2.0 *
/// azimuth_resolution.degrees()`. This tolerates ordinary scan jitter
/// (real radials rarely land exactly on the nominal grid) while still
/// rejecting a genuine gap (a partial scan, or dropped radials) that
/// spans meaningfully more than one nominal step.
///
/// Returns `None` if the sweep has no radials, or per the same rule as
/// [`find_radial_index`] otherwise.
pub fn find_radial_index_for_sweep(sweep: &Sweep, target_azimuth_deg: f64) -> Option<usize> {
    let tolerance_deg = f64::from(sweep.radials.first()?.azimuth_resolution.degrees()) * 2.0;
    find_radial_index(&sweep.radials, target_azimuth_deg, tolerance_deg)
}

/// Absolute angular difference between two azimuths in degrees, treating
/// azimuth as circular (`0.0 == 360.0`), always in `[0.0, 180.0]`.
fn circular_diff_deg(a: f64, b: f64) -> f64 {
    let diff = (a - b).rem_euclid(360.0);
    diff.min(360.0 - diff)
}

/// Find the gate in `moment.gates` covering `target_slant_range_km`.
///
/// Each gate `i` is treated as covering the half-open interval
/// `[center_i - spacing/2, center_i + spacing/2)`, i.e. its start is
/// included and its far edge belongs to the next gate (consistent with
/// how `first_gate_range_km` is a gate *center*, per
/// [`radar_types::Moment`]'s docs). Concretely: a target before the first
/// gate's start, or at/beyond the last gate's far edge, returns `None`;
/// a target exactly at a gate's start (including the first gate's start)
/// resolves to that gate.
///
/// Returns `None` if `moment.gates` is empty or `gate_spacing_km` is not
/// a finite positive value (a spacing of zero or less has no meaningful
/// gate boundaries to compute).
pub fn find_gate_index(moment: &Moment, target_slant_range_km: f64) -> Option<usize> {
    if moment.gates.is_empty() {
        return None;
    }
    let spacing_km = f64::from(moment.gate_spacing_km);
    if !(spacing_km.is_finite() && spacing_km > 0.0) {
        return None;
    }

    let first_gate_start_km = f64::from(moment.first_gate_range_km) - spacing_km / 2.0;
    if target_slant_range_km < first_gate_start_km {
        return None;
    }

    let offset_km = target_slant_range_km - first_gate_start_km;
    let index = (offset_km / spacing_km).floor();
    if !index.is_finite() || index < 0.0 {
        return None;
    }
    let index = index as usize;
    if index >= moment.gates.len() {
        None
    } else {
        Some(index)
    }
}

/// Resolve a cursor lat/lon to polar coordinates (azimuth, slant range)
/// relative to `site`, using `elevation_deg` for the ground-range ->
/// slant-range conversion.
///
/// Composes [`distance_bearing`] (which gives azimuth and ground range)
/// with [`slant_range_from_ground_range_km`] (which needs an elevation
/// angle to invert ground range into slant range). Returns `None` when
/// `elevation_deg` is at/near ±90 degrees, per
/// [`slant_range_from_ground_range_km`].
pub fn cursor_to_polar(site: LatLon, elevation_deg: f64, cursor: LatLon) -> Option<PolarCoord> {
    let great_circle = distance_bearing(site, cursor);
    let slant_range_km = slant_range_from_ground_range_km(great_circle.distance_km, elevation_deg)?;
    Some(PolarCoord {
        azimuth_deg: great_circle.bearing_deg,
        slant_range_km,
    })
}

/// End-to-end cursor -> radial/gate resolution for one sweep: given a
/// radar `site`, a decoded `sweep`, which `moment_kind` to resolve a gate
/// index against, and a `cursor` lat/lon, find the radial whose azimuth
/// matches the cursor's bearing from the site and the gate along that
/// radial's `moment_kind` moment whose range covers the cursor's slant
/// range.
///
/// Uses the sweep's own nominal elevation angle
/// (`sweep.elevation_angle_deg`) for the ground-range/slant-range
/// conversion, per `S02`'s guidance that a sweep's own nominal elevation
/// is the natural default (individual radials can vary slightly from it,
/// but that variation is far smaller than one gate's spacing at any
/// range this matters for).
///
/// Returns `None` if no radial matches within tolerance
/// ([`find_radial_index_for_sweep`]), the matched radial does not carry
/// `moment_kind`, or the resolved slant range falls outside that
/// moment's gates ([`find_gate_index`]).
pub fn locate_radial_gate(
    site: LatLon,
    sweep: &Sweep,
    moment_kind: MomentKind,
    cursor: LatLon,
) -> Option<RadialGate> {
    let polar = cursor_to_polar(site, f64::from(sweep.elevation_angle_deg), cursor)?;
    let radial_index = find_radial_index_for_sweep(sweep, polar.azimuth_deg)?;
    let moment = sweep.radials[radial_index].moments.get(&moment_kind)?;
    let gate_index = find_gate_index(moment, polar.slant_range_km)?;
    Some(RadialGate {
        radial_index,
        gate_index,
    })
}

/// Convenience over [`locate_radial_gate`] that also returns the resolved
/// gate's decoded source value.
pub fn locate_gate_value(
    site: LatLon,
    sweep: &Sweep,
    moment_kind: MomentKind,
    cursor: LatLon,
) -> Option<(RadialGate, &GateValue)> {
    let radial_gate = locate_radial_gate(site, sweep, moment_kind, cursor)?;
    let moment = sweep.radials[radial_gate.radial_index]
        .moments
        .get(&moment_kind)?;
    let gate_value = moment.gates.get(radial_gate.gate_index)?;
    Some((radial_gate, gate_value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spherical::destination_point;
    use radar_types::{
        AzimuthResolution, GateValue, Moment, MomentKind, Radial, RadialStatus, RadialStatusKind,
        Site, Sweep, Timestamp,
    };
    use std::collections::BTreeMap;

    fn radial_at(azimuth_angle_deg: f32, elevation_angle_deg: f32) -> Radial {
        Radial {
            azimuth_number: 1,
            azimuth_angle_deg,
            azimuth_resolution: AzimuthResolution::One,
            elevation_angle_deg,
            radial_status: RadialStatus {
                kind: RadialStatusKind::Intermediate,
                bad_data: false,
            },
            collection_time: Timestamp::from_epoch_millis(0),
            moments: BTreeMap::new(),
        }
    }

    fn moment_with_gates(
        first_gate_range_km: f32,
        gate_spacing_km: f32,
        gate_count: usize,
    ) -> Moment {
        Moment {
            first_gate_range_km,
            gate_spacing_km,
            scale: 1.0,
            offset: 0.0,
            gates: vec![GateValue::Value(10.0); gate_count],
        }
    }

    // --- find_radial_index: azimuth wraparound -----------------------

    #[test]
    fn find_radial_index_handles_wraparound_near_zero() {
        let radials = vec![
            radial_at(358.0, 0.5),
            radial_at(359.0, 0.5),
            radial_at(0.5, 0.5),
            radial_at(1.5, 0.5),
        ];

        // 359.6 is 0.4 from 359.0 and 0.9 (circular) from 0.5: 359.0 wins.
        assert_eq!(find_radial_index(&radials, 359.6, 1.0), Some(1));
        // 0.2 is 0.3 from 0.5 and 1.2 (circular) from 359.0: 0.5 wins.
        assert_eq!(find_radial_index(&radials, 0.2, 1.0), Some(2));
        // 0.0 exactly: nearest is 359.0 (1.0 away) vs 0.5 (0.5 away) -> 0.5 wins.
        assert_eq!(find_radial_index(&radials, 0.0, 1.0), Some(2));
    }

    // --- find_radial_index: irregular spacing -------------------------

    #[test]
    fn find_radial_index_handles_irregular_spacing() {
        // Non-uniform gaps: 10 -> 11 (1 deg), 11 -> 15 (4 deg), 15 -> 40
        // (25 deg, simulating a large gap from dropped/degraded radials).
        let radials = vec![
            radial_at(10.0, 0.5),
            radial_at(11.0, 0.5),
            radial_at(15.0, 0.5),
            radial_at(40.0, 0.5),
        ];

        assert_eq!(find_radial_index(&radials, 10.3, 1.0), Some(0));
        assert_eq!(find_radial_index(&radials, 12.0, 2.0), Some(1));
        assert_eq!(find_radial_index(&radials, 14.5, 2.0), Some(2));
        // 27.5 sits in the middle of the 25-degree gap: both 15.0 and
        // 40.0 are 12.5 away, well past any reasonable tolerance.
        assert_eq!(find_radial_index(&radials, 27.5, 2.0), None);
    }

    // --- find_radial_index: partial scan -------------------------------

    #[test]
    fn find_radial_index_returns_none_outside_partial_scan_coverage() {
        let radials: Vec<Radial> = (45..=135)
            .step_by(1)
            .map(|az| radial_at(az as f32, 0.5))
            .collect();

        assert_eq!(find_radial_index(&radials, 90.0, 1.0), Some(45));
        assert_eq!(find_radial_index(&radials, 200.0, 1.0), None);
        assert_eq!(find_radial_index(&radials, 10.0, 1.0), None);
    }

    #[test]
    fn find_radial_index_empty_slice_returns_none() {
        assert_eq!(find_radial_index(&[], 90.0, 5.0), None);
    }

    #[test]
    fn find_radial_index_for_sweep_derives_tolerance_from_resolution() {
        let mut sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![
                radial_at(0.0, 0.5),
                radial_at(1.0, 0.5),
                radial_at(2.0, 0.5),
            ],
        };
        for radial in &mut sweep.radials {
            radial.azimuth_resolution = AzimuthResolution::Half;
        }
        // Tolerance = 2 * 0.5 = 1.0 degree.
        assert_eq!(find_radial_index_for_sweep(&sweep, 0.9), Some(1));
        assert_eq!(find_radial_index_for_sweep(&sweep, 3.5), None);
    }

    // --- find_gate_index: range boundaries -----------------------------

    #[test]
    fn find_gate_index_range_boundaries() {
        // 5 gates, first center at 1.0 km, 0.25 km spacing.
        // Gate 0 covers [0.875, 1.125), gate 4 covers [1.875, 2.125).
        let moment = moment_with_gates(1.0, 0.25, 5);

        // Exactly at the first gate's start: included (gate 0).
        assert_eq!(find_gate_index(&moment, 0.875), Some(0));
        // Just before the first gate's start: excluded.
        assert_eq!(find_gate_index(&moment, 0.874), None);
        // Exactly at the last gate's far edge: excluded (belongs to a
        // nonexistent next gate under the half-open convention).
        assert_eq!(find_gate_index(&moment, 2.125), None);
        // Just before the last gate's far edge: included (gate 4).
        assert_eq!(find_gate_index(&moment, 2.124), Some(4));
        // Just past the last gate's far edge: excluded.
        assert_eq!(find_gate_index(&moment, 2.2), None);
    }

    #[test]
    fn find_gate_index_empty_gates_returns_none() {
        let moment = moment_with_gates(1.0, 0.25, 0);
        assert_eq!(find_gate_index(&moment, 1.0), None);
    }

    #[test]
    fn find_gate_index_rejects_non_positive_spacing() {
        let moment = moment_with_gates(1.0, 0.0, 5);
        assert_eq!(find_gate_index(&moment, 1.0), None);
    }

    // --- cursor_to_polar ------------------------------------------------

    #[test]
    fn cursor_to_polar_recovers_known_azimuth_and_range() {
        let site = LatLon::new(35.3334, -97.2778);
        let cursor = destination_point(site, 45.0, 100.0);

        let polar = cursor_to_polar(site, 0.5, cursor).expect("valid elevation");
        assert!((polar.azimuth_deg - 45.0).abs() < 1e-6);
        // At 0.5 degrees elevation, slant range is very close to the 100
        // km ground range used to build the cursor point.
        assert!((polar.slant_range_km - 100.0).abs() < 0.1);
    }

    #[test]
    fn cursor_to_polar_none_at_90_degree_elevation() {
        let site = LatLon::new(35.3334, -97.2778);
        let cursor = destination_point(site, 45.0, 100.0);
        assert!(cursor_to_polar(site, 90.0, cursor).is_none());
    }

    // --- end-to-end: cursor -> radial/gate, satisfying the S02 exit
    // criteria ("given a cursor coordinate and a decoded sweep, RadarPro
    // can identify the corresponding radial/gate and return the correct
    // source value").

    #[test]
    fn locate_radial_gate_recovers_known_radial_and_gate_end_to_end() {
        let site_struct = Site::new("KTLX", 35.3334, -97.2778, 370.0);
        let site = LatLon::from(&site_struct);
        let elevation_deg = 0.5_f32;

        // Build a synthetic sweep: 360 radials at 1-degree spacing, each
        // with a reflectivity moment of 100 gates, 0.25 km spacing,
        // first gate centered at 1.0 km.
        let mut radials = Vec::with_capacity(360);
        for az in 0..360 {
            let mut radial = radial_at(az as f32, elevation_deg);
            let mut moments = BTreeMap::new();
            moments.insert(MomentKind::Reflectivity, moment_with_gates(1.0, 0.25, 100));
            radial.moments = moments;
            radials.push(radial);
        }
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: elevation_deg,
            radials,
        };

        // Pick a known target radial/gate, and derive a cursor point that
        // should resolve back to exactly that radial and gate.
        let target_radial_index = 217usize; // azimuth 217 degrees
        let target_gate_index = 42usize;
        let gate_center_km = 1.0 + 0.25 * target_gate_index as f64;
        let ground_range_km =
            crate::beam::ground_range_km(gate_center_km, f64::from(elevation_deg));
        let cursor = destination_point(site, target_radial_index as f64, ground_range_km);

        let resolved = locate_radial_gate(site, &sweep, MomentKind::Reflectivity, cursor)
            .expect("cursor should resolve within the synthetic sweep");

        assert_eq!(resolved.radial_index, target_radial_index);
        assert_eq!(resolved.gate_index, target_gate_index);

        let (resolved_with_value, value) =
            locate_gate_value(site, &sweep, MomentKind::Reflectivity, cursor)
                .expect("cursor should resolve to a gate value");
        assert_eq!(resolved_with_value, resolved);
        assert_eq!(*value, GateValue::Value(10.0));
    }

    #[test]
    fn locate_radial_gate_none_when_moment_kind_absent() {
        let site = LatLon::new(35.3334, -97.2778);
        let mut radial = radial_at(45.0, 0.5);
        let mut moments = BTreeMap::new();
        moments.insert(MomentKind::Reflectivity, moment_with_gates(1.0, 0.25, 100));
        radial.moments = moments;
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![radial],
        };
        let cursor = destination_point(site, 45.0, 10.0);

        assert!(locate_radial_gate(site, &sweep, MomentKind::Velocity, cursor).is_none());
    }
}
