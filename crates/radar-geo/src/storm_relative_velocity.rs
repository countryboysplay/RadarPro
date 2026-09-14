//! Storm-Relative Velocity (SRV): subtract a single, uniform storm-motion
//! vector's beam-parallel (radial) component from a base radial velocity
//! (VEL) moment, gate by gate.
//!
//! Full scientific derivation, references, assumptions/limitations, and
//! validation cases:
//! `Agent Context/reference/algorithms/storm-relative-velocity.md`. This
//! module implements exactly that spec's formula and does not re-derive
//! or deviate from it.
//!
//! ```text
//! SRV(θ_gate) = v_base(θ_gate) − |SM| · cos(θ_gate − θ_sm)
//! ```
//!
//! where `θ_gate` is a gate's azimuth
//! ([`radar_types::Radial::azimuth_angle_deg`], degrees clockwise from
//! true north — the same convention this crate's own
//! [`crate::spherical::GreatCircle::bearing_deg`] uses), `θ_sm` is the
//! storm motion's compass bearing ([`StormMotion::direction_deg`]), and
//! `|SM|` is the storm motion speed ([`StormMotion::speed_mps`]), all in
//! degrees/meters-per-second as documented on those fields. Both
//! `v_base` and `SRV` are in m/s, outbound-positive / inbound-negative
//! (the same sign convention this repo's decoded VEL values already
//! use).
//!
//! `GateValue::Missing` and `GateValue::RangeFolded` gates are passed
//! through unchanged: SRV never invents a numeric value for a gate the
//! input VEL moment did not have one for (per `GLOBAL_CONTRACT.md`'s
//! "Preserve missing values, range folding").
//!
//! This is a pure per-gate arithmetic transform (a single storm motion
//! vector, applied uniformly to a whole radial's gates) with no I/O and
//! no failure mode, so it returns a plain [`Moment`], not a `Result`.

use radar_types::{GateValue, Moment};

/// A storm's horizontal motion vector, as a single, uniform value applied
/// to a whole radial/sweep (this repo's v1 scope — see the "Inputs"
/// section of `storm-relative-velocity.md` for why a per-cell,
/// auto-tracked vector is out of scope for now).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StormMotion {
    /// Storm motion speed in meters per second (`>= 0.0`).
    pub speed_mps: f32,
    /// Compass bearing, in degrees clockwise from true north, **toward**
    /// which the storm is moving (a heading, like ordinary NWS forecast
    /// language "moving northeast at 30 mph") — **not** the
    /// meteorological "wind direction" convention of naming the
    /// direction motion is coming *from*. See "Method" step 3 in
    /// `storm-relative-velocity.md`.
    pub direction_deg: f32,
}

/// Compute Storm-Relative Velocity for one radial's base velocity (VEL)
/// moment.
///
/// `azimuth_angle_deg` is that radial's azimuth
/// ([`radar_types::Radial::azimuth_angle_deg`]) — every gate in
/// `velocity` is treated as sharing this one azimuth, per this repo's
/// geometry (azimuth is keyed per radial, not per gate). `velocity` is
/// the radial's decoded `MomentKind::Velocity` [`Moment`].
///
/// Returns a new [`Moment`] with the same geometry as `velocity`
/// (`first_gate_range_km`, `gate_spacing_km`, `scale`, `offset` all
/// unchanged — SRV does not alter gate geometry or wire scale/offset
/// metadata, only the decoded value at each gate) and one output gate
/// per input gate:
///
/// - `GateValue::Value(v_base)` becomes
///   `GateValue::Value(v_base − |SM|·cos(θ_gate − θ_sm))`.
/// - `GateValue::Missing` and `GateValue::RangeFolded` are copied through
///   unchanged.
pub fn storm_relative_velocity(
    storm_motion: StormMotion,
    azimuth_angle_deg: f32,
    velocity: &Moment,
) -> Moment {
    let sm_radial_mps = storm_motion_radial_component_mps(storm_motion, azimuth_angle_deg);

    let gates = velocity
        .gates
        .iter()
        .map(|gate| match *gate {
            GateValue::Value(v_base) => GateValue::Value(v_base - sm_radial_mps),
            missing_or_folded => missing_or_folded,
        })
        .collect();

    Moment {
        first_gate_range_km: velocity.first_gate_range_km,
        gate_spacing_km: velocity.gate_spacing_km,
        scale: velocity.scale,
        offset: velocity.offset,
        gates,
    }
}

/// The outbound-positive radial (beam-parallel) component of a storm
/// motion vector along a gate's azimuth: `|SM| · cos(θ_gate − θ_sm)`.
///
/// Trigonometry is done in `f64` (matching this crate's other bearing
/// math, e.g. [`crate::spherical::distance_bearing`]) for precision, then
/// narrowed back to `f32` to match [`radar_types::GateValue::Value`]'s
/// own precision. See `storm-relative-velocity.md`'s "Method" section for
/// the full derivation of why plugging compass bearings directly into
/// `cos(θ_gate − θ_sm)` needs no additional angle-convention conversion.
fn storm_motion_radial_component_mps(storm_motion: StormMotion, azimuth_angle_deg: f32) -> f32 {
    let delta_deg = f64::from(azimuth_angle_deg) - f64::from(storm_motion.direction_deg);
    (f64::from(storm_motion.speed_mps) * delta_deg.to_radians().cos()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a VEL-shaped [`Moment`] with one gate per value in `values`.
    /// Geometry (`first_gate_range_km`/`gate_spacing_km`) and
    /// `scale`/`offset` are arbitrary fixed placeholders — SRV does not
    /// use or alter them, it only reads/copies them.
    fn moment_with_values(values: &[GateValue]) -> Moment {
        Moment {
            first_gate_range_km: 0.25,
            gate_spacing_km: 0.25,
            scale: 1.0,
            offset: 0.0,
            gates: values.to_vec(),
        }
    }

    fn value_moment(values: &[f32]) -> Moment {
        let gates: Vec<GateValue> = values.iter().copied().map(GateValue::Value).collect();
        moment_with_values(&gates)
    }

    fn assert_close(actual: f32, expected: f32, tolerance: f32, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{what}: expected {expected}, got {actual} (tolerance {tolerance})"
        );
    }

    fn expect_value(gate: GateValue, what: &str) -> f32 {
        match gate {
            GateValue::Value(v) => v,
            other => panic!("{what}: expected GateValue::Value, got {other:?}"),
        }
    }

    // --- (a) Zero storm motion is the identity transform ----------------

    #[test]
    fn zero_storm_motion_is_identity_transform() {
        let zero_motion = StormMotion {
            speed_mps: 0.0,
            direction_deg: 0.0,
        };
        let velocity = moment_with_values(&[
            GateValue::Value(-5.0),
            GateValue::Value(10.0),
            GateValue::Missing,
            GateValue::Value(25.0),
            GateValue::RangeFolded,
        ]);

        // Any azimuth: the zero-speed term is exactly zero regardless of
        // the angle difference, so this must hold for every azimuth, not
        // just a convenient one.
        for azimuth_angle_deg in [0.0_f32, 37.5, 90.0, 180.0, 271.25, 359.9] {
            let srv = storm_relative_velocity(zero_motion, azimuth_angle_deg, &velocity);
            // Exact equality (bit-for-bit): 0.0 * cos(x) is exact zero for
            // any finite cos(x), so no epsilon is needed here.
            assert_eq!(srv.gates, velocity.gates, "azimuth {azimuth_angle_deg}");
        }
    }

    // --- (b) A pure storm-motion-only field collapses to ~0 --------------

    #[test]
    fn pure_storm_motion_field_collapses_to_zero() {
        let storm_motion = StormMotion {
            speed_mps: 20.0,
            direction_deg: 90.0,
        };

        for azimuth_angle_deg in [0.0_f32, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
            let delta = f64::from(azimuth_angle_deg) - f64::from(storm_motion.direction_deg);
            let v_base = f64::from(storm_motion.speed_mps) * delta.to_radians().cos();
            let velocity = value_moment(&[v_base as f32]);

            let srv = storm_relative_velocity(storm_motion, azimuth_angle_deg, &velocity);
            let output = expect_value(srv.gates[0], "pure-motion field");
            assert_close(output, 0.0, 1e-4, &format!("azimuth {azimuth_angle_deg}"));
        }
    }

    // --- (c) Exact analytic case (the spec's worked numeric example) -----

    #[test]
    fn analytic_worked_example_matches_expected_values_exactly() {
        let storm_motion = StormMotion {
            speed_mps: 20.0,
            direction_deg: 90.0,
        };

        // (azimuth_deg, v_base, expected_srv) -- from
        // storm-relative-velocity.md's "Validation" (c) / "Method" worked
        // example, five one-gate radials at these five azimuths.
        let cases = [
            (0.0_f32, -5.0_f32, -5.0_f32),
            (45.0, 10.0, 10.0 - 10.0 * std::f32::consts::SQRT_2), // 10.0 - 20*cos(-45deg)
            (90.0, 25.0, 5.0),
            (180.0, 8.0, 8.0),
            (270.0, -15.0, 5.0),
        ];

        for (azimuth_angle_deg, v_base, expected_srv) in cases {
            let velocity = value_moment(&[v_base]);
            let srv = storm_relative_velocity(storm_motion, azimuth_angle_deg, &velocity);
            let output = expect_value(srv.gates[0], "analytic case");
            assert_close(
                output,
                expected_srv,
                1e-3,
                &format!("azimuth {azimuth_angle_deg}"),
            );
        }
    }

    // --- (d) Missing/range-folded gates are never computed through -------

    #[test]
    fn missing_and_range_folded_gates_pass_through_unchanged() {
        let non_zero_motion = StormMotion {
            speed_mps: 35.0,
            direction_deg: 213.0,
        };
        let velocity = moment_with_values(&[
            GateValue::Missing,
            GateValue::Value(12.0),
            GateValue::RangeFolded,
        ]);

        for azimuth_angle_deg in [0.0_f32, 90.0, 213.0, 300.0] {
            let srv = storm_relative_velocity(non_zero_motion, azimuth_angle_deg, &velocity);
            assert_eq!(
                srv.gates[0],
                GateValue::Missing,
                "azimuth {azimuth_angle_deg}"
            );
            assert_eq!(
                srv.gates[2],
                GateValue::RangeFolded,
                "azimuth {azimuth_angle_deg}"
            );
            // Never silently upgraded to a computed Value.
            assert!(!matches!(srv.gates[0], GateValue::Value(_)));
            assert!(!matches!(srv.gates[2], GateValue::Value(_)));
        }
    }

    // --- (e) Sign-convention regression check -----------------------------

    #[test]
    fn sign_convention_regression_toward_vs_away_and_negative_radial_component() {
        // Storm moving toward the 90-degree gate's own azimuth (i.e.
        // straight along that beam, outbound) vs. a storm moving directly
        // away from it (opposite bearing, 270 degrees), same speed.
        let storm_toward = StormMotion {
            speed_mps: 20.0,
            direction_deg: 90.0,
        };
        let storm_away = StormMotion {
            speed_mps: 20.0,
            direction_deg: 270.0,
        };
        let velocity_90 = value_moment(&[25.0]);

        let srv_toward = expect_value(
            storm_relative_velocity(storm_toward, 90.0, &velocity_90).gates[0],
            "storm toward",
        );
        let srv_away = expect_value(
            storm_relative_velocity(storm_away, 90.0, &velocity_90).gates[0],
            "storm away",
        );

        // A storm moving toward this gate's azimuth removes a large
        // positive (outbound) bias, reducing the residual magnitude...
        assert_close(srv_toward, 5.0, 1e-3, "storm moving toward the gate");
        // ...while a storm moving away adds that same bias back in,
        // increasing the residual magnitude in the same (outbound) sense.
        assert_close(srv_away, 45.0, 1e-3, "storm moving away from the gate");
        assert!(
            srv_toward.abs() < srv_away.abs(),
            "storm moving toward the gate's azimuth should yield a smaller \
             residual magnitude than one moving away: toward={srv_toward}, away={srv_away}"
        );

        // The 270-degree row from the worked example: the storm's own
        // radial component there is negative (-20 m/s, inbound), so
        // subtracting it (`v_base - (-20.0)`) must increase the result,
        // not decrease it -- guards against a project-and-add sign flip.
        let velocity_270 = value_moment(&[-15.0]);
        let srv_270 = expect_value(
            storm_relative_velocity(storm_toward, 270.0, &velocity_270).gates[0],
            "270-degree row",
        );
        assert_close(srv_270, 5.0, 1e-3, "270-degree row");
        assert!(
            srv_270 > -15.0,
            "subtracting a negative SM_radial must increase the result"
        );
    }
}
