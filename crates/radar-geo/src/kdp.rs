//! Specific differential phase (KDP): half the range derivative of the
//! differential phase PHI, in deg/km. It rises with liquid water content
//! (heavy rain), is immune to attenuation and calibration, and stays near
//! zero in dry hail -- so a strong-REF core with low KDP points to hail.
//!
//! Per radial, the usual NWS-style estimate:
//!
//! 1. Keep PHI only where CC is at least [`MIN_CC`] (meteorological echo;
//!    clutter and biological targets have noisy, meaningless PHI).
//! 2. Unwrap PHI across the 0/360 seam.
//! 3. Median-filter over [`MEDIAN_GATES`] to knock out single-gate spikes.
//! 4. Least-squares slope over a window centred on each gate: short
//!    ([`SHORT_WINDOW_GATES`]) in heavy rain (REF at or above
//!    [`HEAVY_RAIN_DBZ`]) where KDP changes fast, long
//!    ([`LONG_WINDOW_GATES`]) elsewhere where PHI is noisy. More than half
//!    the window must hold kept PHI.
//!
//! Output only where REF is at least [`MIN_DBZ`]; elsewhere `Missing`.

use radar_types::{GateValue, Moment, MomentKind, Volume};

/// PHI below this CC is ignored.
pub const MIN_CC: f32 = 0.85;
/// No KDP is reported in echo weaker than this (dBZ).
pub const MIN_DBZ: f32 = 20.0;
/// At or above this REF the short window is used.
pub const HEAVY_RAIN_DBZ: f32 = 40.0;
pub const SHORT_WINDOW_GATES: usize = 9;
pub const LONG_WINDOW_GATES: usize = 25;
const MEDIAN_GATES: usize = 5;

/// Add a [`MomentKind::SpecificDifferentialPhase`] moment to every radial
/// that carries PHI and has echo to report (drawing skips radials without it).
pub fn add_kdp(volume: &mut Volume) {
    for sweep in &mut volume.sweeps {
        for radial in &mut sweep.radials {
            let Some(phi) = radial.moments.get(&MomentKind::DifferentialPhase) else {
                continue;
            };
            let Some(kdp) = kdp_moment(
                phi,
                radial.moments.get(&MomentKind::CorrelationCoefficient),
                radial.moments.get(&MomentKind::Reflectivity),
            ) else {
                continue;
            };
            radial
                .moments
                .insert(MomentKind::SpecificDifferentialPhase, kdp);
        }
    }
}

/// The value of `moment` at the gate nearest `range_km`, if it holds one.
fn value_at(moment: Option<&Moment>, range_km: f32) -> Option<f32> {
    let m = moment?;
    let i = ((range_km - m.first_gate_range_km) / m.gate_spacing_km).round();
    if i < 0.0 {
        return None;
    }
    match m.gates.get(i as usize)? {
        GateValue::Value(v) => Some(*v),
        _ => None,
    }
}

/// `None` for a clear-air radial (no echo at [`MIN_DBZ`] or more).
fn kdp_moment(phi: &Moment, cc: Option<&Moment>, reflectivity: Option<&Moment>) -> Option<Moment> {
    let n = phi.gates.len();
    let range = |i: usize| phi.first_gate_range_km + i as f32 * phi.gate_spacing_km;
    let mut out = Moment {
        first_gate_range_km: phi.first_gate_range_km,
        gate_spacing_km: phi.gate_spacing_km,
        scale: 1.0,
        offset: 0.0,
        gates: vec![GateValue::Missing; n],
    };
    // Clear-air radials (most of them) have nothing to report.
    if reflectivity.is_some_and(|z| {
        !z.gates
            .iter()
            .any(|g| matches!(g, GateValue::Value(v) if *v >= MIN_DBZ))
    }) {
        return None;
    }

    // 1-2: kept, unwrapped PHI.
    let mut kept: Vec<Option<f32>> = Vec::with_capacity(n);
    let mut last: Option<f32> = None;
    let mut unwrap = 0.0f32;
    for (i, gate) in phi.gates.iter().enumerate() {
        let GateValue::Value(v) = *gate else {
            kept.push(None);
            continue;
        };
        // No CC on this radial: keep everything (the window rule still rejects noise).
        if cc.is_some() && value_at(cc, range(i)).is_none_or(|c| c < MIN_CC) {
            kept.push(None);
            continue;
        }
        let mut u = v + unwrap;
        if let Some(prev) = last {
            if u - prev > 180.0 {
                unwrap -= 360.0;
                u -= 360.0;
            } else if u - prev < -180.0 {
                unwrap += 360.0;
                u += 360.0;
            }
        }
        last = Some(u);
        kept.push(Some(u));
    }

    // 3: median of the kept neighbours (at least 3 of them).
    let half = MEDIAN_GATES / 2;
    let smoothed: Vec<Option<f32>> = (0..n)
        .map(|i| {
            kept[i]?;
            let mut window = [0.0f32; MEDIAN_GATES];
            let mut len = 0;
            for v in kept[i.saturating_sub(half)..(i + half + 1).min(n)]
                .iter()
                .flatten()
            {
                window[len] = *v;
                len += 1;
            }
            if len < 3 {
                return None;
            }
            let window = &mut window[..len];
            window.sort_unstable_by(f32::total_cmp);
            Some(window[len / 2])
        })
        .collect();

    // 4: windowed least-squares slope from prefix sums (O(1) per gate).
    let mut sums = vec![[0.0f64; 5]; n + 1]; // count, x, y, xy, xx
    for (i, s) in smoothed.iter().enumerate() {
        let mut next = sums[i];
        if let Some(y) = *s {
            let (x, y) = (i as f64, f64::from(y));
            next[0] += 1.0;
            next[1] += x;
            next[2] += y;
            next[3] += x * y;
            next[4] += x * x;
        }
        sums[i + 1] = next;
    }
    out.gates = (0..n)
        .map(|i| {
            if smoothed[i].is_none() {
                return GateValue::Missing;
            }
            let dbz = match reflectivity {
                Some(_) => match value_at(reflectivity, range(i)) {
                    Some(z) if z >= MIN_DBZ => z,
                    _ => return GateValue::Missing,
                },
                None => MIN_DBZ,
            };
            let window = if dbz >= HEAVY_RAIN_DBZ {
                SHORT_WINDOW_GATES
            } else {
                LONG_WINDOW_GATES
            };
            let (lo, hi) = (i.saturating_sub(window / 2), (i + window / 2 + 1).min(n));
            let s: [f64; 5] = std::array::from_fn(|k| sums[hi][k] - sums[lo][k]);
            let count = s[0];
            if count <= (window / 2) as f64 {
                return GateValue::Missing;
            }
            let denominator = count * s[4] - s[1] * s[1];
            if denominator <= 0.0 {
                return GateValue::Missing;
            }
            let slope_deg_per_gate = (count * s[3] - s[1] * s[2]) / denominator;
            GateValue::Value((0.5 * slope_deg_per_gate / f64::from(phi.gate_spacing_km)) as f32)
        })
        .collect();
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moment(values: impl IntoIterator<Item = f32>) -> Moment {
        Moment {
            first_gate_range_km: 2.0,
            gate_spacing_km: 0.25,
            scale: 1.0,
            offset: 0.0,
            gates: values.into_iter().map(GateValue::Value).collect(),
        }
    }

    fn kdp_at(m: &Moment, i: usize) -> f32 {
        match m.gates[i] {
            GateValue::Value(v) => v,
            other => panic!("gate {i} is {other:?}"),
        }
    }

    #[test]
    fn steady_phase_rise_gives_half_its_slope() {
        // PHI rising 1 deg per 0.25 km gate = 4 deg/km, so KDP = 2 deg/km.
        let phi = moment((0..100).map(|i| 60.0 + i as f32));
        let cc = moment(std::iter::repeat_n(0.99, 100));
        let reflectivity = moment(std::iter::repeat_n(45.0, 100));
        let kdp = kdp_moment(&phi, Some(&cc), Some(&reflectivity)).unwrap();
        assert!((kdp_at(&kdp, 50) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn unwraps_the_360_seam() {
        let phi = moment((0..100).map(|i| (340.0 + i as f32) % 360.0));
        let kdp = kdp_moment(&phi, None, None).unwrap();
        assert!((kdp_at(&kdp, 20) - 2.0).abs() < 1e-3, "seam at gate 20");
    }

    #[test]
    fn masks_weak_echo_and_low_cc() {
        let phi = moment((0..100).map(|i| 60.0 + i as f32));
        let mut reflectivity = moment(std::iter::repeat_n(45.0, 100));
        reflectivity.gates[50] = GateValue::Value(10.0);
        let mut cc = moment(std::iter::repeat_n(0.99, 100));
        for g in &mut cc.gates[70..100] {
            *g = GateValue::Value(0.5); // clutter: PHI meaningless
        }
        let kdp = kdp_moment(&phi, Some(&cc), Some(&reflectivity)).unwrap();
        assert_eq!(kdp.gates[50], GateValue::Missing, "below MIN_DBZ");
        assert_eq!(kdp.gates[90], GateValue::Missing, "no kept PHI in window");
    }

    #[test]
    fn a_single_spike_barely_moves_it() {
        let mut phi = moment(std::iter::repeat_n(80.0, 100));
        phi.gates[50] = GateValue::Value(140.0);
        let kdp = kdp_moment(&phi, None, None).unwrap();
        assert!(kdp_at(&kdp, 50).abs() < 0.05);
    }
}
