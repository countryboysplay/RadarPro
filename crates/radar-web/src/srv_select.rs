//! Pure, host-testable logic (S11 Phase 2b) for building a synthetic
//! Storm-Relative Velocity (SRV) [`Sweep`] out of an already-resolved VEL
//! sweep: applies `radar_geo::storm_relative_velocity::storm_relative_velocity`
//! per radial and reassembles the result as a new sweep carrying exactly one
//! moment, [`MomentKind::StormRelativeVelocity`].
//!
//! No `wgpu`/`wasm-bindgen`/`web-sys` dependency at all -- like
//! [`crate::sweep_select`]/[`crate::render_select`], this is plain
//! `radar_types`/`radar_geo` computation, so it compiles and runs its
//! `#[test]`s on any target (`cargo test -p radar-web` on the host).
//! [`crate::browser`] (wasm32 GPU glue) calls into this module rather than
//! duplicating the per-radial transform-and-reassemble here.
//!
//! This module does not compute or re-derive the SRV formula itself --
//! that is entirely `radar_geo::storm_relative_velocity::storm_relative_velocity`,
//! already implemented and unit-tested in the `radar-geo` crate (see
//! `Agent Context/reference/algorithms/storm-relative-velocity.md` for the
//! science). This module's only job is sweep/radial bookkeeping: iterating
//! `sweep.radials`, looking up each radial's VEL moment, calling that
//! function once per radial, and reassembling a new [`Sweep`].

use std::collections::BTreeMap;

use radar_geo::storm_relative_velocity::{storm_relative_velocity, StormMotion};
use radar_types::{MomentKind, Radial, Sweep};

/// Build a synthetic SRV sweep from `sweep` (already resolved to carry the
/// [`MomentKind::Velocity`] moment, e.g. via
/// [`crate::render_select::resolve_sweep`] with `MomentKind::Velocity`) and
/// a uniform `storm_motion` vector.
///
/// The returned [`Sweep`] has the same `elevation_number`/
/// `elevation_angle_deg` and the same `radials` (identity, order, azimuth,
/// status, collection time) as `sweep` -- only each [`Radial`]'s `moments`
/// map is replaced. Each output radial's `moments` map has **exactly one**
/// entry, `MomentKind::StormRelativeVelocity` mapped to the transformed
/// [`radar_types::Moment`] -- every other moment the input radial may have
/// carried (REF, SW, ZDR, CC, PHI, or even the original VEL itself) is
/// deliberately dropped: this synthetic sweep exists only to be fed into
/// the existing per-`MomentKind` render path for the one kind it was built
/// for ([`MomentKind::StormRelativeVelocity`]), per this feature's
/// architecture decision that SRV is a distinct, independently-selectable
/// product rather than a display option layered on VEL.
///
/// A radial with no [`MomentKind::Velocity`] entry (should not happen for a
/// sweep already resolved to carry VEL, since `resolve_sweep` only requires
/// *at least one* radial to carry the moment -- a partial per-radial VEL
/// dropout is a real, if rare, possibility on real data) is carried through
/// with an **empty** `moments` map rather than skipped or panicking: it
/// contributes no gates to the synthetic SRV sweep for that azimuth, the
/// same "moment absent for this radial" contract [`Radial::moments`]
/// already documents for every other moment.
pub fn build_storm_relative_velocity_sweep(sweep: &Sweep, storm_motion: StormMotion) -> Sweep {
    Sweep {
        elevation_number: sweep.elevation_number,
        elevation_angle_deg: sweep.elevation_angle_deg,
        radials: sweep
            .radials
            .iter()
            .map(|radial| transform_radial(radial, storm_motion))
            .collect(),
    }
}

/// Transform one radial: look up its VEL moment (if present) and replace
/// `moments` with a single `StormRelativeVelocity` entry, or an empty map
/// if this radial carried no VEL moment at all -- see
/// [`build_storm_relative_velocity_sweep`]'s doc comment.
fn transform_radial(radial: &Radial, storm_motion: StormMotion) -> Radial {
    let mut moments = BTreeMap::new();
    if let Some(velocity) = radial.moments.get(&MomentKind::Velocity) {
        let srv = storm_relative_velocity(storm_motion, radial.azimuth_angle_deg, velocity);
        moments.insert(MomentKind::StormRelativeVelocity, srv);
    }
    Radial {
        azimuth_number: radial.azimuth_number,
        azimuth_angle_deg: radial.azimuth_angle_deg,
        azimuth_resolution: radial.azimuth_resolution,
        elevation_angle_deg: radial.elevation_angle_deg,
        radial_status: radial.radial_status,
        collection_time: radial.collection_time,
        moments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{
        AzimuthResolution, GateValue, Moment, RadialStatus, RadialStatusKind, Timestamp,
    };

    /// Build a minimal VEL-carrying radial at `azimuth_angle_deg` with one
    /// gate per value in `values`. Geometry (`first_gate_range_km`/
    /// `gate_spacing_km`) and `scale`/`offset` are arbitrary fixed
    /// placeholders, mirroring `radar_geo::storm_relative_velocity`'s own
    /// test fixtures for consistency.
    fn radial_with_velocity(azimuth_angle_deg: f32, values: &[GateValue]) -> Radial {
        let mut moments = BTreeMap::new();
        moments.insert(
            MomentKind::Velocity,
            Moment {
                first_gate_range_km: 0.25,
                gate_spacing_km: 0.25,
                scale: 1.0,
                offset: 0.0,
                gates: values.to_vec(),
            },
        );
        Radial {
            azimuth_number: 1,
            azimuth_angle_deg,
            azimuth_resolution: AzimuthResolution::One,
            elevation_angle_deg: 0.5,
            radial_status: RadialStatus {
                kind: RadialStatusKind::Intermediate,
                bad_data: false,
            },
            collection_time: Timestamp::from_epoch_millis(0),
            moments,
        }
    }

    fn radial_without_velocity(azimuth_angle_deg: f32) -> Radial {
        Radial {
            azimuth_number: 2,
            azimuth_angle_deg,
            azimuth_resolution: AzimuthResolution::One,
            elevation_angle_deg: 0.5,
            radial_status: RadialStatus {
                kind: RadialStatusKind::Intermediate,
                bad_data: false,
            },
            collection_time: Timestamp::from_epoch_millis(0),
            moments: BTreeMap::new(),
        }
    }

    fn expect_value(gate: GateValue) -> f32 {
        match gate {
            GateValue::Value(v) => v,
            other => panic!("expected GateValue::Value, got {other:?}"),
        }
    }

    /// The spec's `|SM|=20 m/s, θ_sm=90°` worked example (also used by
    /// `radar_geo::storm_relative_velocity`'s own tests), reused here for a
    /// full sweep-level structural check rather than a re-derivation of the
    /// formula.
    #[test]
    fn builds_srv_sweep_matching_the_spec_worked_example() {
        let storm_motion = StormMotion {
            speed_mps: 20.0,
            direction_deg: 90.0,
        };
        let sweep = Sweep {
            elevation_number: 3,
            elevation_angle_deg: 0.5,
            radials: vec![
                radial_with_velocity(0.0, &[GateValue::Value(-5.0)]),
                radial_with_velocity(45.0, &[GateValue::Value(10.0)]),
                radial_with_velocity(90.0, &[GateValue::Value(25.0)]),
                radial_with_velocity(180.0, &[GateValue::Value(8.0)]),
                radial_with_velocity(270.0, &[GateValue::Value(-15.0)]),
            ],
        };

        let srv_sweep = build_storm_relative_velocity_sweep(&sweep, storm_motion);

        // Sweep-level identity is preserved unchanged.
        assert_eq!(srv_sweep.elevation_number, sweep.elevation_number);
        assert_eq!(srv_sweep.elevation_angle_deg, sweep.elevation_angle_deg);
        assert_eq!(srv_sweep.radials.len(), sweep.radials.len());

        let expected = [
            -5.0_f32,
            10.0 - 10.0 * std::f32::consts::SQRT_2,
            5.0,
            8.0,
            5.0,
        ];
        for (radial, &expected_srv) in srv_sweep.radials.iter().zip(expected.iter()) {
            assert_eq!(radial.moments.len(), 1, "exactly one moment per radial");
            let srv_moment = radial
                .moments
                .get(&MomentKind::StormRelativeVelocity)
                .expect("StormRelativeVelocity moment present");
            let value = expect_value(srv_moment.gates[0]);
            assert!(
                (value - expected_srv).abs() <= 1e-3,
                "azimuth {}: expected {expected_srv}, got {value}",
                radial.azimuth_angle_deg
            );
        }
    }

    #[test]
    fn preserves_radial_identity_and_geometry_fields() {
        let storm_motion = StormMotion {
            speed_mps: 0.0,
            direction_deg: 0.0,
        };
        let input_radial = radial_with_velocity(37.5, &[GateValue::Value(1.0)]);
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![input_radial.clone()],
        };

        let srv_sweep = build_storm_relative_velocity_sweep(&sweep, storm_motion);
        let output_radial = &srv_sweep.radials[0];

        assert_eq!(output_radial.azimuth_number, input_radial.azimuth_number);
        assert_eq!(
            output_radial.azimuth_angle_deg,
            input_radial.azimuth_angle_deg
        );
        assert_eq!(
            output_radial.azimuth_resolution,
            input_radial.azimuth_resolution
        );
        assert_eq!(
            output_radial.elevation_angle_deg,
            input_radial.elevation_angle_deg
        );
        assert_eq!(output_radial.radial_status, input_radial.radial_status);
        assert_eq!(output_radial.collection_time, input_radial.collection_time);

        let input_vel = &input_radial.moments[&MomentKind::Velocity];
        let output_srv = &output_radial.moments[&MomentKind::StormRelativeVelocity];
        assert_eq!(
            output_srv.first_gate_range_km,
            input_vel.first_gate_range_km
        );
        assert_eq!(output_srv.gate_spacing_km, input_vel.gate_spacing_km);
        assert_eq!(output_srv.scale, input_vel.scale);
        assert_eq!(output_srv.offset, input_vel.offset);
    }

    #[test]
    fn missing_and_range_folded_gates_pass_through_unchanged() {
        let storm_motion = StormMotion {
            speed_mps: 35.0,
            direction_deg: 213.0,
        };
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![radial_with_velocity(
                90.0,
                &[
                    GateValue::Missing,
                    GateValue::Value(12.0),
                    GateValue::RangeFolded,
                ],
            )],
        };

        let srv_sweep = build_storm_relative_velocity_sweep(&sweep, storm_motion);
        let srv_moment = &srv_sweep.radials[0].moments[&MomentKind::StormRelativeVelocity];
        assert_eq!(srv_moment.gates[0], GateValue::Missing);
        assert!(matches!(srv_moment.gates[1], GateValue::Value(_)));
        assert_eq!(srv_moment.gates[2], GateValue::RangeFolded);
    }

    #[test]
    fn a_radial_with_no_velocity_moment_gets_an_empty_moments_map() {
        let storm_motion = StormMotion {
            speed_mps: 10.0,
            direction_deg: 0.0,
        };
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![
                radial_with_velocity(0.0, &[GateValue::Value(1.0)]),
                radial_without_velocity(1.0),
            ],
        };

        let srv_sweep = build_storm_relative_velocity_sweep(&sweep, storm_motion);
        assert_eq!(srv_sweep.radials.len(), 2);
        assert_eq!(srv_sweep.radials[0].moments.len(), 1);
        assert!(srv_sweep.radials[1].moments.is_empty());
    }

    #[test]
    fn other_original_moments_are_not_carried_into_the_synthetic_sweep() {
        let storm_motion = StormMotion {
            speed_mps: 0.0,
            direction_deg: 0.0,
        };
        let mut radial = radial_with_velocity(0.0, &[GateValue::Value(1.0)]);
        radial.moments.insert(
            MomentKind::Reflectivity,
            Moment {
                first_gate_range_km: 1.0,
                gate_spacing_km: 0.25,
                scale: 1.0,
                offset: 0.0,
                gates: vec![GateValue::Value(20.0)],
            },
        );
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![radial],
        };

        let srv_sweep = build_storm_relative_velocity_sweep(&sweep, storm_motion);
        let moments = &srv_sweep.radials[0].moments;
        assert_eq!(moments.len(), 1);
        assert!(!moments.contains_key(&MomentKind::Reflectivity));
        assert!(!moments.contains_key(&MomentKind::Velocity));
        assert!(moments.contains_key(&MomentKind::StormRelativeVelocity));
    }
}
