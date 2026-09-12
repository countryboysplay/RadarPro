//! JSON output encoding: a GeoJSON `FeatureCollection` for a map layer, and
//! a JSON change-event list for a details-panel/list UI.
//!
//! Deliberately a plain, non-wasm-gated module: all of the actual encoding
//! logic lives here, native and unit-testable on the host target, exactly
//! matching this codebase's established "no logic of its own in the
//! browser glue" convention (see `radar-web`'s crate docs). `src/wasm_api.rs`
//! (wasm32-only) is a thin wrapper that calls these functions and returns
//! their `String` result as a `JsValue`/plain `String` across the
//! wasm-bindgen boundary, for the caller to `JSON.parse`.
//!
//! # Why a JSON string, not a hand-built `js_sys` object tree
//!
//! `radar-web`'s `range_rings_geojson` returns a `JsValue` built directly
//! from `js_sys::Array`, because its payload is purely nested coordinate
//! arrays -- cheap and simple to build that way, with no extra JSON
//! encode/decode pass. This crate's payload is a full GeoJSON
//! `FeatureCollection` with many string/numeric/optional `properties`
//! fields per feature; hand-building the equivalent `js_sys::Object`/
//! `js_sys::Array` tree field-by-field would be substantially more glue
//! code for no real benefit, since `serde_json` already produces exactly
//! the right JSON text and `JSON.parse` (a native, highly optimized browser
//! primitive) is the standard, idiomatic way to hand a MapLibre GeoJSON
//! source its data anyway (`map.getSource('alerts').setData(JSON.parse(...))`
//! or, in practice, most map libraries also accept a JSON *string* to
//! `fetch`-adjacent APIs directly). This is a deliberate, documented
//! departure from `range_rings_geojson`'s approach, not an inconsistency.

use serde::Serialize;

use crate::model::{Alert, AlertGeometry};
use crate::store::{AlertChange, AlertKey, ExpiryReason};

#[derive(Serialize)]
#[serde(tag = "type")]
enum GeometryJson {
    Polygon {
        coordinates: Vec<Vec<[f64; 2]>>,
    },
    MultiPolygon {
        coordinates: Vec<Vec<Vec<[f64; 2]>>>,
    },
}

impl From<&AlertGeometry> for GeometryJson {
    fn from(geometry: &AlertGeometry) -> Self {
        match geometry {
            AlertGeometry::Polygon(rings) => GeometryJson::Polygon {
                coordinates: rings.clone(),
            },
            AlertGeometry::MultiPolygon(polygons) => GeometryJson::MultiPolygon {
                coordinates: polygons.clone(),
            },
        }
    }
}

#[derive(Serialize)]
struct AlertPropertiesJson<'a> {
    #[serde(rename = "messageType")]
    message_type: &'static str,
    event: &'a str,
    severity: &'static str,
    certainty: &'static str,
    urgency: &'static str,
    sender: &'a str,
    #[serde(rename = "senderId")]
    sender_id: &'a str,
    /// Epoch milliseconds (UTC) -- directly usable as `new Date(ms)` in JS.
    issued: i64,
    effective: i64,
    expires: i64,
    ends: Option<i64>,
    headline: Option<&'a str>,
    description: Option<&'a str>,
    instruction: Option<&'a str>,
    #[serde(rename = "affectedAreas")]
    affected_areas: &'a [String],
}

impl<'a> From<&'a Alert> for AlertPropertiesJson<'a> {
    fn from(alert: &'a Alert) -> Self {
        AlertPropertiesJson {
            message_type: alert.message_type.as_str(),
            event: &alert.event,
            severity: alert.severity.as_str(),
            certainty: alert.certainty.as_str(),
            urgency: alert.urgency.as_str(),
            sender: &alert.sender,
            sender_id: &alert.sender_id,
            issued: alert.issued.epoch_millis(),
            effective: alert.effective.epoch_millis(),
            expires: alert.expires.epoch_millis(),
            ends: alert.ends.map(|t| t.epoch_millis()),
            headline: alert.headline.as_deref(),
            description: alert.description.as_deref(),
            instruction: alert.instruction.as_deref(),
            affected_areas: &alert.affected_areas,
        }
    }
}

#[derive(Serialize)]
struct AlertFeatureJson<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    /// The alert's stable [`AlertKey`] (see `crate::store`'s module docs),
    /// used as this GeoJSON feature's persistent id -- not the alert's own
    /// `id`, which rotates on every update.
    id: &'a str,
    properties: AlertPropertiesJson<'a>,
    geometry: Option<GeometryJson>,
}

#[derive(Serialize)]
struct FeatureCollectionJson<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    features: Vec<AlertFeatureJson<'a>>,
}

/// Encode a set of `(stable key, alert)` pairs -- as returned by
/// [`crate::store::AlertStore::active_alerts_with_keys`] -- into a GeoJSON
/// `FeatureCollection` JSON string, ready for a MapLibre `geojson` source.
pub fn alerts_to_geojson<'a>(
    alerts: impl IntoIterator<Item = (&'a AlertKey, &'a Alert)>,
) -> String {
    let features = alerts
        .into_iter()
        .map(|(key, alert)| AlertFeatureJson {
            kind: "Feature",
            id: key.as_str(),
            properties: AlertPropertiesJson::from(alert),
            geometry: alert.geometry.as_ref().map(GeometryJson::from),
        })
        .collect();
    let collection = FeatureCollectionJson {
        kind: "FeatureCollection",
        features,
    };
    // `serde_json::to_string` only fails on a handful of programmer-error
    // conditions (a map with non-string keys, a `NaN`/`Infinity` float, or
    // a `Serialize` impl that itself errors) -- none reachable from this
    // module's own types (every float here came from already-validated,
    // finite `f64` coordinates or a `Timestamp`'s `i64` millisecond
    // count), so an encode failure here would indicate a real bug in this
    // function, not malformed *input* (which was already rejected back in
    // `crate::parse`). Documented, not silently swallowed.
    serde_json::to_string(&collection).expect("FeatureCollectionJson always encodes")
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum AlertChangeJson<'a> {
    New {
        key: &'a str,
        alert: AlertJson<'a>,
    },
    Updated {
        key: &'a str,
        alert: AlertJson<'a>,
    },
    Cancelled {
        key: &'a str,
        alert: AlertJson<'a>,
    },
    Expired {
        key: &'a str,
        alert: AlertJson<'a>,
        reason: &'static str,
    },
}

/// A full alert's fields (properties plus geometry), used for change
/// events -- a details-panel/list UI needs the whole alert, not just a
/// GeoJSON feature's `properties`.
#[derive(Serialize)]
struct AlertJson<'a> {
    id: &'a str,
    #[serde(flatten)]
    properties: AlertPropertiesJson<'a>,
    geometry: Option<GeometryJson>,
}

impl<'a> From<&'a Alert> for AlertJson<'a> {
    fn from(alert: &'a Alert) -> Self {
        AlertJson {
            id: &alert.id,
            properties: AlertPropertiesJson::from(alert),
            geometry: alert.geometry.as_ref().map(GeometryJson::from),
        }
    }
}

fn expiry_reason_str(reason: ExpiryReason) -> &'static str {
    match reason {
        ExpiryReason::TimeExpired => "TimeExpired",
        ExpiryReason::AbsentFromPolls => "AbsentFromPolls",
    }
}

/// Encode a batch of [`AlertChange`]s (as returned by
/// [`crate::store::AlertStore::ingest_poll`],
/// [`crate::store::AlertStore::expire_stale`], or
/// [`crate::store::AlertStore::drain_changes`]) into a JSON array string,
/// for a details-panel/list UI to react to incrementally.
pub fn changes_to_json(changes: &[AlertChange]) -> String {
    let encoded: Vec<AlertChangeJson> = changes
        .iter()
        .map(|change| match change {
            AlertChange::New { key, alert } => AlertChangeJson::New {
                key,
                alert: AlertJson::from(alert),
            },
            AlertChange::Updated { key, alert } => AlertChangeJson::Updated {
                key,
                alert: AlertJson::from(alert),
            },
            AlertChange::Cancelled { key, alert } => AlertChangeJson::Cancelled {
                key,
                alert: AlertJson::from(alert),
            },
            AlertChange::Expired { key, alert, reason } => AlertChangeJson::Expired {
                key,
                alert: AlertJson::from(alert),
                reason: expiry_reason_str(*reason),
            },
        })
        .collect();
    // See `alerts_to_geojson`'s comment: not reachable for this module's
    // own types.
    serde_json::to_string(&encoded).expect("AlertChangeJson always encodes")
}
