//! `provider-gefs` -- S07 GEFS ensemble forecast proof of concept.
//!
//! Moves **one** real field (2-meter temperature) end-to-end from NOAA's
//! public, anonymous `noaa-gefs-pds` S3 bucket to a rendered, color-mapped,
//! geographically correct GPU image, proving the GRIB2/ensemble pipeline
//! works. Per the S07 stage file's "Critical rule", this is deliberately
//! **not** a general forecast framework: no generic multi-field/
//! multi-provider abstraction, no `forecast-core` crate. A second real
//! field and provider (wind, precipitation, MSLP; HRRR, MRMS) are future
//! stages' work to generalize from.
//!
//! # This is forecast guidance, never observed radar
//!
//! GLOBAL_CONTRACT.md: "Never call model precipitation 'future radar'" /
//! "Observations and forecasts must always be distinguishable." Every type
//! in this crate ([`field::GriddedField`]) is explicitly a *forecast*
//! field carrying its own run/init time, forecast lead, and valid time --
//! nothing in this crate is, or is ever labeled as, an observation.
//!
//! # Pipeline
//!
//! ```text
//! keys (pure)      -- construct/parse GEFS S3 object keys
//!   -> client       -- anonymous HTTP: ListObjectsV2 discovery, .idx fetch,
//!                      HEAD (Content-Length), Range GET (sparse fetch)
//!   -> idx          -- parse .idx, find one field's message, compute its
//!                      exact byte range
//!   -> decode        -- GRIB2 decode (via the `grib` crate) + ensemble
//!                      identity extraction -> field::GriddedField
//!   -> render/gpu    -- CPU-side grid buffer + GPU upload/pipeline for a
//!                      simple lat/lon textured quad (reusing
//!                      `radar-render`'s palette LUT machinery)
//! ```
//!
//! See `src/bin/harness.rs` for a native, non-wasm end-to-end proof (real
//! bucket fetch -> decode -> render -> PNG), and
//! `docs/adr/0011-gefs-grib2-crate-selection-and-limitations.md` for the
//! `grib` crate evaluation, the real bucket/`.idx` facts this crate was
//! built against, and every documented limitation/gap.

pub mod client;
pub mod decode;
pub mod ensemble;
pub mod error;
pub mod field;
pub mod gpu;
pub mod idx;
pub mod keys;
pub mod render;
pub mod time;
mod xml;

#[cfg(test)]
mod gpu_tests;

pub use error::GefsError;
