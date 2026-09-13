//! `forecast-core` -- S08: the generic gridded-forecast-field abstraction,
//! extracted from S07's `provider-gefs` (the one concrete, real, CI-green
//! provider that abstraction is generalized *from*, per CLAUDE.md's "prefer
//! a second concrete implementation before generalizing" -- `provider-hrrr`,
//! S08's second provider, is what actually validates the generalization).
//!
//! # Canonical concepts (S08 stage file)
//!
//! - [`provider::ForecastProvider`] -- a trait: discover/fetch/decode one
//!   field for one run/variable/statistic.
//! - [`model::ModelRun`] / [`model::ModelMetadata`] -- one published run's
//!   canonical identity, and a provider's own static description.
//! - [`request::FieldRequest`] -- what a caller asks a provider for.
//! - [`grid::ForecastGrid`] -- one decoded field: metadata plus a row-major
//!   grid of values, generalizing S07's `GriddedField`/`GridGeometry` to
//!   two real grid shapes ([`grid::RegularLatLonGrid`], GEFS's; and
//!   [`grid::LambertConformalGrid`], HRRR's real Lambert Conformal Conic
//!   grid -- see [`projection`] and `docs/adr/0012-hrrr-lambert-conformal-grid.md`).
//! - [`point::PointForecast`] -- a single decoded value at one point.
//! - [`variable::ForecastVariable`] -- canonical variable identity,
//!   generalizing S07's `CanonicalField`.
//! - [`ensemble::EnsembleStatistic`] -- canonical ensemble statistic/member
//!   identity, generalizing S07's `EnsembleIdentity`; carried as
//!   `Option<EnsembleStatistic>` on a [`grid::ForecastGrid`] so a
//!   deterministic provider (HRRR) is not forced to fake one (see that
//!   module's doc comment).
//!
//! Preserving source-native names as metadata (`FORECASTING.md`) is
//! [`grid::NativeVariableMetadata`], carried on every [`grid::ForecastGrid`].
//!
//! # The shared GPU grid renderer
//!
//! [`gpu`] (plus the CPU-side [`render`] helpers and
//! `shaders/forecast_grid.wgsl`) is the *one* grid-render pipeline both
//! `provider-gefs` and `provider-hrrr` render through -- moved here from
//! S07's `provider-gefs::gpu`/`render` and generalized to take a
//! [`grid::ForecastGrid`] with no provider-specific knowledge at all. This
//! is what the S08 stage file's exit criteria means by "the same... grid
//! renderer": one render call path
//! ([`gpu::render_forecast_grid`]), not one per provider. See
//! `shaders/forecast_grid.wgsl`'s module doc for how it branches on grid
//! *projection kind* (a property of the geometry, reusable by any future
//! provider) rather than on provider identity, per the stage file's own
//! rule ("if HRRR requires provider-specific conditionals throughout the
//! UI, improve the abstraction rather than special-casing broadly").
//!
//! # This is forecast guidance, never observed radar
//!
//! GLOBAL_CONTRACT.md: "Never call model precipitation 'future radar'" /
//! "Observations and forecasts must always be distinguishable." Every
//! [`grid::ForecastGrid`] explicitly carries its own run/init time,
//! forecast lead, and valid time -- nothing built from this crate's types
//! is, or should ever be labeled as, an observation.
//!
//! # What stays out of this crate
//!
//! Bucket/key layout, `.idx` parsing, and GRIB2 decode wiring for any
//! specific provider all stay in that provider's own crate
//! (`provider-gefs`, `provider-hrrr`) -- this crate only defines the shared
//! vocabulary and the shared renderer, per [`provider::ForecastProvider`]'s
//! own module doc.

pub mod ensemble;
pub mod gpu;
pub mod grid;
pub mod model;
pub mod point;
pub mod projection;
pub mod provider;
pub mod render;
pub mod request;
pub mod sleep;
pub mod time;
pub mod variable;

#[cfg(test)]
mod gpu_tests;
