//! Velocity dealiasing: unfold radial velocities that exceeded the radar's
//! Nyquist velocity (Vn) and wrapped around to the opposite sign.
//!
//! Region-based, after Py-ART's `dealias_region_based`: a gate's true
//! velocity is `v + 2·Vn·k` for some integer `k`, and `k` is constant across
//! any patch of echo with no jump bigger than half a Nyquist interval.
//!
//! 1. Group gates into regions connected (along range and azimuth) by small
//!    velocity steps.
//! 2. Starting from the largest region (k = 0 -- usually the unaliased area
//!    near the radar), fix each neighboring region's `k` from the mean
//!    velocity difference along their shared boundary, strongest boundary
//!    first, so one noisy gate cannot shift a whole region.
//!
//! A region is unfolded by at most one interval, and only if its raw speeds
//! are near ±Vn (see [`MIN_UNFOLD_MEAN_SPEED_FRACTION_OF_NYQUIST`]).
//!
//! shortcut: assumes the largest region is unaliased, and never recovers
//! true speeds beyond 1.5·Vn; wrong when most of the sweep is aliased (very
//! strong flow at a low Vn). Upgrade by seeding from a VAD or model wind
//! reference if that shows up on real data.
//!
//! Echo not connected to anything keeps its raw value (k = 0).
//! `Missing`/`RangeFolded` gates, and radials whose Nyquist velocity is
//! missing or differs from the sweep's, are left exactly as decoded.

use std::collections::{BinaryHeap, HashMap};

use radar_types::{GateValue, MomentKind, Sweep};

/// Adjacent gates whose velocities differ by less than this fraction of Vn
/// are taken to share the same Nyquist interval.
const CONTINUITY_FRACTION_OF_NYQUIST: f32 = 0.5;

/// A region is only unfolded if its mean |raw velocity| is above this
/// fraction of Vn: a true speed between Vn and 1.5·Vn aliases to a raw
/// reading near ±Vn, never near zero. Without this, noisy near-zero patches
/// get pulled a whole interval by noisy boundaries (seen on real KFTG data:
/// 5.5 m/s -> 62 m/s).
const MIN_UNFOLD_MEAN_SPEED_FRACTION_OF_NYQUIST: f64 = 0.5;

/// Radials whose Nyquist velocity is within this of the sweep's are
/// dealiased together (WSR-88D keeps Vn constant within a cut).
const NYQUIST_MATCH_TOLERANCE_MPS: f32 = 0.5;

/// Dealias every radial's [`MomentKind::Velocity`] moment in `sweep`, in
/// place. Radials are taken as azimuth neighbors in scan order, wrapping at
/// the end.
pub fn dealias_velocity_sweep(sweep: &mut Sweep) {
    let Some(vn) = sweep_nyquist(sweep) else {
        return;
    };
    let interval = 2.0 * vn;
    let continuity = vn * CONTINUITY_FRACTION_OF_NYQUIST;

    let grid: Vec<Vec<Option<f32>>> = sweep
        .radials
        .iter()
        .map(
            |r| match (r.nyquist_velocity_mps, r.moments.get(&MomentKind::Velocity)) {
                (Some(rv), Some(m)) if (rv - vn).abs() <= NYQUIST_MATCH_TOLERANCE_MPS => m
                    .gates
                    .iter()
                    .map(|g| match *g {
                        GateValue::Value(v) => Some(v),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
        )
        .collect();
    let n = grid.len();
    let mut offsets = Vec::with_capacity(n);
    let mut total = 0;
    for gates in &grid {
        offsets.push(total);
        total += gates.len();
    }
    let value = |i: usize, g: usize| grid[i].get(g).copied().flatten();

    // Every pair of neighboring valid gates: (gate a, gate b, v_b - v_a).
    let mut pairs = Vec::new();
    for i in 0..n {
        let next = (i + 1) % n;
        for g in 0..grid[i].len() {
            let Some(v) = value(i, g) else { continue };
            if let Some(w) = value(i, g + 1) {
                pairs.push((offsets[i] + g, offsets[i] + g + 1, w - v));
            }
            if next != i {
                if let Some(w) = value(next, g) {
                    pairs.push((offsets[i] + g, offsets[next] + g, w - v));
                }
            }
        }
    }

    // 1. Regions: union-find over small-step neighbor pairs.
    let mut parent: Vec<usize> = (0..total).collect();
    for &(a, b, d) in &pairs {
        if d.abs() < continuity {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            parent[ra] = rb;
        }
    }
    let mut region_of = vec![usize::MAX; total];
    let mut sizes: Vec<usize> = Vec::new();
    let mut abs_sums: Vec<f64> = Vec::new();
    let mut root_region: HashMap<usize, usize> = HashMap::new();
    for i in 0..n {
        for g in 0..grid[i].len() {
            if value(i, g).is_some() {
                let id = offsets[i] + g;
                let root = find(&mut parent, id);
                let r = *root_region.entry(root).or_insert_with(|| {
                    sizes.push(0);
                    abs_sums.push(0.0);
                    sizes.len() - 1
                });
                sizes[r] += 1;
                abs_sums[r] += f64::from(value(i, g).unwrap_or(0.0).abs());
                region_of[id] = r;
            }
        }
    }

    // Boundary between two regions: sum of (v_b - v_a) over edges, count.
    let mut boundaries: HashMap<(usize, usize), (f64, u32)> = HashMap::new();
    for &(a, b, d) in &pairs {
        let (ra, rb) = (region_of[a], region_of[b]);
        if ra != rb {
            let e = boundaries.entry((ra, rb)).or_default();
            e.0 += f64::from(d);
            e.1 += 1;
        }
    }
    // Per region: (neighbor, Σ(v_neighbor - v_self), edge count).
    let mut adjacent: Vec<Vec<(usize, f64, u32)>> = vec![Vec::new(); sizes.len()];
    for (&(ra, rb), &(sum, count)) in &boundaries {
        adjacent[ra].push((rb, sum, count));
        adjacent[rb].push((ra, -sum, count));
    }

    // 2. Fix each region's k, strongest boundary to already-fixed regions
    // first.
    let interval64 = f64::from(interval);
    let mut k: Vec<Option<f64>> = vec![None; sizes.len()];
    // Per unfixed region: Σ(v_fixed_unfolded - v_self) over edges to fixed
    // regions, and the edge count.
    let mut pull: Vec<(f64, u32)> = vec![(0.0, 0); sizes.len()];
    let mut heap = BinaryHeap::new();
    let mut by_size: Vec<usize> = (0..sizes.len()).collect();
    by_size.sort_by(|a, b| sizes[*b].cmp(&sizes[*a]));
    for seed in by_size {
        if k[seed].is_some() {
            continue;
        }
        let mut next = Some((seed, 0.0));
        while let Some((r, kr)) = next.take() {
            k[r] = Some(kr);
            for &(other, sum, count) in &adjacent[r] {
                if k[other].is_none() {
                    pull[other].0 += -sum + f64::from(count) * interval64 * kr;
                    pull[other].1 += count;
                    heap.push((pull[other].1, other));
                }
            }
            // Stale heap entries (region since fixed, or its pull grew) are skipped.
            while let Some((count, other)) = heap.pop() {
                if k[other].is_none() && pull[other].1 == count {
                    let mean = pull[other].0 / f64::from(count);
                    let mean_speed = abs_sums[other] / sizes[other] as f64;
                    let kr = (mean / interval64).round().clamp(-1.0, 1.0);
                    let near_nyquist =
                        mean_speed > f64::from(vn) * MIN_UNFOLD_MEAN_SPEED_FRACTION_OF_NYQUIST;
                    next = Some((other, if near_nyquist { kr } else { 0.0 }));
                    break;
                }
            }
        }
    }

    for (i, radial) in sweep.radials.iter_mut().enumerate() {
        if grid[i].is_empty() {
            continue;
        }
        if let Some(m) = radial.moments.get_mut(&MomentKind::Velocity) {
            for (g, gate) in m.gates.iter_mut().enumerate() {
                if let GateValue::Value(v) = *gate {
                    let kr = k[region_of[offsets[i] + g]].unwrap_or(0.0);
                    let unfolded = v + interval * kr as f32;
                    // Same 1.5·Vn ceiling per gate: a noisy near-zero gate
                    // inside an aliased region stays raw.
                    if unfolded.abs()
                        <= vn * (1.0 + MIN_UNFOLD_MEAN_SPEED_FRACTION_OF_NYQUIST as f32)
                    {
                        *gate = GateValue::Value(unfolded);
                    }
                }
            }
        }
    }
}

/// The Nyquist velocity most of `sweep`'s VEL radials share (the median),
/// or `None` if no radial has one.
fn sweep_nyquist(sweep: &Sweep) -> Option<f32> {
    let mut values: Vec<f32> = sweep
        .radials
        .iter()
        .filter(|r| r.moments.contains_key(&MomentKind::Velocity))
        .filter_map(|r| r.nyquist_velocity_mps)
        .filter(|v| *v > 0.0)
        .collect();
    values.sort_by(f32::total_cmp);
    values.get(values.len() / 2).copied()
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{
        AzimuthResolution, Moment, Radial, RadialStatus, RadialStatusKind, Timestamp,
    };
    use std::collections::BTreeMap;

    const VN: f32 = 25.0;

    fn fold(v: f32) -> f32 {
        v - 2.0 * VN * (v / (2.0 * VN)).round()
    }

    fn radial(azimuth: f32, nyquist: Option<f32>, gates: Vec<GateValue>) -> Radial {
        let mut moments = BTreeMap::new();
        moments.insert(
            MomentKind::Velocity,
            Moment {
                first_gate_range_km: 2.0,
                gate_spacing_km: 0.25,
                scale: 2.0,
                offset: 129.0,
                gates,
            },
        );
        Radial {
            azimuth_number: 1,
            azimuth_angle_deg: azimuth,
            azimuth_resolution: AzimuthResolution::One,
            elevation_angle_deg: 0.5,
            radial_status: RadialStatus {
                kind: RadialStatusKind::Intermediate,
                bad_data: false,
            },
            collection_time: Timestamp::from_epoch_millis(0),
            nyquist_velocity_mps: nyquist,
            moments,
        }
    }

    fn sweep(radials: Vec<Radial>) -> Sweep {
        Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials,
        }
    }

    fn values(sweep: &Sweep, i: usize) -> Vec<GateValue> {
        sweep.radials[i].moments[&MomentKind::Velocity]
            .gates
            .clone()
    }

    /// Outbound flow on one side of the radar, inbound on the other, peaking
    /// at ±36 m/s far from the radar (aliased at Vn = 25 there, within the
    /// 1.5·Vn recovery ceiling).
    fn true_velocity(i: usize, g: usize) -> f32 {
        let azimuth = (i as f32 * 10.0).to_radians();
        g as f32 * 0.36 * azimuth.cos()
    }

    #[test]
    fn recovers_an_aliased_smooth_field() {
        let mut s = sweep(
            (0..36)
                .map(|i| {
                    let gates = (0..100)
                        .map(|g| GateValue::Value(fold(true_velocity(i, g))))
                        .collect();
                    radial(i as f32 * 10.0, Some(VN), gates)
                })
                .collect(),
        );
        dealias_velocity_sweep(&mut s);
        for i in 0..36 {
            for (g, gate) in values(&s, i).into_iter().enumerate() {
                let GateValue::Value(v) = gate else {
                    panic!("gate lost")
                };
                assert!(
                    (v - true_velocity(i, g)).abs() < 1e-3,
                    "radial {i} gate {g}: got {v}, want {}",
                    true_velocity(i, g)
                );
            }
        }
    }

    #[test]
    fn unaliased_field_is_unchanged() {
        let gates: Vec<GateValue> = (0..50)
            .map(|g| GateValue::Value(-10.0 + g as f32 * 0.3))
            .collect();
        let mut s = sweep(vec![
            radial(0.0, Some(VN), gates.clone()),
            radial(1.0, Some(VN), gates.clone()),
        ]);
        dealias_velocity_sweep(&mut s);
        assert_eq!(values(&s, 0), gates);
        assert_eq!(values(&s, 1), gates);
    }

    #[test]
    fn missing_folded_isolated_and_no_nyquist_gates_pass_through() {
        let gates = vec![
            GateValue::Value(20.0),
            GateValue::Missing,
            GateValue::RangeFolded,
            GateValue::Value(-24.0), // isolated: nothing to unfold against
        ];
        let mut s = sweep(vec![radial(0.0, None, gates.clone())]);
        dealias_velocity_sweep(&mut s);
        assert_eq!(values(&s, 0), gates);

        let mut s = sweep(vec![radial(0.0, Some(VN), gates.clone())]);
        dealias_velocity_sweep(&mut s);
        assert_eq!(values(&s, 0), gates);
    }

    #[test]
    fn a_jump_across_a_fold_unfolds_the_smaller_side() {
        // 20 gates near +24 m/s, then 5 gates past Vn that read as ~-24.
        let mut gates: Vec<GateValue> = (0..20).map(|_| GateValue::Value(24.0)).collect();
        gates.extend((0..5).map(|_| GateValue::Value(fold(27.0))));
        let mut s = sweep(vec![radial(0.0, Some(VN), gates)]);
        dealias_velocity_sweep(&mut s);
        let got = values(&s, 0);
        assert_eq!(got[19], GateValue::Value(24.0));
        assert_eq!(got[24], GateValue::Value(27.0));
    }
}
