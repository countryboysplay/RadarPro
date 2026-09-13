//! `forecast-web` -- S08 follow-up: browser/wasm-bindgen glue exposing
//! `forecast-core`'s `ForecastProvider` abstraction (`provider-gefs`,
//! `provider-hrrr`) to `apps/web`, mirroring the working pattern
//! `weather-alerts` already established for its own domain (see that
//! crate's `src/wasm_api.rs` module docs) rather than inventing a new one.
//!
//! # Why this is a separate crate, not glue inside `forecast-core` itself
//! (unlike `weather-alerts`, which puts its own glue directly in-crate)
//!
//! `weather-alerts` has no lower-level crate to adapt -- it *is* the whole
//! domain. `forecast-core` is different: it is deliberately
//! provider-agnostic (`ForecastProvider`, `ForecastGrid`, the shared GPU
//! renderer) and does not itself know about `provider-gefs`/`provider-hrrr`
//! at all -- adding wasm-bindgen glue directly to it would either force it
//! to depend on both concrete providers (breaking the "GEFS/HRRR must each
//! fail independently" boundary the trait exists to enforce) or leave the
//! glue with no provider to actually call. This crate plays the same role
//! `radar-web` plays over `nexrad-level2`/`radar-geo`/`radar-render`: a
//! thin adapter crate that depends on the concrete pieces so the pieces
//! themselves don't have to.
//!
//! # Why an enum, not a trait object
//!
//! `forecast_core::provider::ForecastProvider` is not dyn-safe (associated
//! `Run`/`Error` types, `impl Future` return position -- see that trait's
//! own module docs). With exactly two concrete providers in this workspace,
//! a small enum dispatching to whichever one is selected
//! ([`wasm_api::ProviderHandle`]'s internal `ProviderImpl`) is the
//! established, non-speculative choice here -- not a generic "provider
//! trait object" framework this stage's exit criteria never asked for.
//!
//! # Scope
//!
//! Only what S08's exit criteria need (`apps/web`'s forecast UI switching
//! between GEFS/HRRR through one shared API): select a provider by id,
//! discover its latest run, fetch one field, and render the
//! most-recently-fetched field through `forecast_core::gpu::render_forecast_grid`
//! verbatim (no reimplementation of that render/palette path). `apps/web`
//! itself is not wired up here -- that is an explicitly separate follow-up
//! task; this crate only has to expose a stable API for it to call.
//!
//! # This is forecast guidance, never observed radar
//!
//! GLOBAL_CONTRACT.md: "Never call model precipitation 'future radar'."
//! Every JSON payload this crate hands to JS carries the same run/lead/
//! valid-time and ensemble-identity fields `forecast_core::grid::ForecastGrid`
//! already models -- nothing is collapsed or renamed away at this
//! boundary.

#[cfg(target_arch = "wasm32")]
mod wasm_api;

#[cfg(target_arch = "wasm32")]
pub use wasm_api::ProviderHandle;
