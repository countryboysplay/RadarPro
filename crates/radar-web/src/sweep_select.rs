//! Pure, host-testable sweep-selection logic shared by the browser glue in
//! [`crate::browser`] (wasm32-only).
//!
//! No `wgpu`/`wasm-bindgen`/`web-sys` dependency at all -- this is plain
//! `radar_types` traversal, so it compiles and runs its `#[test]`s on any
//! target (`cargo test -p radar-web` on the host), not just wasm32.

use radar_types::{MomentKind, Volume};

/// Index into `volume.sweeps` of the lowest-elevation sweep that carries at
/// least one radial with `moment`, or `None` if no sweep does.
///
/// Mirrors `radar-render`'s own S03 harness convention
/// (`crates/radar-render/src/bin/harness.rs::pick_lowest_elevation_sweep_with_moment`)
/// exactly, so this crate selects the same sweep the already-proven native
/// path would for the same volume -- deliberately not reimplemented with any
/// different tie-breaking or filtering rule.
pub fn pick_lowest_elevation_sweep_index(volume: &Volume, moment: MomentKind) -> Option<usize> {
    volume
        .sweeps
        .iter()
        .enumerate()
        .filter(|(_, sweep)| {
            sweep
                .radials
                .iter()
                .any(|radial| radial.moments.contains_key(&moment))
        })
        .min_by(|(_, a), (_, b)| {
            a.elevation_angle_deg
                .partial_cmp(&b.elevation_angle_deg)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{
        AzimuthResolution, Radial, RadialStatus, RadialStatusKind, Site, Sweep, Timestamp,
    };
    use std::collections::BTreeMap;

    fn radial_with(moment: Option<MomentKind>) -> Radial {
        let mut moments = BTreeMap::new();
        if let Some(kind) = moment {
            moments.insert(
                kind,
                radar_types::Moment {
                    first_gate_range_km: 1.0,
                    gate_spacing_km: 0.25,
                    scale: 1.0,
                    offset: 0.0,
                    gates: vec![],
                },
            );
        }
        Radial {
            azimuth_number: 1,
            azimuth_angle_deg: 0.0,
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

    fn sweep(elevation_angle_deg: f32, has_ref: bool) -> Sweep {
        Sweep {
            elevation_number: 1,
            elevation_angle_deg,
            radials: vec![radial_with(has_ref.then_some(MomentKind::Reflectivity))],
        }
    }

    fn volume(sweeps: Vec<Sweep>) -> Volume {
        Volume {
            site: Site::new("KTLX", 35.3333, -97.2778, 370.0),
            start_time: Timestamp::from_epoch_millis(0),
            volume_coverage_pattern: 212,
            sweeps,
        }
    }

    #[test]
    fn picks_lowest_elevation_sweep_that_carries_the_moment() {
        let volume = volume(vec![
            sweep(1.5, true),
            sweep(0.5, true),  // lowest elevation with REF
            sweep(0.1, false), // lower elevation, but no REF -- must be skipped
        ]);
        assert_eq!(
            pick_lowest_elevation_sweep_index(&volume, MomentKind::Reflectivity),
            Some(1)
        );
    }

    #[test]
    fn returns_none_when_no_sweep_carries_the_moment() {
        let volume = volume(vec![sweep(0.5, false), sweep(1.5, false)]);
        assert_eq!(
            pick_lowest_elevation_sweep_index(&volume, MomentKind::Reflectivity),
            None
        );
    }

    #[test]
    fn returns_none_for_an_empty_volume() {
        let volume = volume(vec![]);
        assert_eq!(
            pick_lowest_elevation_sweep_index(&volume, MomentKind::Reflectivity),
            None
        );
    }

    #[test]
    fn ignores_moments_other_than_the_requested_one() {
        let volume = volume(vec![sweep(0.5, false)]);
        // The only sweep has a radial, but with no REF moment on it.
        assert_eq!(
            pick_lowest_elevation_sweep_index(&volume, MomentKind::Velocity),
            None
        );
    }
}
