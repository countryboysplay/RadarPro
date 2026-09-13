//! S09 Phase 1: MRMS national mosaic (reflectivity) and precipitation-rate
//! observation products.
//!
//! # This is an observation product, never a forecast
//!
//! MRMS publishes a plain, unbroken stream of ~2-minute national radar
//! mosaic snapshots -- there is no run/lead-time/ensemble concept at all
//! (confirmed empirically: every real message's GRIB2 `forecast_time` is
//! zero, i.e. Section 1's reference time already *is* the observation's
//! valid time). GLOBAL_CONTRACT requires observations and forecasts always
//! be distinguishable and never calls model precipitation "future radar" --
//! the converse discipline applies just as strongly here: this crate does
//! **not** implement `forecast_core::provider::ForecastProvider` (that
//! trait's shape -- a model run, a forecast lead time, an optional ensemble
//! identity -- has no honest MRMS equivalent) and its own
//! [`grid::MrmsGrid`] type carries exactly one timestamp
//! (`valid_time`, the observed instant), not a run/lead-time pair. See
//! `docs/adr/0014-mrms-grib2-png-unpack-and-local-discipline.md` for the
//! full empirical trail behind every decision in this crate.
//!
//! # What this crate reuses, and what it does not duplicate
//!
//! - Grid geometry: [`forecast_core::grid::GridGeometry::RegularLatLon`]
//!   (MRMS's real grid, GRIB2 Grid Definition Template 0 -- the same family
//!   GEFS uses).
//! - GPU rendering: [`forecast_core::gpu::render_forecast_grid`], the exact
//!   same shared renderer GEFS/HRRR use (no new shader/pipeline/palette-
//!   upload code -- see [`palette::render_mrms_grid`]).
//! - Reflectivity color table: `radar_render::color_table`'s existing
//!   NEXRAD REF ramp (same physical quantity, dBZ).
//!
//! # Modules
//! - [`keys`]: S3 object key construction/parsing (no network).
//! - [`client`]: anonymous HTTP access to `noaa-mrms-pds` (discovery,
//!   fetch, gunzip).
//! - [`decode`]: GRIB2 -> [`grid::MrmsGrid`], including missing-value
//!   sentinel classification.
//! - [`grid`]: the decoded-grid type and its explicit missing-value states.
//! - [`palette`]: color tables for both products, plus the GPU render
//!   convenience wrapper.

pub mod client;
pub mod decode;
pub mod error;
pub mod grid;
pub mod keys;
pub mod palette;
mod xml;
