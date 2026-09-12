//! `weather-alerts` -- S06: the normalized NWS alert domain model,
//! CAP/GeoJSON parsing, and poll-race-safe alert lifecycle state
//! management, per `ARCHITECTURE.md`'s `VectorLayer` ("warnings/outlooks")
//! category and `Agent Context/context/stages/S06-nws-alerts.md`.
//!
//! # Scope
//!
//! This crate covers the *core* (Rust) half of S06: [`model`] (the
//! normalized `Alert` type and its CAP enums), [`parse`] (bounded,
//! never-panicking parsing of a real `/alerts/active` GeoJSON
//! `FeatureCollection` response into `Alert`s), [`store`] (the
//! poll-race-safe lifecycle reconciliation engine -- this stage's core
//! value; see `docs/adr/0010-nws-alert-lifecycle-reconciliation.md`),
//! [`json`] (GeoJSON/JSON output encoding), and a thin `wasm_api` glue
//! layer (wasm32-only) exposing all of the above to a browser caller.
//!
//! **Not** in scope here (a separate follow-up task, per the stage brief):
//! `apps/web`'s actual MapLibre polygon layer, details panel, list view,
//! or the `fetch()`-based poll loop that calls this crate's wasm API.
//!
//! # Threat model (`GLOBAL_CONTRACT.md`)
//!
//! The `/alerts/active` response body is untrusted, network-sourced input.
//! [`parse::parse_feature_collection`] never panics on malformed content,
//! and one malformed feature inside an otherwise-valid response never
//! discards the rest of the batch -- see [`parse`]'s module docs.
//!
//! # The two rules this stage exists to satisfy
//!
//! - **"Official alerts are not inferred from radar."** This crate
//!   contains zero radar-derived logic of any kind; every [`model::Alert`]
//!   comes from parsing NWS's own CAP/GeoJSON feed, full stop.
//! - **"Do not let polling races incorrectly resurrect or remove alerts."**
//!   [`store::AlertStore`] is designed specifically around this: an
//!   explicit `Cancel` is the only *immediate* removal signal; a held
//!   alert's own `expires` timestamp lapsing is checked both at ingest
//!   time and at query time (never merely inferred from "it wasn't in the
//!   latest poll"); and a bare absence from the polled set requires
//!   several independent, consecutive polls to agree before it is treated
//!   as removal at all. See `docs/adr/0010-...` for the full design.
//!
//! # Why this crate depends on `radar-types` for `Timestamp`
//!
//! `radar_types::Timestamp` (a UTC point-in-time value, milliseconds since
//! the Unix epoch) is already this project's one canonical internal time
//! representation (`GLOBAL_CONTRACT.md`: "Internal time is UTC"). Reusing
//! it here -- rather than defining a second, alert-crate-local timestamp
//! type wrapping the same `i64` millisecond count -- avoids exactly the
//! kind of duplicate-domain-concept split this project's own conventions
//! warn against. This is a read-only dependency on `radar_types`'s public
//! `Timestamp` API; this task does not modify `crates/radar-types` itself.
//! See [`parse_rfc3339`] for why parsing the CAP feed's own RFC 3339
//! timestamp strings into a `Timestamp` is still done by hand in this
//! crate rather than by adding a date/time-parsing dependency.

pub mod json;
pub mod model;
pub mod parse;
pub mod store;
mod time;

#[cfg(target_arch = "wasm32")]
mod wasm_api;

#[cfg(target_arch = "wasm32")]
pub use wasm_api::{parse_alerts_json, AlertStoreHandle};

// Re-exported for convenience: the RFC 3339 parser is otherwise purely
// internal plumbing (`parse`/`store` are the crate's real public surface),
// but a caller with its own reason to parse a CAP timestamp string (e.g. a
// future stage handling SPC/NHC products with the same CAP timestamp
// format -- see `docs/adr/0010-...`'s "generalizes to" section) should not
// have to reimplement it.
pub use time::{parse_rfc3339, TimeParseError};
