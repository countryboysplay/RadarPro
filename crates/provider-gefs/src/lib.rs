//! `provider-gefs` -- S07/S08: NOAA GEFS ensemble forecast provider.
//!
//! S07 moved **one** real field (2-meter temperature) end-to-end from
//! NOAA's public, anonymous `noaa-gefs-pds` S3 bucket to a rendered,
//! color-mapped, geographically correct GPU image, proving the GRIB2/
//! ensemble pipeline works -- deliberately not a general forecast
//! framework at the time.
//!
//! S08 generalizes *from* that real, working provider: this crate now
//! implements `forecast_core::provider::ForecastProvider` ([`GefsProvider`])
//! and depends on `forecast-core` for the shared canonical types
//! (`ForecastGrid`, `EnsembleStatistic`, `ForecastVariable`, `UtcTimestamp`)
//! and the shared GPU grid renderer, instead of defining its own parallel
//! copies. What stays here is everything genuinely GEFS-specific: the
//! `noaa-gefs-pds` bucket/key layout ([`keys`]), `.idx` parsing ([`idx`]),
//! the ensemble-identity byte-offset extraction ([`ensemble`], ADR-0011),
//! and GRIB2 decode wiring ([`decode`]).
//!
//! # This is forecast guidance, never observed radar
//!
//! GLOBAL_CONTRACT.md: "Never call model precipitation 'future radar'" /
//! "Observations and forecasts must always be distinguishable." Every
//! [`forecast_core::grid::ForecastGrid`] this crate produces carries its
//! own run/init time, forecast lead, and valid time explicitly -- nothing
//! in this crate is, or is ever labeled as, an observation.
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
//!                      identity extraction -> forecast_core::grid::ForecastGrid
//!   -> provider      -- this crate's `ForecastProvider` implementation,
//!                      tying the above together
//! ```
//!
//! Rendering (the GPU grid pipeline) is `forecast-core`'s job, not this
//! crate's -- see `forecast_core::gpu::render_forecast_grid`, called
//! identically for a GEFS- or HRRR-decoded `ForecastGrid`.
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
pub mod idx;
pub mod keys;
pub mod provider;
mod xml;

pub use error::GefsError;
pub use provider::GefsProvider;
