//! Browser/wasm-bindgen glue for `weather-alerts`.
//!
//! # Why the wasm-bindgen glue lives directly in this crate, not a
//! separate adapter crate
//!
//! `radar-web` is a separate adapter crate over `nexrad-level2` +
//! `radar-geo` + `radar-render` because those are independently useful,
//! independently tested crates with real GPU/canvas concerns the adapter
//! alone needs (`wgpu::Surface` acquisition, `<canvas>` handles) --
//! splitting the adapter out keeps that browser-only surface-acquisition
//! code from ever being compiled into, or a dependency of, the pure decode/
//! geometry/render crates themselves.
//!
//! `weather-alerts` has no analogous lower-level crate to adapt: this
//! crate *is* the entire domain (parsing + lifecycle store), there is no
//! GPU/canvas/surface concern anywhere in its API, and it depends on
//! nothing this workspace would need to keep wasm-unaware. A separate
//! adapter crate here would therefore be pure boilerplate (a second
//! `Cargo.toml`, re-exporting the same types) with no real separation-of-
//! concerns benefit. This module is `#[cfg(target_arch = "wasm32")]`-gated
//! the same way `radar-web`'s `browser` module is, so a native
//! `cargo build --workspace`/`cargo test -p weather-alerts` never needs to
//! resolve `wasm-bindgen` at all -- the only difference from `radar-web`'s
//! layout is that the gate is a module within this crate rather than a
//! whole separate crate.
//!
//! # API shape
//!
//! [`AlertStoreHandle`] wraps a [`crate::store::AlertStore`]. Every method
//! that can fail on malformed input returns `Result<_, JsValue>` (a
//! descriptive error string) rather than panicking; every method that
//! returns structured data returns a JSON string for the caller to
//! `JSON.parse` -- see `crate::json`'s module docs for why a JSON string,
//! not a hand-built `js_sys` tree, was chosen here (unlike `radar-web`'s
//! `range_rings_geojson`).
//!
//! `now_epoch_millis` parameters take a plain `f64` (JS's native number
//! type) so a caller passes `Date.now()` directly with no conversion.

use wasm_bindgen::prelude::*;

use crate::model::Alert;
use crate::parse::{parse_feature_collection, ParsedAlerts};
use crate::store::AlertStore;
use radar_types::Timestamp;

/// Runs once when the wasm module is instantiated: installs a panic hook
/// so a Rust panic surfaces as a readable `console.error` message instead
/// of an opaque wasm trap -- same as `radar-web`'s `on_wasm_module_init`.
#[wasm_bindgen(start)]
fn on_wasm_module_init() {
    console_error_panic_hook::set_once();
}

fn now_from_millis(now_epoch_millis: f64) -> Timestamp {
    Timestamp::from_epoch_millis(now_epoch_millis as i64)
}

#[derive(serde::Serialize)]
struct SkippedFeatureJson<'a> {
    index: usize,
    id: Option<&'a str>,
    error: String,
}

#[derive(serde::Serialize)]
struct ParsedAlertsSummaryJson<'a> {
    #[serde(rename = "alertsGeoJson")]
    alerts_geojson: serde_json::Value,
    #[serde(rename = "filteredNonActual")]
    filtered_non_actual: usize,
    skipped: Vec<SkippedFeatureJson<'a>>,
}

/// Parse a raw `/alerts/active` response body (Deliverable 1 only, no
/// lifecycle tracking) and return a JSON-encoded summary:
/// `{"alertsGeoJson": <FeatureCollection>, "filteredNonActual": N,
/// "skipped": [{"index":.., "id":.., "error":".."}, ...]}`.
///
/// Useful for a caller that wants to inspect parse diagnostics directly
/// (e.g. a debug/QA view) separately from feeding a store. Returns `Err`
/// only when the *entire* response is unusable (invalid JSON, not a
/// `FeatureCollection`); a malformed individual feature is reported inside
/// `skipped`, not as an `Err`. Each successfully-parsed alert is keyed by
/// its own message id in `alertsGeoJson` (there is no "stable key" outside
/// a store -- stability only means something across successive polls).
#[wasm_bindgen(js_name = parseAlertsJson)]
pub fn parse_alerts_json(body: &str) -> Result<String, JsValue> {
    let parsed: ParsedAlerts =
        parse_feature_collection(body).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let alerts_geojson_str =
        crate::json::alerts_to_geojson(parsed.alerts.iter().map(|a: &Alert| (&a.id, a)));
    let alerts_geojson: serde_json::Value = serde_json::from_str(&alerts_geojson_str)
        .expect("alerts_to_geojson always produces valid JSON");

    let skipped = parsed
        .skipped
        .iter()
        .map(|s| SkippedFeatureJson {
            index: s.index,
            id: s.id.as_deref(),
            error: s.error.to_string(),
        })
        .collect();

    let summary = ParsedAlertsSummaryJson {
        alerts_geojson,
        filtered_non_actual: parsed.filtered_non_actual,
        skipped,
    };
    Ok(serde_json::to_string(&summary).expect("ParsedAlertsSummaryJson always encodes"))
}

/// A poll-race-safe holder of currently-active NWS alerts, exposed to
/// JS/wasm. See [`crate::store::AlertStore`]'s docs for the full lifecycle
/// design.
#[wasm_bindgen]
pub struct AlertStoreHandle {
    inner: AlertStore,
}

#[wasm_bindgen]
impl AlertStoreHandle {
    /// A new, empty store using this crate's default absence-limit safety
    /// net (see [`crate::store::DEFAULT_ABSENCE_LIMIT`]).
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            inner: AlertStore::new(),
        }
    }

    /// Feed one successful poll's raw `/alerts/active` response body into
    /// the store: parses it (Deliverable 1: filters to `status == "Actual"`,
    /// never panics on malformed content) and reconciles lifecycle state
    /// (Deliverable 2: new/update/replacement/cancellation/expiration,
    /// race-safe absence handling).
    ///
    /// `now_epoch_millis`: current UTC time (e.g. `Date.now()`), used only
    /// for this poll's time-based expiration check.
    ///
    /// Returns a JSON array of this poll's [`crate::store::AlertChange`]s
    /// (same shape as [`AlertStoreHandle::drain_changes`]). Returns `Err`
    /// only if the *entire* response body is unusable; a malformed
    /// individual feature inside it is dropped from this poll silently
    /// (per Deliverable 1 -- the caller can separately call
    /// [`parse_alerts_json`] for diagnostics on why).
    #[wasm_bindgen(js_name = ingestPoll)]
    pub fn ingest_poll(&mut self, body: &str, now_epoch_millis: f64) -> Result<String, JsValue> {
        let parsed =
            parse_feature_collection(body).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let changes = self
            .inner
            .ingest_poll(parsed.alerts, now_from_millis(now_epoch_millis));
        Ok(crate::json::changes_to_json(&changes))
    }

    /// Reconcile time-based expirations as of `now_epoch_millis` without
    /// waiting for the next poll -- see
    /// [`crate::store::AlertStore::expire_stale`]. Returns a JSON array of
    /// any resulting [`crate::store::AlertChange::Expired`] events.
    #[wasm_bindgen(js_name = expireStale)]
    pub fn expire_stale(&mut self, now_epoch_millis: f64) -> String {
        let changes = self.inner.expire_stale(now_from_millis(now_epoch_millis));
        crate::json::changes_to_json(&changes)
    }

    /// The currently-active alert set (time-filtered as of
    /// `now_epoch_millis`, applied at call time -- see
    /// [`crate::store::AlertStore::active_alerts`]) as a GeoJSON
    /// `FeatureCollection` JSON string, ready for a MapLibre `geojson`
    /// source.
    #[wasm_bindgen(js_name = activeAlertsGeoJson)]
    pub fn active_alerts_geojson(&self, now_epoch_millis: f64) -> String {
        let entries = self
            .inner
            .active_alerts_with_keys(now_from_millis(now_epoch_millis));
        crate::json::alerts_to_geojson(entries)
    }

    /// Drain and return every change accumulated since the last call to
    /// this method (see [`crate::store::AlertStore::drain_changes`]), as a
    /// JSON array string, for a details-panel/list UI to react to
    /// incrementally rather than re-rendering the full active set on every
    /// poll.
    #[wasm_bindgen(js_name = drainChanges)]
    pub fn drain_changes(&mut self) -> String {
        let changes = self.inner.drain_changes();
        crate::json::changes_to_json(&changes)
    }

    /// The number of alerts currently held (including any not yet past
    /// their own `expires` check at query time -- see
    /// [`crate::store::AlertStore::held_count`]).
    #[wasm_bindgen(js_name = heldCount)]
    pub fn held_count(&self) -> usize {
        self.inner.held_count()
    }
}

impl Default for AlertStoreHandle {
    fn default() -> Self {
        Self::new()
    }
}
