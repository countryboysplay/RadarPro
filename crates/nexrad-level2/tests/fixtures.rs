//! Integration tests against real, ground-truthed NEXRAD Archive II Level
//! II fixtures (`fixtures/nexrad-level2/`). See `fixtures/README.md` for
//! provenance.
//!
//! Per `GLOBAL_CONTRACT.md`/S01: "Never accept visual plausibility as
//! correctness" — these tests check the decoded output against
//! independently known ground truth (site, approximate start time,
//! documented VCP range, documented physical gate-value ranges) rather
//! than merely asserting that decoding succeeds.

use nexrad_level2::decode_volume;
use radar_types::{GateValue, MomentKind, Volume};
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("nexrad-level2")
        .join(name)
}

fn load_fixture(name: &str) -> Vec<u8> {
    let path = fixture_path(name);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
}

/// Assert azimuth angles within a sweep are monotonically non-decreasing
/// modulo 360 degrees: each step forward (wrapping through 360 -> 0 at
/// most once per full rotation) should be a small positive delta, never a
/// large backward jump.
fn assert_azimuths_monotonic_with_wraparound(angles: &[f32]) {
    for pair in angles.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let forward_delta = (b - a).rem_euclid(360.0);
        assert!(
            forward_delta < 10.0,
            "azimuth angle regressed or jumped implausibly: {a} -> {b} (forward delta {forward_delta})"
        );
    }
}

/// Loosely bound a moment's physical values against the documented ICD
/// ranges (Table XVII-I) with generous margin, and require every decoded
/// `Value` to be finite (never NaN/inf).
fn assert_moment_values_in_documented_range(kind: MomentKind, values: &[f32]) {
    let (min, max): (f32, f32) = match kind {
        MomentKind::Reflectivity => (-35.0, 100.0),
        MomentKind::Velocity => (-100.0, 100.0),
        MomentKind::SpectrumWidth => (-5.0, 75.0),
        MomentKind::DifferentialReflectivity => (-15.0, 25.0),
        MomentKind::CorrelationCoefficient => (-0.1, 1.2),
        MomentKind::DifferentialPhase => (-5.0, 365.0),
    };
    for &v in values {
        assert!(v.is_finite(), "{kind:?} gate value is not finite: {v}");
        assert!(
            v >= min && v <= max,
            "{kind:?} gate value {v} outside documented range [{min}, {max}]"
        );
    }
}

/// The filename-derived ground-truth volume start time to check a decoded
/// volume against (see `fixtures/README.md`).
struct ExpectedStartTime {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

fn check_volume_invariants(
    volume: &Volume,
    expected_icao: &str,
    expected_start: ExpectedStartTime,
) {
    assert_eq!(volume.site.icao, expected_icao);

    let civil = volume.start_time.to_civil_utc();
    assert_eq!(civil.year, expected_start.year);
    assert_eq!(civil.month, expected_start.month);
    assert_eq!(civil.day, expected_start.day);
    assert_eq!(civil.hour, expected_start.hour);
    assert_eq!(civil.minute, expected_start.minute);
    let second_delta = (i64::from(civil.second) - i64::from(expected_start.second)).abs();
    assert!(
        second_delta <= 5,
        "volume start second {} not within a few seconds of expected {}",
        civil.second,
        expected_start.second
    );

    assert!(
        volume.volume_coverage_pattern >= 1 && volume.volume_coverage_pattern <= 767,
        "VCP {} outside plausible range 1-767",
        volume.volume_coverage_pattern
    );

    assert!(!volume.sweeps.is_empty(), "volume has no sweeps");

    // Site location should be a real WGS84 coordinate, not zeroed/missing.
    assert!(volume.site.latitude_deg.abs() > 0.1);
    assert!(volume.site.longitude_deg.abs() > 0.1);

    let mut any_missing_ref = false;
    let mut any_value_by_kind: std::collections::HashMap<MomentKind, Vec<f32>> =
        std::collections::HashMap::new();

    for sweep in &volume.sweeps {
        assert!(!sweep.radials.is_empty(), "sweep has no radials");

        let angles: Vec<f32> = sweep.radials.iter().map(|r| r.azimuth_angle_deg).collect();
        assert_azimuths_monotonic_with_wraparound(&angles);

        for radial in &sweep.radials {
            for (&kind, moment) in &radial.moments {
                for gate in &moment.gates {
                    match *gate {
                        GateValue::Missing => {
                            if kind == MomentKind::Reflectivity {
                                any_missing_ref = true;
                            }
                        }
                        GateValue::RangeFolded => {}
                        GateValue::Value(v) => {
                            any_value_by_kind.entry(kind).or_default().push(v);
                        }
                    }
                }
            }
        }
    }

    assert!(
        any_missing_ref,
        "expected at least some Missing REF gates given real-world sparse reflectivity coverage"
    );

    for (kind, values) in &any_value_by_kind {
        assert_moment_values_in_documented_range(*kind, values);
    }

    // REF, VEL, SW should be present somewhere in a real dual-pol volume.
    for expected in [
        MomentKind::Reflectivity,
        MomentKind::Velocity,
        MomentKind::SpectrumWidth,
    ] {
        assert!(
            any_value_by_kind.contains_key(&expected),
            "expected at least one decoded {expected:?} value somewhere in the volume"
        );
    }
}

#[test]
fn decodes_ktlx_fixture_with_correct_ground_truth() {
    let bytes = load_fixture("KTLX20240601_000353_V06");
    let volume = decode_volume(&bytes).expect("KTLX fixture should decode successfully");

    check_volume_invariants(
        &volume,
        "KTLX",
        ExpectedStartTime {
            year: 2024,
            month: 6,
            day: 1,
            hour: 0,
            minute: 3,
            second: 53,
        },
    );
}

#[test]
fn decodes_kftg_fixture_with_correct_ground_truth() {
    let bytes = load_fixture("KFTG20240601_000116_V06");
    let volume = decode_volume(&bytes).expect("KFTG fixture should decode successfully");

    check_volume_invariants(
        &volume,
        "KFTG",
        ExpectedStartTime {
            year: 2024,
            month: 6,
            day: 1,
            hour: 0,
            minute: 1,
            second: 16,
        },
    );
}

/// Real, live WSR-88D data downloaded 2026-09-12 specifically because it
/// contains a Message Type 32 (RDA PRF Data) record — the exact gap that
/// caused `decode_volume` to fail with "unsupported message type 32"
/// against every live scan before that message type (and 33, RDA Log
/// Data) were recognized as fixed-slot legacy metadata messages (see
/// `fixtures/README.md` and `message.rs`'s
/// `is_legacy_metadata_message_type` doc comment). Unlike the two 2024
/// fixtures above, this asserts the same ground-truth ranges over a
/// volume that specifically exercises the new framing path, confirming
/// real REF/VEL/SW sweeps decode correctly end-to-end around it, not just
/// that decoding doesn't error.
#[test]
fn decodes_ktlx_live_fixture_containing_message_type_32() {
    let bytes = load_fixture("KTLX20260912_203032_V06");
    let volume = decode_volume(&bytes)
        .expect("KTLX live fixture (contains message type 32) should decode successfully");

    check_volume_invariants(
        &volume,
        "KTLX",
        ExpectedStartTime {
            year: 2026,
            month: 9,
            day: 12,
            hour: 20,
            minute: 30,
            second: 32,
        },
    );
}

/// Ground truth given in the S01 task brief: the first LDM Compressed
/// Record in `KTLX20240601_000353_V06` decompresses to exactly 325,888
/// bytes (134 x 2432-byte legacy message frames), per the Archive
/// II/User ICD's documented metadata-record size.
#[test]
fn ktlx_first_ldm_record_decompresses_to_documented_metadata_record_size() {
    let bytes = load_fixture("KTLX20240601_000353_V06");

    // Volume Header Record (24 bytes) + 4-byte control word.
    let control_word_offset = 24;
    let control_word = i32::from_be_bytes([
        bytes[control_word_offset],
        bytes[control_word_offset + 1],
        bytes[control_word_offset + 2],
        bytes[control_word_offset + 3],
    ]);
    let size = control_word.unsigned_abs() as usize;
    let compressed_start = control_word_offset + 4;
    let compressed = &bytes[compressed_start..compressed_start + size];

    assert_eq!(&compressed[0..3], b"BZh", "expected bzip2 magic");

    use std::io::Read;
    let mut reader = bzip2_rs::DecoderReader::new(compressed);
    let mut decompressed = Vec::new();
    reader
        .read_to_end(&mut decompressed)
        .expect("valid bzip2 stream");

    assert_eq!(decompressed.len(), 325_888);
}

/// A real, valid bzip2 stream (the KTLX fixture's first LDM record) that
/// has been truncated must fail to decompress with a structured error,
/// not panic and not silently return a short/garbage buffer.
#[test]
fn truncating_a_real_bzip2_stream_is_reported_as_an_error() {
    let bytes = load_fixture("KTLX20240601_000353_V06");

    let control_word_offset = 24;
    let control_word = i32::from_be_bytes([
        bytes[control_word_offset],
        bytes[control_word_offset + 1],
        bytes[control_word_offset + 2],
        bytes[control_word_offset + 3],
    ]);
    let size = control_word.unsigned_abs() as usize;
    let compressed_start = control_word_offset + 4;
    let compressed = &bytes[compressed_start..compressed_start + size];

    // Cut the real compressed stream in half: still starts with a valid
    // bzip2 magic, but is not a complete stream.
    let truncated = &compressed[..compressed.len() / 2];

    use std::io::Read;
    let mut reader = bzip2_rs::DecoderReader::new(truncated);
    let mut decompressed = Vec::new();
    let result = reader.read_to_end(&mut decompressed);

    assert!(
        result.is_err(),
        "truncated bzip2 stream should fail to decompress, not silently succeed"
    );
}

/// Truncating the whole file partway through its first LDM Compressed
/// Record's declared payload must surface as a structured decode error
/// (not a panic), exercising `decode_volume`'s own framing-level bounds
/// check end-to-end on real bytes.
#[test]
fn decode_volume_rejects_file_truncated_mid_first_record() {
    let bytes = load_fixture("KTLX20240601_000353_V06");
    // Keep the volume header and control word, but cut off partway
    // through the first LDM record's compressed payload.
    let truncated = &bytes[..24 + 4 + 100];

    let err = decode_volume(truncated).unwrap_err();
    match err {
        nexrad_level2::DecodeError::TruncatedLdmRecord { .. } => {}
        other => panic!("expected TruncatedLdmRecord, got {other:?}"),
    }
}
