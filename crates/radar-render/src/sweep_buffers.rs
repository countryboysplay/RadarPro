//! CPU-side construction of the GPU storage-buffer representation for one
//! sweep's one moment: a flat [`GpuRadialMeta`] array (one entry per radial
//! that carries the requested moment) plus a flat [`GpuGateSample`] array
//! holding every one of those radials' gates back-to-back.
//!
//! This module has no `wgpu` dependency and performs no GPU I/O — it only
//! builds plain, `bytemuck`-castable Rust structs from a
//! [`radar_types::Sweep`], so it is fully unit-testable on a machine with
//! no GPU (see the `tests` module below and `lookup_texture.rs`/
//! `palette.rs`, which follow the same split).
//!
//! # Why a storage buffer, and why pre-convert to a float + flag
//!
//! `radar_types::GateValue` already preserves "missing" (wire value 0) and
//! "range-folded" (wire value 1) as distinct states rather than collapsing
//! them into a numeric placeholder (`GLOBAL_CONTRACT.md`: "preserve missing
//! values, range folding ... never smooth/interpolate radar in ways that
//! create false meteorological structure"). This module preserves that same
//! three-way distinction on the GPU side as [`GateFlag`]: the shader must
//! branch on the flag *before* ever treating `value` as a physical
//! reading, so a missing/range-folded gate can never be silently sampled
//! through the palette as if it were valid data.
//!
//! Converting raw wire values to physical units here (on the CPU, once,
//! at buffer-build time) rather than shipping the raw `u8` and the
//! per-moment scale/offset to the shader was chosen for two reasons: it
//! keeps the WGSL fragment shader free of per-gate branching on scale/
//! offset math (only a flag check), and it keeps the scale/offset
//! conversion itself in ordinary, easily-unit-tested Rust rather than
//! WGSL. The scale/offset metadata is not lost — [`radar_types::Moment`]
//! still carries it upstream of this conversion — it is simply resolved
//! before upload rather than deferred to the shader, which is one of the
//! two options the S03 stage doc explicitly allows.

use radar_types::{GateValue, MomentKind, Radial, Sweep};

/// Marks a [`GpuGateSample`]'s `flag` field: the gate held a valid,
/// converted physical value in `value`.
pub const GATE_FLAG_VALID: u32 = 0;
/// Marks a [`GpuGateSample`]'s `flag` field: the gate was below the
/// signal threshold (wire value 0, [`GateValue::Missing`]). `value` is
/// `0.0` but must never be read as a physical value.
pub const GATE_FLAG_MISSING: u32 = 1;
/// Marks a [`GpuGateSample`]'s `flag` field: the gate was range-folded
/// (wire value 1, [`GateValue::RangeFolded`], ambiguous range). `value` is
/// `0.0` but must never be read as a physical value.
pub const GATE_FLAG_RANGE_FOLDED: u32 = 2;

/// One gate's GPU-side sample: a physical value (meaningful only when
/// `flag == GATE_FLAG_VALID`) plus the flag that preserves
/// missing/range-folded as distinguishable-from-valid-data states.
///
/// `#[repr(C)]` with two 4-byte fields packs to 8 bytes with no compiler-
/// inserted padding, which is also the natural WGSL storage-buffer stride
/// for a `struct { value: f32, flag: u32 }` — the Rust and WGSL layouts
/// agree without needing explicit padding fields.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuGateSample {
    pub value: f32,
    pub flag: u32,
}

impl GpuGateSample {
    fn from_gate_value(gate: GateValue) -> Self {
        match gate {
            GateValue::Missing => GpuGateSample {
                value: 0.0,
                flag: GATE_FLAG_MISSING,
            },
            GateValue::RangeFolded => GpuGateSample {
                value: 0.0,
                flag: GATE_FLAG_RANGE_FOLDED,
            },
            GateValue::Value(v) => GpuGateSample {
                value: v,
                flag: GATE_FLAG_VALID,
            },
        }
    }
}

/// One radial's GPU-side metadata: enough for the shader to redo
/// `radar_geo::lookup::find_gate_index`'s half-open-interval gate math
/// itself, plus `gate_offset` into the flat [`GpuGateSample`] array
/// shared by every radial in a [`SweepBufferData`].
///
/// 24 bytes (six 4-byte fields), no padding needed: every field is
/// already 4-byte-aligned, so `#[repr(C)]` lays this out identically to a
/// WGSL `struct` of the same field order/types.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuRadialMeta {
    /// Azimuth angle in degrees, clockwise from true north — copied
    /// unchanged from [`radar_types::Radial::azimuth_angle_deg`].
    pub azimuth_deg: f32,
    /// Elevation angle in degrees for this specific radial — copied
    /// unchanged from [`radar_types::Radial::elevation_angle_deg`]. Not
    /// used by the S03 shader (which treats the sweep as a flat plane;
    /// see `render.rs`), but retained since it is part of this radial's
    /// identity and a later stage may need it (e.g. per-radial beam
    /// height).
    pub elevation_angle_deg: f32,
    /// Range from the radar to gate 0's *center*, in kilometers — copied
    /// unchanged from [`radar_types::Moment::first_gate_range_km`].
    pub first_gate_range_km: f32,
    /// Distance between successive gate centers, in kilometers — copied
    /// unchanged from [`radar_types::Moment::gate_spacing_km`].
    pub gate_spacing_km: f32,
    /// Number of gates this radial contributes to the shared gate-sample
    /// array, starting at `gate_offset`.
    pub gate_count: u32,
    /// Index into [`SweepBufferData::gate_samples`] where this radial's
    /// gates begin.
    pub gate_offset: u32,
}

/// The GPU storage-buffer representation for one sweep's one moment: one
/// [`GpuRadialMeta`] per radial that carries `moment_kind`, in the same
/// relative order as the sweep's own `radials` (never reordered — radial
/// spacing may be irregular and a partial scan may skip azimuths, but
/// scan order among the radials that *are* present is preserved), plus
/// every one of those radials' gates concatenated into one flat array.
///
/// Radials that do not carry `moment_kind` at all (a genuinely missing
/// radial, not merely a radial with all-missing gates) are simply absent
/// from `radials` — see the module docs' "irregular radial
/// spacing/missing/partial radials" requirement. [`crate::lookup_texture`]
/// is built from this same filtered list, so an azimuth bucket falling in
/// such a gap correctly finds no matching radial rather than an
/// out-of-range or wrong index.
#[derive(Debug, Clone, PartialEq)]
pub struct SweepBufferData {
    /// Original sweep's radials, filtered to only those carrying
    /// `moment_kind`, in original scan order. Kept alongside the GPU
    /// buffers (not just their indices) because [`crate::lookup_texture`]
    /// needs the actual azimuth angles to build its lookup table, and
    /// keeping the association here (rather than recomputing the filter
    /// twice) avoids the two buffers silently drifting out of sync.
    pub source_radials: Vec<Radial>,
    pub radial_meta: Vec<GpuRadialMeta>,
    pub gate_samples: Vec<GpuGateSample>,
}

/// Build the GPU storage-buffer representation for `sweep`'s `moment_kind`
/// moment. Returns an empty [`SweepBufferData`] (all three fields empty)
/// if no radial in `sweep` carries `moment_kind` — the caller decides
/// whether that is an error worth reporting.
pub fn build_sweep_buffers(sweep: &Sweep, moment_kind: MomentKind) -> SweepBufferData {
    let mut source_radials = Vec::new();
    let mut radial_meta = Vec::with_capacity(sweep.radials.len());
    let mut gate_samples = Vec::new();

    for radial in &sweep.radials {
        let Some(moment) = radial.moments.get(&moment_kind) else {
            continue;
        };

        let gate_offset = gate_samples.len() as u32;
        gate_samples.extend(
            moment
                .gates
                .iter()
                .map(|gate| GpuGateSample::from_gate_value(*gate)),
        );

        radial_meta.push(GpuRadialMeta {
            azimuth_deg: radial.azimuth_angle_deg,
            elevation_angle_deg: radial.elevation_angle_deg,
            first_gate_range_km: moment.first_gate_range_km,
            gate_spacing_km: moment.gate_spacing_km,
            gate_count: moment.gates.len() as u32,
            gate_offset,
        });
        source_radials.push(radial.clone());
    }

    SweepBufferData {
        source_radials,
        radial_meta,
        gate_samples,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{AzimuthResolution, RadialStatus, RadialStatusKind, Timestamp};
    use std::collections::BTreeMap;

    fn radial_with_ref(
        azimuth_deg: f32,
        gates: Vec<GateValue>,
        first_gate_range_km: f32,
        gate_spacing_km: f32,
    ) -> Radial {
        let mut moments = BTreeMap::new();
        moments.insert(
            MomentKind::Reflectivity,
            radar_types::Moment {
                first_gate_range_km,
                gate_spacing_km,
                scale: 1.0,
                offset: 0.0,
                gates,
            },
        );
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
            moments,
        }
    }

    fn radial_without_moments(azimuth_deg: f32) -> Radial {
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
    fn gate_flags_survive_conversion_distinctly() {
        let gates = vec![
            GateValue::Missing,
            GateValue::RangeFolded,
            GateValue::Value(23.5),
        ];
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![radial_with_ref(0.0, gates, 1.0, 0.25)],
        };

        let built = build_sweep_buffers(&sweep, MomentKind::Reflectivity);

        assert_eq!(built.gate_samples.len(), 3);
        assert_eq!(built.gate_samples[0].flag, GATE_FLAG_MISSING);
        assert_eq!(built.gate_samples[1].flag, GATE_FLAG_RANGE_FOLDED);
        assert_eq!(built.gate_samples[2].flag, GATE_FLAG_VALID);
        assert_eq!(built.gate_samples[2].value, 23.5);
        // Missing/range-folded carry a placeholder 0.0 value, but it is
        // never reachable as a value because `flag` is checked first —
        // this assertion just documents that the placeholder itself is
        // not a plausible-looking fabricated reading like a real 0 dBZ.
        assert_eq!(built.gate_samples[0].value, 0.0);
        assert_eq!(built.gate_samples[1].value, 0.0);
    }

    #[test]
    fn radials_missing_the_moment_are_excluded_not_zero_filled() {
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![
                radial_with_ref(0.0, vec![GateValue::Value(10.0)], 1.0, 0.25),
                radial_without_moments(1.0),
                radial_with_ref(2.0, vec![GateValue::Value(20.0)], 1.0, 0.25),
            ],
        };

        let built = build_sweep_buffers(&sweep, MomentKind::Reflectivity);

        assert_eq!(built.radial_meta.len(), 2);
        assert_eq!(built.source_radials.len(), 2);
        assert_eq!(built.radial_meta[0].azimuth_deg, 0.0);
        assert_eq!(built.radial_meta[1].azimuth_deg, 2.0);
    }

    #[test]
    fn gate_offsets_index_into_shared_flat_array_correctly() {
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![
                radial_with_ref(
                    0.0,
                    vec![GateValue::Value(1.0), GateValue::Value(2.0)],
                    1.0,
                    0.25,
                ),
                radial_with_ref(
                    1.0,
                    vec![
                        GateValue::Value(3.0),
                        GateValue::Value(4.0),
                        GateValue::Value(5.0),
                    ],
                    1.0,
                    0.25,
                ),
            ],
        };

        let built = build_sweep_buffers(&sweep, MomentKind::Reflectivity);

        assert_eq!(built.radial_meta[0].gate_offset, 0);
        assert_eq!(built.radial_meta[0].gate_count, 2);
        assert_eq!(built.radial_meta[1].gate_offset, 2);
        assert_eq!(built.radial_meta[1].gate_count, 3);
        assert_eq!(built.gate_samples.len(), 5);

        let second_radial_first_gate =
            built.gate_samples[built.radial_meta[1].gate_offset as usize];
        assert_eq!(second_radial_first_gate.value, 3.0);
    }

    #[test]
    fn empty_sweep_produces_empty_buffers() {
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![],
        };
        let built = build_sweep_buffers(&sweep, MomentKind::Reflectivity);
        assert!(built.radial_meta.is_empty());
        assert!(built.gate_samples.is_empty());
        assert!(built.source_radials.is_empty());
    }

    #[test]
    fn sweep_with_no_matching_moment_produces_empty_buffers() {
        let sweep = Sweep {
            elevation_number: 1,
            elevation_angle_deg: 0.5,
            radials: vec![radial_without_moments(0.0), radial_without_moments(1.0)],
        };
        let built = build_sweep_buffers(&sweep, MomentKind::Reflectivity);
        assert!(built.radial_meta.is_empty());
        assert!(built.gate_samples.is_empty());
    }

    #[test]
    fn gpu_structs_have_expected_pod_sizes() {
        // Documents the layout claims in the doc comments above and
        // guards against an accidental field addition silently changing
        // the WGSL-visible stride.
        assert_eq!(std::mem::size_of::<GpuGateSample>(), 8);
        assert_eq!(std::mem::size_of::<GpuRadialMeta>(), 24);
    }
}
