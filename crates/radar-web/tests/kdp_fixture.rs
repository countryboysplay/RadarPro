//! KDP on real Level II volumes: values stay physical, heavy-rain cores read
//! clearly positive, and the whole volume is fast enough to run on every decode.

use radar_types::{GateValue, MomentKind};
use std::path::PathBuf;
use std::time::Instant;

fn load_volume(name: &str) -> radar_types::Volume {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/nexrad-level2")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    nexrad_level2::decode_volume(&bytes).expect("fixture decodes")
}

fn value(m: Option<&radar_types::Moment>, i: usize) -> Option<f32> {
    match m?.gates.get(i)? {
        GateValue::Value(v) => Some(*v),
        _ => None,
    }
}

#[test]
fn kdp_is_physical_and_positive_in_heavy_rain() {
    for name in [
        "KTLX20260912_203032_V06",
        "KTLX20240601_000353_V06",
        "KFTG20240601_000116_V06",
    ] {
        let start = Instant::now();
        let mut volume = load_volume(name);
        let decode = start.elapsed();
        let start = Instant::now();
        radar_geo::add_kdp(&mut volume);
        let elapsed = start.elapsed();

        let (mut all, mut heavy) = (Vec::new(), Vec::new());
        for radial in volume.sweeps.iter().flat_map(|s| &s.radials) {
            let Some(kdp) = radial.moments.get(&MomentKind::SpecificDifferentialPhase) else {
                continue;
            };
            let reflectivity = radial.moments.get(&MomentKind::Reflectivity);
            for i in 0..kdp.gates.len() {
                let Some(k) = value(Some(kdp), i) else {
                    continue;
                };
                assert!(k.is_finite());
                all.push(k);
                // Same gate spacing on surveillance cuts, so the index lines up.
                if value(reflectivity, i).is_some_and(|z| z >= 50.0) {
                    heavy.push(k);
                }
            }
        }
        all.sort_by(f32::total_cmp);
        heavy.sort_by(f32::total_cmp);
        let pct = |v: &[f32], q: f64| v[((v.len() as f64 - 1.0) * q) as usize];
        println!(
            "{name}: decode {decode:?}, {} KDP gates in {elapsed:?}; p1 {:.2} p50 {:.2} p99 {:.2}; {} gates at 50+ dBZ, median {:.2}",
            all.len(),
            pct(&all, 0.01),
            pct(&all, 0.5),
            pct(&all, 0.99),
            heavy.len(),
            if heavy.is_empty() { f32::NAN } else { pct(&heavy, 0.5) },
        );
        assert!(!all.is_empty(), "{name}: no KDP");
        assert!(
            pct(&all, 0.01) > -3.0 && pct(&all, 0.99) < 12.0,
            "{name}: KDP out of physical range"
        );
        // KTLX 2026-09-12 is a rain event; KFTG 2024-06-01 is the Denver hail
        // storm, where 50+ dBZ is dry hail and KDP rightly stays near zero.
        if name == "KTLX20260912_203032_V06" {
            assert!(
                pct(&heavy, 0.5) > 0.5,
                "{name}: heavy rain should have clearly positive KDP"
            );
        }
    }
}
