//! Non-weather echo filter ("smart filter"): hide reflectivity gates whose
//! dual-pol readings say the target isn't precipitation -- insects, birds,
//! ground clutter, chaff, smoke. Reflectivity strength alone can't separate
//! them (light snow and clear-air insects are both 5-20 dBZ); dual-pol can.
//!
//! A gate is hidden when either test says non-weather:
//!
//! - CC below [`MIN_WEATHER_CC`]: rain and snow have CC near 1; biological
//!   and clutter targets usually sit well below 0.8.
//! - Weak echo (below [`INSECT_TEST_MAX_DBZ`]) with ZDR above
//!   [`INSECT_MIN_ZDR_DB`]: light rain is small, nearly round drops (ZDR
//!   about 0-1 dB), while insects are long and thin (ZDR 4-10 dB). Needed
//!   because insect layers can have high CC: live KVWX clear air on
//!   2026-10-19 was 64% CC >= 0.9 but had median ZDR 6.9 dB, and MRMS showed
//!   no precipitation there.
//!
//! Gates at or above [`ALWAYS_SHOW_DBZ`] are never hidden: large hail,
//! tornado debris and the melting layer also lower CC, and those are the
//! echoes a chaser most needs to see. A gate with no CC reading is kept --
//! the filter only removes echo it has evidence against.
//!
//! On split-cut low tilts the surveillance sweep (REF/ZDR/CC/PHI) and the
//! Doppler sweep (REF/VEL/SW) are separate; a sweep without CC borrows CC
//! and ZDR from the CC-carrying sweep at the nearest elevation.

use radar_types::{GateValue, Moment, MomentKind, Sweep, Volume};

use crate::lookup::{find_gate_index, find_radial_index_for_sweep};

/// Below this CC a gate is treated as non-weather.
pub const MIN_WEATHER_CC: f32 = 0.80;

/// Weak echo with ZDR above this (dB) is treated as insects.
pub const INSECT_MIN_ZDR_DB: f32 = 4.0;

/// The ZDR insect test only applies below this reflectivity: size-sorted
/// big drops at storm edges can reach ZDR 3-5 dB at 30+ dBZ.
pub const INSECT_TEST_MAX_DBZ: f32 = 30.0;

/// Reflectivity at or above this is always shown (hail, debris).
pub const ALWAYS_SHOW_DBZ: f32 = 35.0;

/// How far apart two sweeps' elevations may be for one to lend the other
/// its CC (split cuts share an angle to within a few hundredths).
const MAX_ELEVATION_MATCH_DEG: f32 = 0.25;

/// Mark non-weather reflectivity gates of sweep `sweep_index` as
/// [`GateValue::Missing`], in place. Returns `false` (and changes nothing)
/// when that sweep has no REF or no CC is available at its elevation.
pub fn hide_non_weather_reflectivity(volume: &mut Volume, sweep_index: usize) -> bool {
    let Some(sweep) = volume.sweeps.get(sweep_index) else {
        return false;
    };
    if !carries(sweep, MomentKind::Reflectivity) {
        return false;
    }
    let cc_sweep = if carries(sweep, MomentKind::CorrelationCoefficient) {
        sweep
    } else {
        let elevation_gap = |s: &Sweep| (s.elevation_angle_deg - sweep.elevation_angle_deg).abs();
        let Some(nearest) = volume
            .sweeps
            .iter()
            .filter(|s| carries(s, MomentKind::CorrelationCoefficient))
            .filter(|s| elevation_gap(s) <= MAX_ELEVATION_MATCH_DEG)
            .min_by(|a, b| elevation_gap(a).total_cmp(&elevation_gap(b)))
        else {
            return false;
        };
        nearest
    };

    // Per radial: which REF gates to hide (computed before mutating, since
    // the CC may come from this same sweep).
    let masks: Vec<Option<Vec<bool>>> = sweep
        .radials
        .iter()
        .map(|radial| {
            let reflectivity = radial.moments.get(&MomentKind::Reflectivity)?;
            let cc_index =
                find_radial_index_for_sweep(cc_sweep, f64::from(radial.azimuth_angle_deg))?;
            let dual_pol = &cc_sweep.radials[cc_index].moments;
            let cc = dual_pol.get(&MomentKind::CorrelationCoefficient)?;
            let zdr = dual_pol.get(&MomentKind::DifferentialReflectivity);
            Some(
                reflectivity
                    .gates
                    .iter()
                    .enumerate()
                    .map(|(g, gate)| {
                        let GateValue::Value(dbz) = *gate else {
                            return false;
                        };
                        if dbz >= ALWAYS_SHOW_DBZ {
                            return false;
                        }
                        let range_km = f64::from(reflectivity.first_gate_range_km)
                            + g as f64 * f64::from(reflectivity.gate_spacing_km);
                        let at = |m: &Moment| match find_gate_index(m, range_km)
                            .and_then(|i| m.gates.get(i))
                        {
                            Some(GateValue::Value(v)) => Some(*v),
                            _ => None,
                        };
                        let low_cc = at(cc).is_some_and(|c| c < MIN_WEATHER_CC);
                        let insect_zdr = dbz < INSECT_TEST_MAX_DBZ
                            && zdr.and_then(at).is_some_and(|d| d > INSECT_MIN_ZDR_DB);
                        low_cc || insect_zdr
                    })
                    .collect(),
            )
        })
        .collect();

    for (radial, mask) in volume.sweeps[sweep_index].radials.iter_mut().zip(masks) {
        if let (Some(m), Some(mask)) = (radial.moments.get_mut(&MomentKind::Reflectivity), mask) {
            for (gate, hide) in m.gates.iter_mut().zip(mask) {
                if hide {
                    *gate = GateValue::Missing;
                }
            }
        }
    }
    true
}

fn carries(sweep: &Sweep, kind: MomentKind) -> bool {
    sweep.radials.iter().any(|r| r.moments.contains_key(&kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{AzimuthResolution, Radial, RadialStatus, RadialStatusKind, Site, Timestamp};
    use std::collections::BTreeMap;

    fn moment(values: &[GateValue]) -> Moment {
        Moment {
            first_gate_range_km: 2.125,
            gate_spacing_km: 0.25,
            scale: 2.0,
            offset: 66.0,
            gates: values.to_vec(),
        }
    }

    fn sweep(elevation: f32, moments: &[(MomentKind, Vec<GateValue>)]) -> Sweep {
        let radials = (0..360)
            .map(|az| Radial {
                azimuth_number: az as u16 + 1,
                azimuth_angle_deg: az as f32 + 0.5,
                azimuth_resolution: AzimuthResolution::One,
                elevation_angle_deg: elevation,
                radial_status: RadialStatus {
                    kind: RadialStatusKind::Intermediate,
                    bad_data: false,
                },
                collection_time: Timestamp::from_epoch_millis(0),
                nyquist_velocity_mps: None,
                moments: moments
                    .iter()
                    .map(|(k, v)| (*k, moment(v)))
                    .collect::<BTreeMap<_, _>>(),
            })
            .collect();
        Sweep {
            elevation_number: 1,
            elevation_angle_deg: elevation,
            radials,
        }
    }

    fn volume(sweeps: Vec<Sweep>) -> Volume {
        Volume {
            site: Site {
                icao: "KTST".into(),
                latitude_deg: 35.0,
                longitude_deg: -97.0,
                height_m: 300.0,
            },
            start_time: Timestamp::from_epoch_millis(0),
            volume_coverage_pattern: 212,
            sweeps,
        }
    }

    use GateValue::{Missing, Value};

    /// Gates: insects (10 dBZ, CC 0.5), light snow (10 dBZ, CC 0.98, ZDR
    /// 0.5), hail or debris (45 dBZ, CC 0.5), weak echo with no dual-pol
    /// reading, a gap, high-CC insects (10 dBZ, CC 0.95, ZDR 7), and
    /// size-sorted big drops at a storm edge (32 dBZ, ZDR 4.5).
    fn reflectivity() -> Vec<GateValue> {
        vec![
            Value(10.0),
            Value(10.0),
            Value(45.0),
            Value(10.0),
            Missing,
            Value(10.0),
            Value(32.0),
        ]
    }
    fn cc() -> Vec<GateValue> {
        vec![
            Value(0.5),
            Value(0.98),
            Value(0.5),
            Missing,
            Missing,
            Value(0.95),
            Value(0.97),
        ]
    }
    fn zdr() -> Vec<GateValue> {
        vec![
            Value(6.0),
            Value(0.5),
            Value(0.0),
            Missing,
            Missing,
            Value(7.0),
            Value(4.5),
        ]
    }
    const FILTERED: [GateValue; 7] = [
        Missing,
        Value(10.0),
        Value(45.0),
        Value(10.0),
        Missing,
        Missing,
        Value(32.0),
    ];

    fn ref_gates(v: &Volume, sweep: usize) -> Vec<GateValue> {
        v.sweeps[sweep].radials[0].moments[&MomentKind::Reflectivity]
            .gates
            .clone()
    }

    #[test]
    fn hides_only_low_cc_weak_echo_on_the_same_sweep() {
        let mut v = volume(vec![sweep(
            0.5,
            &[
                (MomentKind::Reflectivity, reflectivity()),
                (MomentKind::CorrelationCoefficient, cc()),
                (MomentKind::DifferentialReflectivity, zdr()),
            ],
        )]);
        assert!(hide_non_weather_reflectivity(&mut v, 0));
        assert_eq!(ref_gates(&v, 0), FILTERED);
    }

    #[test]
    fn split_cut_borrows_cc_from_the_matching_elevation() {
        let mut v = volume(vec![
            sweep(
                0.48,
                &[
                    (MomentKind::Reflectivity, reflectivity()),
                    (MomentKind::CorrelationCoefficient, cc()),
                    (MomentKind::DifferentialReflectivity, zdr()),
                ],
            ),
            sweep(
                0.5,
                &[
                    (MomentKind::Reflectivity, reflectivity()),
                    (MomentKind::Velocity, reflectivity()),
                ],
            ),
        ]);
        assert!(hide_non_weather_reflectivity(&mut v, 1));
        assert_eq!(ref_gates(&v, 1), FILTERED);
    }

    #[test]
    fn no_cc_near_this_elevation_changes_nothing() {
        let mut v = volume(vec![
            sweep(1.5, &[(MomentKind::CorrelationCoefficient, cc())]),
            sweep(0.5, &[(MomentKind::Reflectivity, reflectivity())]),
        ]);
        assert!(!hide_non_weather_reflectivity(&mut v, 1));
        assert_eq!(ref_gates(&v, 1), reflectivity());
    }
}
