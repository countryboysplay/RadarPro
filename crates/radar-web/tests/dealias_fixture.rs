//! Velocity dealiasing against a real KTLX volume (an active severe day):
//! Nyquist velocity must decode, dealiasing may only shift gates by whole
//! Nyquist intervals, and it must remove most aliasing seams.

use radar_types::{GateValue, MomentKind, Sweep};
use std::path::PathBuf;

fn load_volume(name: &str) -> radar_types::Volume {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/nexrad-level2")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    nexrad_level2::decode_volume(&bytes).expect("fixture decodes")
}

/// Adjacent gate pairs along a radial whose velocity jumps by more than
/// 1.5x the Nyquist velocity -- what an aliasing seam (a ~2·Vn jump) looks
/// like, as opposed to ordinary noise.
fn seams(sweep: &Sweep) -> usize {
    sweep
        .radials
        .iter()
        .filter_map(|r| {
            Some((
                r.nyquist_velocity_mps?,
                r.moments.get(&MomentKind::Velocity)?,
            ))
        })
        .flat_map(|(vn, m)| {
            m.gates.windows(2).filter(move |w| match (w[0], w[1]) {
                (GateValue::Value(a), GateValue::Value(b)) => (a - b).abs() > 1.5 * vn,
                _ => false,
            })
        })
        .count()
}

#[test]
fn dealiasing_real_volumes_only_unfolds_and_removes_most_seams() {
    for name in [
        "KTLX20260912_203032_V06",
        "KTLX20240601_000353_V06",
        "KFTG20240601_000116_V06",
    ] {
        check_volume(name);
    }
}

fn check_volume(name: &str) {
    let volume = load_volume(name);
    let mut raw_seams = 0;
    let mut dealiased_seams = 0;
    let mut changed = 0;
    let started = std::time::Instant::now();
    for sweep in &volume.sweeps {
        if !sweep
            .radials
            .iter()
            .any(|r| r.moments.contains_key(&MomentKind::Velocity))
        {
            continue;
        }
        let mut d = sweep.clone();
        radar_geo::dealias_velocity_sweep(&mut d);
        raw_seams += seams(sweep);
        dealiased_seams += seams(&d);
        for (r, dr) in sweep.radials.iter().zip(&d.radials) {
            let vn = r
                .nyquist_velocity_mps
                .expect("every VEL radial has a RAD block");
            assert!((5.0..60.0).contains(&vn), "implausible Nyquist {vn} m/s");
            let (Some(a), Some(b)) = (
                r.moments.get(&MomentKind::Velocity),
                dr.moments.get(&MomentKind::Velocity),
            ) else {
                continue;
            };
            for (ga, gb) in a.gates.iter().zip(&b.gates) {
                match (*ga, *gb) {
                    (GateValue::Value(x), GateValue::Value(y)) => {
                        let k = (y - x) / (2.0 * vn);
                        assert!(
                            (k - k.round()).abs() < 1e-3,
                            "shift {x} -> {y} is not a Nyquist multiple"
                        );
                        changed += usize::from(k.round() != 0.0);
                    }
                    (x, y) => assert_eq!(x, y, "non-value gates must pass through"),
                }
            }
        }
    }
    eprintln!(
        "{name}: seams raw={raw_seams} dealiased={dealiased_seams} gates unfolded={changed} in {:?}",
        started.elapsed()
    );
    assert!(
        changed > 0,
        "a severe-weather volume should have some aliased gates"
    );
    assert!(
        dealiased_seams < raw_seams / 2,
        "dealiasing should remove most seams"
    );
}
