//! Parsing an `/alerts/active` GeoJSON `FeatureCollection` response into
//! [`crate::model::Alert`] values.
//!
//! # Threat model
//!
//! The response body is untrusted, network-sourced input (`GLOBAL_CONTRACT.md`:
//! "Remote data is unreliable and untrusted" / "No uncontrolled panics on
//! malformed input"), so every field is read via `serde_json::Value`
//! lookups with explicit presence/type/range validation -- never derived
//! `Deserialize` on the domain model, and never an indexing operation that
//! could panic. A malformed top-level document (not JSON at all, or not a
//! `FeatureCollection` shape) is a hard [`AlertParseError`]; a malformed
//! *individual feature* inside an otherwise-valid `FeatureCollection*` is
//! reported per-feature (see [`ParsedAlerts::skipped`]) without discarding
//! the rest of the batch -- "one bad alert in a `FeatureCollection` must
//! not lose all the good ones" is a stage requirement, proven directly by
//! `tests/parse.rs`'s `malformed_geometry_in_one_feature_does_not_lose_other_valid_features`.
//!
//! # `status` filtering
//!
//! Only `status == "Actual"` features become [`crate::model::Alert`]
//! values; `Test`, `Exercise`, `System`, and `Draft` are silently filtered
//! (counted in [`ParsedAlerts::filtered_non_actual`], not treated as
//! errors). This is a deliberate choice for a live weather application:
//! `Test`/`Exercise` messages are confirmed, real, and common in the live
//! feed (a `"Test Message"`/`"KEEPALIVE"` event was observed in a real
//! national snapshot fetched during development, with `status: "Test"`)
//! and must never be presented to a user as an actual hazard.

use serde_json::Value;
use thiserror::Error;

use crate::model::{
    Alert, AlertGeometry, AlertReference, Certainty, LonLat, MessageType, Ring, Severity, Urgency,
};
use crate::time::{parse_rfc3339, TimeParseError};

/// A hard failure to parse the top-level response at all: not valid JSON,
/// or not shaped like a GeoJSON `FeatureCollection`.
#[derive(Debug, Error)]
pub enum AlertParseError {
    #[error("response body is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    #[error("response is not a GeoJSON FeatureCollection: missing or non-string \"type\" field")]
    NotFeatureCollection,

    #[error("response \"type\" is {found:?}, expected \"FeatureCollection\"")]
    WrongTopLevelType { found: String },

    #[error("response is missing a \"features\" array")]
    MissingFeatures,
}

/// A failure to parse one feature within an otherwise-valid
/// `FeatureCollection`. Never causes the rest of the batch to be dropped;
/// see [`ParsedAlerts::skipped`].
#[derive(Debug, Error, PartialEq)]
pub enum FeatureParseError {
    #[error("feature is not a JSON object")]
    NotAnObject,

    #[error("feature is missing required field \"{0}\"")]
    MissingField(&'static str),

    #[error("field \"{field}\" has the wrong JSON type: expected {expected}")]
    WrongFieldType {
        field: &'static str,
        expected: &'static str,
    },

    #[error("unrecognized messageType {0:?} (expected \"Alert\", \"Update\", or \"Cancel\")")]
    UnknownMessageType(String),

    #[error("unrecognized severity {0:?}")]
    UnknownSeverity(String),

    #[error("unrecognized certainty {0:?}")]
    UnknownCertainty(String),

    #[error("unrecognized urgency {0:?}")]
    UnknownUrgency(String),

    #[error("invalid {field} timestamp {value:?}: {source}")]
    InvalidTimestamp {
        field: &'static str,
        value: String,
        #[source]
        source: TimeParseError,
    },

    #[error("invalid geometry: {0}")]
    Geometry(#[from] GeometryError),

    #[error("references[{index}] is malformed: {reason}")]
    InvalidReference { index: usize, reason: String },
}

/// A failure to parse a feature's `geometry`.
#[derive(Debug, Error, PartialEq)]
pub enum GeometryError {
    #[error("geometry is not a JSON object")]
    NotAnObject,

    #[error("geometry is missing a string \"type\" field")]
    MissingType,

    #[error("unsupported geometry type {0:?} (expected \"Polygon\" or \"MultiPolygon\")")]
    UnsupportedType(String),

    #[error("geometry is missing a \"coordinates\" array")]
    MissingCoordinates,

    #[error("{context}: expected an array, found something else")]
    NotAnArray { context: &'static str },

    #[error("{context}: position must be a 2-element [lon, lat] array, found {len} element(s)")]
    WrongPositionArity { context: &'static str, len: usize },

    #[error("{context}: coordinate value is not a finite number")]
    NonNumericCoordinate { context: &'static str },

    #[error("polygon ring at index {ring_index} has only {len} position(s); a linear ring needs at least 4")]
    RingTooShort { ring_index: usize, len: usize },
}

/// One feature that failed to parse, with enough context to report or log
/// without re-parsing.
#[derive(Debug)]
pub struct SkippedFeature {
    /// Index of this feature within the response's `features` array.
    pub index: usize,
    /// The feature's `properties.id`, if that much could be read before
    /// the error occurred.
    pub id: Option<String>,
    pub error: FeatureParseError,
}

/// The result of parsing one `/alerts/active` response.
#[derive(Debug, Default)]
pub struct ParsedAlerts {
    /// Successfully parsed, `status == "Actual"` alerts, in the order they
    /// appeared in the response.
    pub alerts: Vec<Alert>,
    /// Features that failed to parse (malformed input), with why.
    pub skipped: Vec<SkippedFeature>,
    /// Count of features that parsed structurally but were filtered out
    /// because `status != "Actual"` (not an error; see module docs).
    pub filtered_non_actual: usize,
}

/// Parse a complete `/alerts/active` GeoJSON `FeatureCollection` response
/// body.
///
/// Returns [`AlertParseError`] only for a failure that makes the *entire*
/// response unusable (invalid JSON, not a `FeatureCollection`). Failures
/// scoped to a single feature are instead collected into the returned
/// [`ParsedAlerts::skipped`], so one malformed alert can never take down
/// parsing of the rest of a real response.
pub fn parse_feature_collection(body: &str) -> Result<ParsedAlerts, AlertParseError> {
    let root: Value = serde_json::from_str(body)?;

    let root_type = root
        .get("type")
        .and_then(Value::as_str)
        .ok_or(AlertParseError::NotFeatureCollection)?;
    if root_type != "FeatureCollection" {
        return Err(AlertParseError::WrongTopLevelType {
            found: root_type.to_string(),
        });
    }

    let features = root
        .get("features")
        .and_then(Value::as_array)
        .ok_or(AlertParseError::MissingFeatures)?;

    let mut result = ParsedAlerts::default();
    for (index, feature) in features.iter().enumerate() {
        match parse_feature(feature) {
            Ok(Some(alert)) => result.alerts.push(alert),
            Ok(None) => result.filtered_non_actual += 1,
            Err(error) => {
                let id = feature
                    .get("properties")
                    .and_then(|p| p.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                result.skipped.push(SkippedFeature { index, id, error });
            }
        }
    }
    Ok(result)
}

/// Parse one GeoJSON `Feature`. Returns `Ok(None)` when the feature is
/// structurally valid but filtered by `status` (see module docs).
fn parse_feature(feature: &Value) -> Result<Option<Alert>, FeatureParseError> {
    if !feature.is_object() {
        return Err(FeatureParseError::NotAnObject);
    }
    let properties = field_object(feature, "properties")?;

    let status = string_field(properties, "status")?;
    if status != "Actual" {
        return Ok(None);
    }

    let id = string_field(properties, "id")?.to_string();
    let message_type = match string_field(properties, "messageType")? {
        "Alert" => MessageType::Alert,
        "Update" => MessageType::Update,
        "Cancel" => MessageType::Cancel,
        other => return Err(FeatureParseError::UnknownMessageType(other.to_string())),
    };
    let references = parse_references(properties)?;

    let event = string_field(properties, "event")?.to_string();
    let severity = parse_severity(string_field(properties, "severity")?)?;
    let certainty = parse_certainty(string_field(properties, "certainty")?)?;
    let urgency = parse_urgency(string_field(properties, "urgency")?)?;

    let sender = string_field(properties, "senderName")?.to_string();
    let sender_id = string_field(properties, "sender")?.to_string();

    let issued = timestamp_field(properties, "sent")?;
    let effective = timestamp_field(properties, "effective")?;
    let expires = timestamp_field(properties, "expires")?;
    let ends = optional_timestamp_field(properties, "ends")?;

    let headline = optional_string_field(properties, "headline")?;
    let description = optional_string_field(properties, "description")?;
    let instruction = optional_string_field(properties, "instruction")?;

    let affected_areas = string_field(properties, "areaDesc")?
        .split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let geometry = parse_geometry(feature.get("geometry").unwrap_or(&Value::Null))?;

    Ok(Some(Alert {
        id,
        message_type,
        references,
        event,
        severity,
        certainty,
        urgency,
        sender,
        sender_id,
        issued,
        effective,
        expires,
        ends,
        headline,
        description,
        instruction,
        affected_areas,
        geometry,
    }))
}

fn parse_severity(raw: &str) -> Result<Severity, FeatureParseError> {
    match raw {
        "Unknown" => Ok(Severity::Unknown),
        "Minor" => Ok(Severity::Minor),
        "Moderate" => Ok(Severity::Moderate),
        "Severe" => Ok(Severity::Severe),
        "Extreme" => Ok(Severity::Extreme),
        other => Err(FeatureParseError::UnknownSeverity(other.to_string())),
    }
}

fn parse_certainty(raw: &str) -> Result<Certainty, FeatureParseError> {
    match raw {
        "Unknown" => Ok(Certainty::Unknown),
        "Unlikely" => Ok(Certainty::Unlikely),
        "Possible" => Ok(Certainty::Possible),
        "Likely" => Ok(Certainty::Likely),
        "Observed" => Ok(Certainty::Observed),
        other => Err(FeatureParseError::UnknownCertainty(other.to_string())),
    }
}

fn parse_urgency(raw: &str) -> Result<Urgency, FeatureParseError> {
    match raw {
        "Unknown" => Ok(Urgency::Unknown),
        "Past" => Ok(Urgency::Past),
        "Future" => Ok(Urgency::Future),
        "Expected" => Ok(Urgency::Expected),
        "Immediate" => Ok(Urgency::Immediate),
        other => Err(FeatureParseError::UnknownUrgency(other.to_string())),
    }
}

fn parse_references(properties: &Value) -> Result<Vec<AlertReference>, FeatureParseError> {
    let Some(raw) = properties.get("references") else {
        return Ok(Vec::new());
    };
    let Some(array) = raw.as_array() else {
        return Err(FeatureParseError::WrongFieldType {
            field: "references",
            expected: "array",
        });
    };

    let mut references = Vec::with_capacity(array.len());
    for (index, entry) in array.iter().enumerate() {
        let identifier = entry
            .get("identifier")
            .and_then(Value::as_str)
            .ok_or_else(|| FeatureParseError::InvalidReference {
                index,
                reason: "missing string \"identifier\"".to_string(),
            })?
            .to_string();
        let sender = entry
            .get("sender")
            .and_then(Value::as_str)
            .ok_or_else(|| FeatureParseError::InvalidReference {
                index,
                reason: "missing string \"sender\"".to_string(),
            })?
            .to_string();
        let sent_raw = entry.get("sent").and_then(Value::as_str).ok_or_else(|| {
            FeatureParseError::InvalidReference {
                index,
                reason: "missing string \"sent\"".to_string(),
            }
        })?;
        let sent = parse_rfc3339(sent_raw).map_err(|e| FeatureParseError::InvalidReference {
            index,
            reason: format!("invalid \"sent\" timestamp {sent_raw:?}: {e}"),
        })?;
        references.push(AlertReference {
            identifier,
            sender,
            sent,
        });
    }
    Ok(references)
}

// -- small field-access helpers -------------------------------------------
//
// Every helper below returns a structured `FeatureParseError` instead of
// panicking/indexing, per this module's threat model.

fn field_object<'a>(value: &'a Value, field: &'static str) -> Result<&'a Value, FeatureParseError> {
    let inner = value
        .get(field)
        .ok_or(FeatureParseError::MissingField(field))?;
    if inner.is_object() {
        Ok(inner)
    } else {
        Err(FeatureParseError::WrongFieldType {
            field,
            expected: "object",
        })
    }
}

fn string_field<'a>(value: &'a Value, field: &'static str) -> Result<&'a str, FeatureParseError> {
    value
        .get(field)
        .ok_or(FeatureParseError::MissingField(field))?
        .as_str()
        .ok_or(FeatureParseError::WrongFieldType {
            field,
            expected: "string",
        })
}

fn optional_string_field(
    value: &Value,
    field: &'static str,
) -> Result<Option<String>, FeatureParseError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(FeatureParseError::WrongFieldType {
            field,
            expected: "string or null",
        }),
    }
}

fn timestamp_field(
    value: &Value,
    field: &'static str,
) -> Result<radar_types::Timestamp, FeatureParseError> {
    let raw = string_field(value, field)?;
    parse_rfc3339(raw).map_err(|source| FeatureParseError::InvalidTimestamp {
        field,
        value: raw.to_string(),
        source,
    })
}

fn optional_timestamp_field(
    value: &Value,
    field: &'static str,
) -> Result<Option<radar_types::Timestamp>, FeatureParseError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let ts = parse_rfc3339(s).map_err(|source| FeatureParseError::InvalidTimestamp {
                field,
                value: s.clone(),
                source,
            })?;
            Ok(Some(ts))
        }
        Some(_) => Err(FeatureParseError::WrongFieldType {
            field,
            expected: "string or null",
        }),
    }
}

// -- geometry parsing -------------------------------------------------------

/// Parse a GeoJSON `geometry` value: `null` -> `Ok(None)` (no shape, not an
/// error -- see [`AlertGeometry`]'s docs); `Polygon`/`MultiPolygon` ->
/// `Ok(Some(..))`; anything structurally wrong -> [`GeometryError`].
fn parse_geometry(value: &Value) -> Result<Option<AlertGeometry>, GeometryError> {
    if value.is_null() {
        return Ok(None);
    }
    let object = value.as_object().ok_or(GeometryError::NotAnObject)?;
    let geometry_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or(GeometryError::MissingType)?;
    let coordinates = object
        .get("coordinates")
        .ok_or(GeometryError::MissingCoordinates)?;

    match geometry_type {
        "Polygon" => {
            let rings = parse_polygon_coordinates(coordinates, "Polygon.coordinates")?;
            Ok(Some(AlertGeometry::Polygon(rings)))
        }
        "MultiPolygon" => {
            let raw_polygons = coordinates.as_array().ok_or(GeometryError::NotAnArray {
                context: "MultiPolygon.coordinates",
            })?;
            let mut polygons = Vec::with_capacity(raw_polygons.len());
            for polygon_coords in raw_polygons {
                polygons.push(parse_polygon_coordinates(
                    polygon_coords,
                    "MultiPolygon.coordinates[*]",
                )?);
            }
            Ok(Some(AlertGeometry::MultiPolygon(polygons)))
        }
        other => Err(GeometryError::UnsupportedType(other.to_string())),
    }
}

/// Parse a single `Polygon`-shaped coordinates array (a list of linear
/// rings) shared by both `Polygon.coordinates` and one entry of
/// `MultiPolygon.coordinates`.
fn parse_polygon_coordinates(
    value: &Value,
    context: &'static str,
) -> Result<Vec<Ring>, GeometryError> {
    let raw_rings = value
        .as_array()
        .ok_or(GeometryError::NotAnArray { context })?;
    let mut rings = Vec::with_capacity(raw_rings.len());
    for (ring_index, raw_ring) in raw_rings.iter().enumerate() {
        let ring = parse_ring(raw_ring, context)?;
        if ring.len() < 4 {
            return Err(GeometryError::RingTooShort {
                ring_index,
                len: ring.len(),
            });
        }
        rings.push(ring);
    }
    Ok(rings)
}

fn parse_ring(value: &Value, context: &'static str) -> Result<Ring, GeometryError> {
    let raw_positions = value
        .as_array()
        .ok_or(GeometryError::NotAnArray { context })?;
    let mut ring = Vec::with_capacity(raw_positions.len());
    for position in raw_positions {
        ring.push(parse_position(position, context)?);
    }
    Ok(ring)
}

fn parse_position(value: &Value, context: &'static str) -> Result<LonLat, GeometryError> {
    let raw = value
        .as_array()
        .ok_or(GeometryError::NotAnArray { context })?;
    if raw.len() != 2 {
        return Err(GeometryError::WrongPositionArity {
            context,
            len: raw.len(),
        });
    }
    let lon = raw[0]
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or(GeometryError::NonNumericCoordinate { context })?;
    let lat = raw[1]
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or(GeometryError::NonNumericCoordinate { context })?;
    Ok([lon, lat])
}
