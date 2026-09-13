//! `provider-hrrr` -- S08: NOAA HRRR forecast provider, the second concrete
//! `forecast_core::provider::ForecastProvider` implementation (alongside
//! `provider-gefs`), built specifically to validate `forecast-core`'s
//! abstraction against a real provider with genuinely different
//! characteristics: deterministic (no ensemble), CONUS-only, ~3km
//! resolution, hourly cadence, and a real Lambert Conformal Conic grid
//! (GEFS's own grid is regular latitude/longitude) -- see
//! `docs/adr/0012-hrrr-lambert-conformal-grid.md`.
//!
//! # Pipeline
//!
//! ```text
//! keys (pure)      -- construct/parse HRRR S3 object keys
//!   -> client       -- anonymous HTTP: run discovery, .idx fetch, HEAD
//!                      (Content-Length), Range GET (sparse fetch)
//!   -> idx          -- parse .idx, find one field's message, compute its
//!                      exact byte range
//!   -> decode        -- GRIB2 decode (via the `grib` crate) into
//!                      `forecast_core::grid::ForecastGrid`
//!   -> provider      -- this crate's `ForecastProvider` implementation,
//!                      tying the above together
//! ```
//!
//! Rendering (the GPU grid pipeline) is `forecast-core`'s job, not this
//! crate's -- see `forecast_core::gpu::render_forecast_grid`, called
//! identically for a GEFS- or HRRR-decoded `ForecastGrid`.
//!
//! # This is forecast guidance, never observed radar
//!
//! GLOBAL_CONTRACT.md: "Never call model precipitation 'future radar'" /
//! "Observations and forecasts must always be distinguishable." Every
//! [`forecast_core::grid::ForecastGrid`] this crate produces carries its
//! own run/init time, forecast lead, and valid time explicitly.

pub mod client;
pub mod decode;
pub mod error;
pub mod idx;
pub mod keys;
pub mod provider;
mod sleep;
mod xml;

pub use error::HrrrError;
pub use provider::HrrrProvider;
