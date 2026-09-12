//! CPU-side construction of the 1D radial-index lookup table: for each of
//! `texel_count` azimuth buckets spanning 0-360 degrees, the index (into a
//! [`crate::sweep_buffers::SweepBufferData`]'s `radial_meta`) of the radial
//! whose azimuth is closest to that bucket's center, or [`SENTINEL_NO_RADIAL`]
//! if no radial is close enough.
//!
//! This is the second half of the storage-buffer + lookup-texture hybrid
//! (see the crate-level docs): it gives the shader O(1) azimuth -> radial
//! resolution (`textureLoad` at a bucket index) instead of scanning every
//! radial per pixel.
//!
//! No `wgpu` dependency — this builds a plain `Vec<u32>`, uploaded to a
//! `R32Uint` 1D texture by `gpu.rs`. Fully unit-testable without a GPU.

use radar_types::{AzimuthResolution, Radial};

/// Sentinel value for a lookup-table entry with no matching radial within
/// tolerance — a gap in a partial scan, or a genuine dropped-radial gap
/// wider than the tolerance derived from the sweep's azimuth resolution.
/// The shader must check for this exact value and render that pixel as
/// no-data, never fall through to treating it as "radial 0".
pub const SENTINEL_NO_RADIAL: u32 = u32::MAX;

/// Number of azimuth buckets in the lookup table: one texel per 0.1
/// degrees of azimuth (360.0 / 3600 = 0.1). 0.1 degrees is five times
/// finer than WSR-88D's finest native azimuth resolution (0.5 degrees,
/// [`AzimuthResolution::Half`]), so no on-screen bucket boundary should be
/// coarser than the radar's own angular resolution; finer than that buys
/// no additional real resolution and only costs texture memory (3600
/// texels x 4 bytes = 14.4 KiB, negligible either way).
pub const DEFAULT_LOOKUP_TEXEL_COUNT: u32 = 3600;

/// Build the radial-index lookup table for `radials` (which should be a
/// [`crate::sweep_buffers::SweepBufferData::source_radials`] — i.e.
/// already filtered to the radials that carry the moment being rendered,
/// in the same order as the corresponding `radial_meta`, since table
/// entries are indices into that array), using `azimuth_resolution` to
/// derive the matching tolerance.
///
/// Reuses [`radar_geo::find_radial_index`] directly — the same "nearest
/// radial by circular azimuth distance" convention already documented and
/// tested there — rather than reimplementing azimuth matching here, per
/// `GLOBAL_CONTRACT.md`'s "reuse `radar-geo`'s azimuth/gate math, don't
/// reinvent it".
///
/// The tolerance passed to `find_radial_index` is `2.0 *
/// azimuth_resolution.degrees()`, mirroring
/// [`radar_geo::find_radial_index_for_sweep`]'s own derivation (that
/// convenience takes a whole `Sweep`, which this module does not have —
/// it only has the already-filtered radial slice — so the same tolerance
/// rule is applied directly here instead of going through it).
pub fn build_radial_lookup(
    radials: &[Radial],
    azimuth_resolution: AzimuthResolution,
    texel_count: u32,
) -> Vec<u32> {
    let tolerance_deg = f64::from(azimuth_resolution.degrees()) * 2.0;
    let texel_count_usize = texel_count as usize;
    let mut table = Vec::with_capacity(texel_count_usize);

    for texel in 0..texel_count_usize {
        let bucket_center_deg = (texel as f64 + 0.5) * 360.0 / f64::from(texel_count.max(1));
        let entry = match radar_geo::find_radial_index(radials, bucket_center_deg, tolerance_deg) {
            Some(index) => index as u32,
            None => SENTINEL_NO_RADIAL,
        };
        table.push(entry);
    }

    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{RadialStatus, RadialStatusKind, Timestamp};
    use std::collections::BTreeMap;

    fn radial_at(azimuth_deg: f32) -> Radial {
        Radial {
            azimuth_number: 1,
            azimuth_angle_deg: azimuth_deg,
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

    #[test]
    fn full_360_scan_assigns_nearest_radial_everywhere() {
        // One radial per whole degree, 0..359 — a full, regular scan.
        let radials: Vec<Radial> = (0..360).map(|az| radial_at(az as f32)).collect();
        let table = build_radial_lookup(&radials, AzimuthResolution::One, 3600);

        assert_eq!(table.len(), 3600);
        // Bucket centered on exactly 45.0 degrees (texel 450: center =
        // (450 + 0.5) * 0.1 = 45.05, closest to radial index 45).
        assert_eq!(table[450], 45);
        // No sentinel entries anywhere in a full scan.
        assert!(!table.contains(&SENTINEL_NO_RADIAL));
    }

    #[test]
    fn wraparound_bucket_near_zero_resolves_correctly() {
        // Radials at 358, 359, 0, 1 degrees.
        let radials = vec![
            radial_at(358.0),
            radial_at(359.0),
            radial_at(0.0),
            radial_at(1.0),
        ];
        let table = build_radial_lookup(&radials, AzimuthResolution::One, 3600);

        // Texel 3599 covers [359.9, 360.0), center 359.95 -> nearest is
        // radial index 2 (azimuth 0.0), 0.05 degrees away circularly,
        // versus radial index 1 (azimuth 359.0), 0.95 degrees away.
        assert_eq!(table[3599], 2);
        // Texel 0 covers [0.0, 0.1), center 0.05 -> also nearest radial
        // index 2 (azimuth 0.0 exactly).
        assert_eq!(table[0], 2);
    }

    #[test]
    fn partial_scan_gap_gets_sentinel_outside_coverage() {
        // Radials only from 45 to 135 degrees (a partial scan).
        let radials: Vec<Radial> = (45..=135).map(|az| radial_at(az as f32)).collect();
        let table = build_radial_lookup(&radials, AzimuthResolution::One, 3600);

        // Bucket at 90 degrees (well inside coverage) resolves to a real
        // radial (azimuth 90 -> index 45 in this 45..=135 slice).
        let bucket_90 = (90.0 * 3600.0 / 360.0) as usize;
        assert_ne!(table[bucket_90], SENTINEL_NO_RADIAL);
        assert_eq!(table[bucket_90], 45);

        // Buckets well outside [45, 135] get the sentinel, not radial 0
        // or the nearest in-range radial silently substituted.
        let bucket_0 = 0usize;
        let bucket_200 = (200.0 * 3600.0 / 360.0) as usize;
        assert_eq!(table[bucket_0], SENTINEL_NO_RADIAL);
        assert_eq!(table[bucket_200], SENTINEL_NO_RADIAL);
    }

    #[test]
    fn empty_radials_produces_all_sentinel_table() {
        let table = build_radial_lookup(&[], AzimuthResolution::One, 360);
        assert_eq!(table.len(), 360);
        assert!(table.iter().all(|&entry| entry == SENTINEL_NO_RADIAL));
    }

    #[test]
    fn irregular_spacing_resolves_to_nearest_actual_radial() {
        // Non-uniform gaps, mirroring radar-geo's own irregular-spacing
        // test fixture.
        let radials = vec![
            radial_at(10.0),
            radial_at(11.0),
            radial_at(15.0),
            radial_at(40.0),
        ];
        let table = build_radial_lookup(&radials, AzimuthResolution::One, 3600);

        let bucket_at = |deg: f64| (deg * 3600.0 / 360.0) as usize;
        assert_eq!(table[bucket_at(10.3)], 0);
        assert_eq!(table[bucket_at(14.5)], 2);
        // 27.5 sits in the middle of the 25-degree gap between 15 and 40:
        // both candidates are 12.5 degrees away, past the ~2-degree
        // tolerance derived from `AzimuthResolution::One`.
        assert_eq!(table[bucket_at(27.5)], SENTINEL_NO_RADIAL);
    }
}
