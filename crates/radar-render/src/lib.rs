//! `radar-render` — S03 GPU radar renderer prototype/measurement crate.
//!
//! This crate is deliberately a **prototype**, not a finished feature: its
//! job is to prove one GPU representation of a decoded radar sweep works
//! and to measure it (`src/bin/harness.rs`), then record that choice in
//! `docs/adr/0007-radar-render-gpu-data-representation.md` before later
//! stages build a real, interactive renderer on top of it. Scope, per
//! `Agent Context/context/stages/S03-gpu-renderer.md`: one radar, one
//! sweep, one moment (REF), one example palette.
//!
//! # The GPU representation
//!
//! A hybrid of two of the three candidates the stage doc lists:
//!
//! 1. **A storage-buffer representation of one sweep's one moment**
//!    ([`sweep_buffers`]): a flat [`sweep_buffers::GpuRadialMeta`] array
//!    (one entry per radial that actually carries the requested moment —
//!    radials without it are simply absent, not zero-filled) plus every
//!    one of those radials' gates concatenated into a flat
//!    [`sweep_buffers::GpuGateSample`] array. This is what lets the
//!    representation support irregular radial spacing and missing/
//!    partial radials: nothing here assumes a fixed `(radial, gate)`
//!    grid.
//! 2. **A 1D radial-index lookup texture** ([`lookup_texture`]): for O(1)
//!    azimuth -> radial resolution in the shader, reusing
//!    [`radar_geo::find_radial_index`]'s "nearest radial by circular
//!    azimuth distance" convention directly rather than reimplementing
//!    it. A documented sentinel ([`lookup_texture::SENTINEL_NO_RADIAL`])
//!    marks azimuth buckets with no matching radial (a partial-scan gap).
//!
//! The plain-2D-texture candidate (a texture indexed directly by
//! `(radial, gate)`) was considered and **not** implemented — see
//! `docs/adr/0007-radar-render-gpu-data-representation.md` for why: it
//! forces a fixed, uniform `(radial, gate)` grid, which fights this
//! stage's "support irregular radial spacing and missing/partial
//! radials" requirement, and building it would require resampling real
//! (possibly ragged) radial/gate layouts onto that grid — exactly the
//! kind of interpolation `GLOBAL_CONTRACT.md` warns can turn missing/
//! flagged values into fabricated meteorology.
//!
//! # Never-fabricate rule
//!
//! [`sweep_buffers::GpuGateSample`] preserves "missing"/"range-folded" as
//! flags distinct from "valid" ([`radar_types::GateValue`]'s own
//! distinction, carried through unchanged), and
//! `shaders/radar_sweep.wgsl` branches on that flag *before* ever
//! sampling the palette — a missing or range-folded gate can never be
//! silently rendered as if it were a real reading. See the module docs on
//! [`sweep_buffers`] and [`palette`] for the full reasoning.
//!
//! # Nearest-gate/nearest-radial only
//!
//! Both the CPU-side lookup-table build and the shader's azimuth-to-
//! radial and range-to-gate resolution are nearest-match, half-open-
//! interval lookups (mirroring `radar_geo::lookup`'s own conventions) —
//! there is no interpolation between radials or between gates anywhere in
//! this crate, satisfying the stage doc's "nearest-gate mode must exist
//! and be the default" (here, it is the *only* mode; interpolation is out
//! of scope for this prototype).
//!
//! # Scope boundary: no real geodesy in the shader
//!
//! The shader works entirely in a flat, site-relative Cartesian plane
//! (kilometers), not real latitude/longitude — resolving an actual
//! geographic cursor position to azimuth/range against Earth curvature is
//! `radar-geo`'s job (see `docs/adr/0006-earth-model-for-radar-geometry.md`)
//! and stays a separate concern from proving this render pipeline. See
//! `shaders/radar_sweep.wgsl`'s header comment for the full boundary.
//!
//! # S05: beyond the one hardcoded REF ramp
//!
//! [`palette`] above is explicitly an S03 placeholder (one hardcoded REF
//! ramp). [`color_table`] is S05's real, original, documented,
//! user-editable color-table format (`COLOR_TABLE_FORMAT.md`, one file per
//! built-in default under `color_tables/`, `docs/adr/0009-original-color-table-format.md`)
//! supporting any [`radar_types::MomentKind`], stepped or gradient
//! mappings, alpha, and explicit missing/range-folded colors — it
//! generalizes (reuses, does not duplicate) `palette`'s interpolation
//! primitives rather than replacing this module.

pub mod camera;
pub mod color_table;
pub mod gpu;
pub mod lookup_texture;
pub mod palette;
pub mod sweep_buffers;

#[cfg(test)]
mod gpu_tests;
