//! `mrms-web` -- S09 Phase 2: browser/wasm-bindgen glue exposing
//! `crates/mrms` (national MRMS radar-mosaic OBSERVATION products) to
//! `apps/web`, mirroring `forecast-web::wasm_api`'s working pattern
//! (`crates/forecast-web/src/wasm_api.rs`) rather than inventing a new one:
//! JSON strings for structured data, `Result<_, JsValue>`/a rejected
//! `Promise` for errors, `future_to_promise` for the genuinely async
//! discover/fetch/render calls, and `Rc<RefCell<..>>` interior state so a
//! caller invoking two methods back-to-back without awaiting the first
//! `Promise` can never trip a double-borrow panic.
//!
//! # Why a separate crate, not glue inside `mrms` itself
//!
//! Same reasoning `forecast-web` documents for `forecast-core`: `mrms` is a
//! plain, browser/native-agnostic fetch/decode/render client with no
//! `wasm-bindgen` dependency of its own (see `crates/mrms/Cargo.toml`) --
//! adding JS-facing glue directly to it would tie a library crate to one
//! consumption mode. This crate plays the same "thin adapter" role
//! `forecast-web`/`radar-web` already play over their own lower-level
//! crates.
//!
//! # This is an observation, never a forecast
//!
//! GLOBAL_CONTRACT.md requires observations and forecasts always be
//! distinguishable and forbids calling model precipitation "future radar" --
//! the converse discipline applies here just as strongly (see
//! `mrms::lib`'s own module doc): this crate's public API deliberately never
//! borrows forecast-flavored naming (no "run", "lead", "ensemble") from
//! `forecast-web`'s. [`wasm_api::MrmsHandle`] discovers and fetches a
//! **snapshot** (one real-world instant), not a run; every JSON payload
//! carries exactly one timestamp (`validTime`, the observed instant),
//! never a run/lead pair.
//!
//! # Scope
//!
//! Only what S09 Phase 2 needs so Phase 3 (`apps/web` wiring, a separate
//! follow-up task) has a stable API to call: select a product by id,
//! discover its latest published snapshot, fetch and decode it, and render
//! the most-recently-fetched snapshot through
//! `forecast_core::gpu::render_forecast_grid` (via
//! `mrms::palette::render_mrms_grid`) verbatim -- no reimplementation of
//! that render/palette path, and `apps/web` itself is not touched here.

#[cfg(target_arch = "wasm32")]
mod wasm_api;

#[cfg(target_arch = "wasm32")]
pub use wasm_api::MrmsHandle;
