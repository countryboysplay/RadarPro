//! Non-weather filter against real Level II volumes: it must only ever turn
//! weak (< 35 dBZ) REF gates into Missing, and must leave strong echo alone.

use radar_types::{GateValue, MomentKind};
use std::path::PathBuf;

fn load_volume(name: &str) -> radar_types::Volume {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/nexrad-level2")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    nexrad_level2::decode_volume(&bytes).expect("fixture decodes")
}

#[test]
fn filter_hides_only_weak_echo_on_real_volumes() {
    for name in [
        "KTLX20260912_203032_V06",
        "KTLX20240601_000353_V06",
        "KFTG20240601_000116_V06",
    ] {
        let raw = load_volume(name);
        let mut filtered = raw.clone();
        let (mut valid, mut hidden, mut strong) = (0usize, 0usize, 0usize);
        for index in 0..raw.sweeps.len() {
            radar_geo::hide_non_weather_reflectivity(&mut filtered, index);
            for (r, f) in raw.sweeps[index]
                .radials
                .iter()
                .zip(&filtered.sweeps[index].radials)
            {
                let (Some(a), Some(b)) = (
                    r.moments.get(&MomentKind::Reflectivity),
                    f.moments.get(&MomentKind::Reflectivity),
                ) else {
                    continue;
                };
                for (x, y) in a.gates.iter().zip(&b.gates) {
                    match (*x, *y) {
                        (GateValue::Value(v), GateValue::Missing) => {
                            assert!(
                                v < radar_geo::non_weather::ALWAYS_SHOW_DBZ,
                                "{name}: hid {v} dBZ"
                            );
                            valid += 1;
                            hidden += 1;
                        }
                        (GateValue::Value(v), y) => {
                            assert_eq!(y, GateValue::Value(v), "{name}: changed a kept gate");
                            valid += 1;
                            strong += usize::from(v >= radar_geo::non_weather::ALWAYS_SHOW_DBZ);
                        }
                        (x, y) => assert_eq!(x, y, "{name}: non-value gates pass through"),
                    }
                }
            }
        }
        eprintln!(
            "{name}: hid {hidden} of {valid} REF gates ({:.1}%), kept all {strong} >= 35 dBZ",
            100.0 * hidden as f64 / valid as f64
        );
        assert!(hidden > 0, "{name}: expected some non-weather echo");
    }
}
