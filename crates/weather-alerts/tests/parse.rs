//! Integration tests for `weather_alerts::parse`, covering every parsing
//! scenario `Agent Context/context/stages/S06-nws-alerts.md` names:
//! polygon, multipolygon, and malformed geometry -- plus the parser's
//! other documented contracts (status filtering, null geometry, and
//! parsing a real NWS fixture).

use serde_json::json;
use weather_alerts::model::{AlertGeometry, Certainty, MessageType, Severity, Urgency};
use weather_alerts::parse::{
    parse_feature_collection, AlertParseError, FeatureParseError, GeometryError,
};

/// A minimal, structurally-real `/alerts/active` feature, as a mutable
/// `serde_json::Value` a test can tweak one field on. Field values mirror
/// real shapes confirmed against a fetched NWS response (see
/// `tests/fixtures/README.md`), not guessed.
fn base_feature(id: &str) -> serde_json::Value {
    json!({
        "id": format!("https://api.weather.gov/alerts/{id}"),
        "type": "Feature",
        "geometry": null,
        "properties": {
            "@id": format!("https://api.weather.gov/alerts/{id}"),
            "id": id,
            "areaDesc": "Cleveland, OK; Canadian, OK",
            "references": [],
            "sent": "2026-09-12T18:00:00-05:00",
            "effective": "2026-09-12T18:00:00-05:00",
            "onset": "2026-09-12T18:00:00-05:00",
            "expires": "2026-09-12T19:00:00-05:00",
            "ends": null,
            "status": "Actual",
            "messageType": "Alert",
            "severity": "Severe",
            "certainty": "Observed",
            "urgency": "Immediate",
            "event": "Severe Thunderstorm Warning",
            "sender": "w-nws.webmaster@noaa.gov",
            "senderName": "NWS Norman OK",
            "headline": "Severe Thunderstorm Warning issued for Cleveland and Canadian Counties",
            "description": "A severe thunderstorm was located near Norman.",
            "instruction": "Move to an interior room on the lowest floor.",
        }
    })
}

fn feature_collection(features: Vec<serde_json::Value>) -> String {
    json!({
        "@context": [],
        "type": "FeatureCollection",
        "features": features,
        "title": "test",
        "updated": "2026-09-12T18:00:00-05:00",
    })
    .to_string()
}

// -- polygon / multipolygon / malformed geometry (the stage's named scenarios) --

#[test]
fn polygon_geometry_parses_correctly() {
    let mut feature = base_feature("urn:oid:test.polygon.1");
    feature["geometry"] = json!({
        "type": "Polygon",
        "coordinates": [[
            [-97.5, 35.2], [-97.4, 35.2], [-97.4, 35.3], [-97.5, 35.3], [-97.5, 35.2]
        ]]
    });
    let body = feature_collection(vec![feature]);

    let parsed = parse_feature_collection(&body).expect("valid FeatureCollection");
    assert_eq!(parsed.alerts.len(), 1);
    assert!(parsed.skipped.is_empty());

    match &parsed.alerts[0].geometry {
        Some(AlertGeometry::Polygon(rings)) => {
            assert_eq!(rings.len(), 1);
            assert_eq!(rings[0].len(), 5);
            assert_eq!(rings[0][0], [-97.5, 35.2]);
            assert_eq!(rings[0][4], [-97.5, 35.2], "ring should be closed");
        }
        other => panic!("expected Polygon, got {other:?}"),
    }
}

#[test]
fn multipolygon_geometry_parses_with_distinct_rings() {
    let mut feature = base_feature("urn:oid:test.multipolygon.1");
    // Two entirely separate polygons -- a common real shape for a single
    // CAP alert covering two disjoint storm cells/areas.
    feature["geometry"] = json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[
                [-97.5, 35.2], [-97.4, 35.2], [-97.4, 35.3], [-97.5, 35.3], [-97.5, 35.2]
            ]],
            [[
                [-96.0, 34.0], [-95.9, 34.0], [-95.9, 34.1], [-96.0, 34.1], [-96.0, 34.0]
            ]]
        ]
    });
    let body = feature_collection(vec![feature]);

    let parsed = parse_feature_collection(&body).expect("valid FeatureCollection");
    assert_eq!(parsed.alerts.len(), 1);

    match &parsed.alerts[0].geometry {
        Some(AlertGeometry::MultiPolygon(polygons)) => {
            assert_eq!(
                polygons.len(),
                2,
                "two distinct polygons must be preserved separately"
            );
            assert_eq!(polygons[0].len(), 1);
            assert_eq!(polygons[1].len(), 1);
            // The two polygons must not be merged/flattened into one ring
            // list -- confirm they cover disjoint coordinate ranges.
            assert_eq!(polygons[0][0][0], [-97.5, 35.2]);
            assert_eq!(polygons[1][0][0], [-96.0, 34.0]);
        }
        other => panic!("expected MultiPolygon, got {other:?}"),
    }
}

#[test]
fn null_geometry_is_not_an_error() {
    let feature = base_feature("urn:oid:test.nullgeom.1"); // geometry: null by default
    let body = feature_collection(vec![feature]);

    let parsed = parse_feature_collection(&body).expect("valid FeatureCollection");
    assert_eq!(parsed.alerts.len(), 1);
    assert!(parsed.skipped.is_empty());
    assert_eq!(parsed.alerts[0].geometry, None);
}

#[test]
fn malformed_geometry_garbage_coordinates_is_rejected() {
    let mut feature = base_feature("urn:oid:test.malformed.1");
    feature["geometry"] = json!({
        "type": "Polygon",
        "coordinates": [[
            ["not", "numbers"], [-97.4, 35.2], [-97.4, 35.3], [-97.5, 35.3]
        ]]
    });
    let body = feature_collection(vec![feature]);

    let parsed = parse_feature_collection(&body).expect("top-level FeatureCollection is valid");
    assert!(parsed.alerts.is_empty());
    assert_eq!(parsed.skipped.len(), 1);
    assert!(matches!(
        parsed.skipped[0].error,
        FeatureParseError::Geometry(GeometryError::NonNumericCoordinate { .. })
    ));
}

#[test]
fn malformed_geometry_wrong_nesting_depth_is_rejected() {
    let mut feature = base_feature("urn:oid:test.malformed.2");
    // A Polygon's coordinates must be an array of rings (array of arrays
    // of positions) -- this is a flat array of positions instead (missing
    // one level of nesting).
    feature["geometry"] = json!({
        "type": "Polygon",
        "coordinates": [-97.5, 35.2]
    });
    let body = feature_collection(vec![feature]);

    let parsed = parse_feature_collection(&body).expect("top-level FeatureCollection is valid");
    assert!(parsed.alerts.is_empty());
    assert_eq!(parsed.skipped.len(), 1);
    assert!(matches!(
        parsed.skipped[0].error,
        FeatureParseError::Geometry(GeometryError::NotAnArray { .. })
    ));
}

#[test]
fn malformed_geometry_ring_too_short_is_rejected() {
    let mut feature = base_feature("urn:oid:test.malformed.3");
    feature["geometry"] = json!({
        "type": "Polygon",
        // Only 3 positions -- a linear ring needs at least 4.
        "coordinates": [[[-97.5, 35.2], [-97.4, 35.2], [-97.4, 35.3]]]
    });
    let body = feature_collection(vec![feature]);

    let parsed = parse_feature_collection(&body).expect("top-level FeatureCollection is valid");
    assert!(parsed.alerts.is_empty());
    assert_eq!(parsed.skipped.len(), 1);
    assert!(matches!(
        parsed.skipped[0].error,
        FeatureParseError::Geometry(GeometryError::RingTooShort { len: 3, .. })
    ));
}

#[test]
fn malformed_geometry_unsupported_type_is_rejected() {
    let mut feature = base_feature("urn:oid:test.malformed.4");
    feature["geometry"] = json!({
        "type": "Point",
        "coordinates": [-97.5, 35.2]
    });
    let body = feature_collection(vec![feature]);

    let parsed = parse_feature_collection(&body).expect("top-level FeatureCollection is valid");
    assert!(parsed.alerts.is_empty());
    assert_eq!(parsed.skipped.len(), 1);
    assert!(matches!(
        parsed.skipped[0].error,
        FeatureParseError::Geometry(GeometryError::UnsupportedType(ref t)) if t == "Point"
    ));
}

/// The stage's explicit requirement: "one bad alert in a `FeatureCollection`
/// should not lose all the good ones."
#[test]
fn malformed_geometry_in_one_feature_does_not_lose_other_valid_features() {
    let good_1 = base_feature("urn:oid:test.batch.good1");
    let mut bad = base_feature("urn:oid:test.batch.bad");
    bad["geometry"] = json!({"type": "Polygon", "coordinates": "garbage"});
    let good_2 = base_feature("urn:oid:test.batch.good2");

    let body = feature_collection(vec![good_1, bad, good_2]);
    let parsed = parse_feature_collection(&body).expect("top-level FeatureCollection is valid");

    assert_eq!(
        parsed.alerts.len(),
        2,
        "both good features must still parse"
    );
    assert_eq!(
        parsed
            .alerts
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        vec!["urn:oid:test.batch.good1", "urn:oid:test.batch.good2"]
    );
    assert_eq!(parsed.skipped.len(), 1);
    assert_eq!(parsed.skipped[0].index, 1);
    assert_eq!(
        parsed.skipped[0].id.as_deref(),
        Some("urn:oid:test.batch.bad")
    );
}

// -- status filtering --------------------------------------------------------

#[test]
fn non_actual_status_is_filtered_not_errored() {
    let mut test_msg = base_feature("urn:oid:test.status.test");
    test_msg["properties"]["status"] = json!("Test");
    // Real "KEEPALIVE" test messages carry `severity`/`certainty`/`urgency`
    // "Unknown" and a null headline -- confirmed against a real national
    // snapshot; mirrored here even though status filtering happens before
    // those fields would otherwise need to parse.
    test_msg["properties"]["severity"] = json!("Unknown");
    test_msg["properties"]["certainty"] = json!("Unknown");
    test_msg["properties"]["urgency"] = json!("Unknown");
    test_msg["properties"]["headline"] = json!(null);

    let actual_msg = base_feature("urn:oid:test.status.actual");

    let body = feature_collection(vec![test_msg, actual_msg]);
    let parsed = parse_feature_collection(&body).expect("valid FeatureCollection");

    assert_eq!(parsed.alerts.len(), 1);
    assert_eq!(parsed.alerts[0].id, "urn:oid:test.status.actual");
    assert_eq!(parsed.filtered_non_actual, 1);
    assert!(parsed.skipped.is_empty(), "filtering is not an error");
}

#[test]
fn unknown_severity_certainty_urgency_are_valid_not_errors() {
    let mut feature = base_feature("urn:oid:test.unknown-enums");
    feature["properties"]["severity"] = json!("Unknown");
    feature["properties"]["certainty"] = json!("Unknown");
    feature["properties"]["urgency"] = json!("Unknown");

    let body = feature_collection(vec![feature]);
    let parsed = parse_feature_collection(&body).expect("valid FeatureCollection");

    assert_eq!(parsed.alerts.len(), 1);
    assert_eq!(parsed.alerts[0].severity, Severity::Unknown);
    assert_eq!(parsed.alerts[0].certainty, Certainty::Unknown);
    assert_eq!(parsed.alerts[0].urgency, Urgency::Unknown);
}

#[test]
fn null_headline_description_instruction_are_valid_not_errors() {
    let mut feature = base_feature("urn:oid:test.nulls");
    feature["properties"]["headline"] = json!(null);
    feature["properties"]["description"] = json!(null);
    feature["properties"]["instruction"] = json!(null);

    let body = feature_collection(vec![feature]);
    let parsed = parse_feature_collection(&body).expect("valid FeatureCollection");

    assert_eq!(parsed.alerts.len(), 1);
    assert_eq!(parsed.alerts[0].headline, None);
    assert_eq!(parsed.alerts[0].description, None);
    assert_eq!(parsed.alerts[0].instruction, None);
}

#[test]
fn area_desc_splits_on_semicolon_not_comma() {
    let mut feature = base_feature("urn:oid:test.areadesc");
    feature["properties"]["areaDesc"] = json!("Lipscomb, TX; Ochiltree, TX");

    let body = feature_collection(vec![feature]);
    let parsed = parse_feature_collection(&body).expect("valid FeatureCollection");

    assert_eq!(
        parsed.alerts[0].affected_areas,
        vec!["Lipscomb, TX".to_string(), "Ochiltree, TX".to_string()]
    );
}

// -- top-level malformed input ------------------------------------------------

#[test]
fn invalid_json_is_a_hard_error() {
    let result = parse_feature_collection("{not json");
    assert!(matches!(result, Err(AlertParseError::InvalidJson(_))));
}

#[test]
fn non_feature_collection_type_is_a_hard_error() {
    let body = json!({"type": "Feature", "features": []}).to_string();
    let result = parse_feature_collection(&body);
    assert!(matches!(
        result,
        Err(AlertParseError::WrongTopLevelType { .. })
    ));
}

#[test]
fn missing_features_array_is_a_hard_error() {
    let body = json!({"type": "FeatureCollection"}).to_string();
    let result = parse_feature_collection(&body);
    assert!(matches!(result, Err(AlertParseError::MissingFeatures)));
}

// -- real NWS fixture data ----------------------------------------------------

const REAL_OK_FIXTURE: &str = include_str!("fixtures/alerts-active-ok-20260912.json");

#[test]
fn parses_real_nws_fixture_without_errors() {
    let parsed = parse_feature_collection(REAL_OK_FIXTURE).expect("real fixture is valid JSON");
    assert!(
        parsed.skipped.is_empty(),
        "real fixture should have no malformed features: {:?}",
        parsed.skipped
    );
    assert_eq!(
        parsed.alerts.len(),
        13,
        "fixture had 13 features at fetch time"
    );
}

/// Ground truth: feature index 2 of the real fixture is a `messageType:
/// "Update"` for a Severe Thunderstorm Warning, referencing exactly one
/// prior message.
#[test]
fn real_fixture_update_message_carries_references() {
    let parsed = parse_feature_collection(REAL_OK_FIXTURE).expect("real fixture is valid JSON");
    let update = parsed
        .alerts
        .iter()
        .find(|a| a.id == "urn:oid:2.49.0.1.840.0.0b480821d97d7b520d561cbea7e16c04cd1695ef.002.1")
        .expect("known real Update message present in fixture");

    assert_eq!(update.message_type, MessageType::Update);
    assert_eq!(update.event, "Severe Thunderstorm Warning");
    assert_eq!(update.references.len(), 1);
    assert_eq!(
        update.references[0].identifier,
        "urn:oid:2.49.0.1.840.0.e534d55c7611fc50f6f04105dcc6018708138198.001.1"
    );
    assert!(matches!(update.geometry, Some(AlertGeometry::Polygon(_))));
}

#[test]
fn real_fixture_id_is_bare_identifier_not_the_api_url() {
    let parsed = parse_feature_collection(REAL_OK_FIXTURE).expect("real fixture is valid JSON");
    for alert in &parsed.alerts {
        assert!(
            alert.id.starts_with("urn:oid:"),
            "expected bare CAP identifier, got {:?}",
            alert.id
        );
        assert!(
            !alert.id.starts_with("https://"),
            "id must not be the dereferenceable API URL: {:?}",
            alert.id
        );
    }
}
