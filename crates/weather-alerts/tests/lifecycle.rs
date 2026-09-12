//! Integration tests for `weather_alerts::store::AlertStore`, covering
//! every lifecycle scenario `Agent Context/context/stages/S06-nws-alerts.md`
//! names: initial warning, update, replacement, cancellation, expiration --
//! plus the stage's explicit race-safety rule ("do not let polling races
//! incorrectly resurrect or remove alerts").
//!
//! See `docs/adr/0010-nws-alert-lifecycle-reconciliation.md` for the full
//! design this suite proves.

use radar_types::Timestamp;
use weather_alerts::model::{Alert, AlertReference, Certainty, MessageType, Severity, Urgency};
use weather_alerts::store::{AlertChange, AlertStore, ExpiryReason};

fn ts(millis: i64) -> Timestamp {
    Timestamp::from_epoch_millis(millis)
}

/// A minimal, otherwise-valid alert for lifecycle testing -- only the
/// fields these tests actually vary are parameters; everything else is a
/// fixed, sensible default.
fn make_alert(
    id: &str,
    message_type: MessageType,
    references: Vec<AlertReference>,
    event: &str,
    expires_millis: i64,
) -> Alert {
    Alert {
        id: id.to_string(),
        message_type,
        references,
        event: event.to_string(),
        severity: Severity::Severe,
        certainty: Certainty::Observed,
        urgency: Urgency::Immediate,
        sender: "NWS Norman OK".to_string(),
        sender_id: "w-nws.webmaster@noaa.gov".to_string(),
        issued: ts(0),
        effective: ts(0),
        expires: ts(expires_millis),
        ends: None,
        headline: Some(format!("{event} issued")),
        description: Some("A hazardous weather event was observed.".to_string()),
        instruction: Some("Take shelter.".to_string()),
        affected_areas: vec!["Cleveland, OK".to_string()],
        geometry: None,
    }
}

fn reference_to(identifier: &str) -> AlertReference {
    AlertReference {
        identifier: identifier.to_string(),
        sender: "w-nws.webmaster@noaa.gov".to_string(),
        sent: ts(0),
    }
}

// -- initial warning ----------------------------------------------------------

#[test]
fn initial_warning_introduces_a_new_active_alert() {
    let mut store = AlertStore::new();
    let alert = make_alert(
        "urn:oid:test.1",
        MessageType::Alert,
        vec![],
        "Severe Thunderstorm Warning",
        10_000,
    );

    let changes = store.ingest_poll(vec![alert.clone()], ts(0));

    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], AlertChange::New { key, alert: a }
        if key == "urn:oid:test.1" && a.id == "urn:oid:test.1"));

    let active = store.active_alerts(ts(0));
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, "urn:oid:test.1");
    assert_eq!(store.held_count(), 1);
}

// -- update ---------------------------------------------------------------------

#[test]
fn update_message_replaces_held_alert_content_under_the_same_stable_key() {
    let mut store = AlertStore::new();
    let original = make_alert(
        "urn:oid:test.2.v1",
        MessageType::Alert,
        vec![],
        "Severe Thunderstorm Warning",
        10_000,
    );
    store.ingest_poll(vec![original], ts(0));

    let update = make_alert(
        "urn:oid:test.2.v2",
        MessageType::Update,
        vec![reference_to("urn:oid:test.2.v1")],
        "Severe Thunderstorm Warning",
        20_000, // extended expiration, as a real continuation update would carry
    );
    let changes = store.ingest_poll(vec![update], ts(5_000));

    assert_eq!(changes.len(), 1);
    match &changes[0] {
        AlertChange::Updated { key, alert } => {
            assert_eq!(
                key, "urn:oid:test.2.v1",
                "stable key must not change across an update"
            );
            assert_eq!(
                alert.id, "urn:oid:test.2.v2",
                "content must reflect the new message"
            );
            assert_eq!(alert.expires, ts(20_000));
        }
        other => panic!("expected Updated, got {other:?}"),
    }

    assert_eq!(
        store.held_count(),
        1,
        "update must not create a second held alert"
    );
    let held = store
        .get("urn:oid:test.2.v1")
        .expect("stable key still resolves");
    assert_eq!(held.id, "urn:oid:test.2.v2");
    assert_eq!(held.expires, ts(20_000));
}

#[test]
fn a_second_update_chains_off_the_first_updates_message_id_not_the_original() {
    let mut store = AlertStore::new();
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.chain.v1",
            MessageType::Alert,
            vec![],
            "Flood Warning",
            10_000,
        )],
        ts(0),
    );
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.chain.v2",
            MessageType::Update,
            vec![reference_to("urn:oid:test.chain.v1")],
            "Flood Warning",
            20_000,
        )],
        ts(1_000),
    );
    // Real NWS `references` link to the *immediately preceding* message,
    // so a second update must reference v2, not v1.
    let changes = store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.chain.v3",
            MessageType::Update,
            vec![reference_to("urn:oid:test.chain.v2")],
            "Flood Warning",
            30_000,
        )],
        ts(2_000),
    );

    assert_eq!(changes.len(), 1);
    assert!(
        matches!(&changes[0], AlertChange::Updated { key, .. } if key == "urn:oid:test.chain.v1")
    );
    assert_eq!(store.held_count(), 1);
    assert_eq!(
        store.get("urn:oid:test.chain.v1").unwrap().id,
        "urn:oid:test.chain.v3"
    );
}

// -- replacement ------------------------------------------------------------------

/// S06 lists "replacement" distinctly from "update". Per this crate's
/// documented design (see `crate::store`'s module docs), a message whose
/// `messageType` is `Alert` (not `Update`) but whose `references` still
/// names a currently-held alert is treated identically to an `Update`:
/// content replacement under the same stable key. This is the mechanism
/// this crate uses for a same-hazard "replacement" issued as a fresh
/// `Alert`-type message rather than an `Update`.
#[test]
fn alert_type_message_referencing_a_held_alert_is_treated_as_a_replacement() {
    let mut store = AlertStore::new();
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.replace.v1",
            MessageType::Alert,
            vec![],
            "Severe Thunderstorm Warning",
            10_000,
        )],
        ts(0),
    );

    let replacement = make_alert(
        "urn:oid:test.replace.v2",
        MessageType::Alert, // NOT Update -- this is what distinguishes "replacement"
        vec![reference_to("urn:oid:test.replace.v1")],
        "Tornado Warning", // upgraded hazard type
        20_000,
    );
    let changes = store.ingest_poll(vec![replacement], ts(1_000));

    assert_eq!(changes.len(), 1);
    match &changes[0] {
        AlertChange::Updated { key, alert } => {
            assert_eq!(key, "urn:oid:test.replace.v1");
            assert_eq!(alert.event, "Tornado Warning");
        }
        other => panic!("expected Updated (replacement), got {other:?}"),
    }
    assert_eq!(
        store.held_count(),
        1,
        "replacement must not create a second held alert"
    );
}

// -- cancellation ------------------------------------------------------------------

#[test]
fn cancel_message_confidently_removes_the_referenced_alert() {
    let mut store = AlertStore::new();
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.3",
            MessageType::Alert,
            vec![],
            "Tornado Warning",
            10_000,
        )],
        ts(0),
    );
    assert_eq!(store.held_count(), 1);

    let cancel = make_alert(
        "urn:oid:test.3.cancel",
        MessageType::Cancel,
        vec![reference_to("urn:oid:test.3")],
        "Tornado Warning",
        10_000,
    );
    let changes = store.ingest_poll(vec![cancel], ts(1_000));

    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], AlertChange::Cancelled { key, .. } if key == "urn:oid:test.3"));
    assert_eq!(store.held_count(), 0);
    assert!(store.active_alerts(ts(1_000)).is_empty());
    assert!(store.get("urn:oid:test.3").is_none());
}

#[test]
fn cancel_referencing_nothing_currently_held_is_a_silent_no_op() {
    let mut store = AlertStore::new();
    let cancel = make_alert(
        "urn:oid:test.4.cancel",
        MessageType::Cancel,
        vec![reference_to("urn:oid:test.4.never-seen")],
        "Flash Flood Warning",
        10_000,
    );
    let changes = store.ingest_poll(vec![cancel], ts(0));

    assert!(changes.is_empty());
    assert_eq!(store.held_count(), 0);
}

#[test]
fn a_cancel_message_is_never_itself_held_as_an_active_alert() {
    let mut store = AlertStore::new();
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.5",
            MessageType::Alert,
            vec![],
            "Tornado Warning",
            10_000,
        )],
        ts(0),
    );
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.5.cancel",
            MessageType::Cancel,
            vec![reference_to("urn:oid:test.5")],
            "Tornado Warning",
            10_000,
        )],
        ts(1_000),
    );

    assert_eq!(store.held_count(), 0);
    assert!(store.get("urn:oid:test.5.cancel").is_none());
}

// -- expiration (time-based, checked at query time) --------------------------------

#[test]
fn expiration_removes_an_alert_by_time_even_with_no_new_poll() {
    let mut store = AlertStore::new();
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.6",
            MessageType::Alert,
            vec![],
            "Winter Storm Warning",
            10_000,
        )],
        ts(0),
    );
    assert_eq!(store.active_alerts(ts(5_000)).len(), 1, "not yet expired");

    // No new poll at all -- purely a query at a later `now`, past `expires`.
    let active_after_expiry = store.active_alerts(ts(10_001));
    assert!(
        active_after_expiry.is_empty(),
        "active_alerts must apply the expiration check at query time, not just at the last poll"
    );
    // But the store has not necessarily *removed* it yet -- `active_alerts`
    // is a pure, non-mutating read (see its docs).
    assert_eq!(store.held_count(), 1);

    let changes = store.expire_stale(ts(10_001));
    assert_eq!(changes.len(), 1);
    assert!(matches!(
        &changes[0],
        AlertChange::Expired { key, reason: ExpiryReason::TimeExpired, .. } if key == "urn:oid:test.6"
    ));
    assert_eq!(store.held_count(), 0, "expire_stale actually removes it");
}

#[test]
fn ingest_poll_also_applies_time_based_expiration() {
    let mut store = AlertStore::new();
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.7",
            MessageType::Alert,
            vec![],
            "Flood Warning",
            10_000,
        )],
        ts(0),
    );

    // A later poll response that happens not to include this alert at all
    // (e.g. it has genuinely expired and NWS stopped listing it) --
    // `now` is past `expires`, so this must be reported as `TimeExpired`,
    // not `AbsentFromPolls` (the more specific, authoritative reason wins).
    let changes = store.ingest_poll(vec![], ts(10_001));

    assert_eq!(changes.len(), 1);
    assert!(matches!(
        &changes[0],
        AlertChange::Expired {
            reason: ExpiryReason::TimeExpired,
            ..
        }
    ));
    assert_eq!(store.held_count(), 0);
}

// -- race safety: the stage's core rule -----------------------------------------

/// The stage's explicit requirement, tested directly: "a held, non-expired
/// alert that's simply missing from one poll (no Cancel, not expired) must
/// still be reported as active immediately after that poll."
#[test]
fn alert_missing_from_one_poll_with_no_cancel_and_not_expired_is_still_active() {
    let mut store = AlertStore::new();
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.8",
            MessageType::Alert,
            vec![],
            "Tornado Warning",
            1_000_000,
        )],
        ts(0),
    );

    // Simulates a transient empty/partial response, or the alert simply
    // not appearing this one time for a reason other than cancellation or
    // expiration.
    let changes = store.ingest_poll(vec![], ts(1_000));

    assert!(
        changes.is_empty(),
        "a single missed poll must not itself produce any change: {changes:?}"
    );
    assert_eq!(store.held_count(), 1);
    let active = store.active_alerts(ts(1_000));
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, "urn:oid:test.8");
}

#[test]
fn alert_reappearing_after_a_missed_poll_resets_the_absence_counter() {
    let mut store = AlertStore::with_absence_limit(2);
    let alert = make_alert(
        "urn:oid:test.9",
        MessageType::Alert,
        vec![],
        "Tornado Warning",
        1_000_000,
    );
    store.ingest_poll(vec![alert.clone()], ts(0));

    // Missing once (absence = 1, limit = 2 -- not yet removed).
    store.ingest_poll(vec![], ts(1_000));
    assert_eq!(store.held_count(), 1);

    // Reappears (idempotent repeat of the exact same message) -- absence
    // counter must reset to 0, not carry over.
    let changes = store.ingest_poll(vec![alert.clone()], ts(2_000));
    assert!(
        changes.is_empty(),
        "an idempotent repeat of already-known content must not produce a spurious New/Updated change"
    );
    assert_eq!(store.held_count(), 1);

    // Missing again -- if the counter had not reset, this would be
    // absence #2 (>= limit) and wrongly remove the alert. It must not.
    let changes = store.ingest_poll(vec![], ts(3_000));
    assert!(
        changes.is_empty(),
        "absence counter must have reset on reappearance, not accumulated across the gap: {changes:?}"
    );
    assert_eq!(store.held_count(), 1);
}

/// The secondary safety net: several *independent, consecutive* missed
/// polls (still no explicit Cancel, still not time-expired) eventually do
/// remove a held alert, since NWS is known to sometimes stop listing an
/// alert without ever sending an explicit Cancel. Uses a small
/// `absence_limit` override so the test does not need many polls to prove
/// the mechanism.
#[test]
fn alert_absent_for_n_consecutive_polls_is_removed_as_a_safety_net() {
    let mut store = AlertStore::with_absence_limit(3);
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.10",
            MessageType::Alert,
            vec![],
            "Special Weather Statement",
            1_000_000,
        )],
        ts(0),
    );

    // Absences 1 and 2: still held, no change reported.
    assert!(store.ingest_poll(vec![], ts(1_000)).is_empty());
    assert_eq!(store.held_count(), 1);
    assert!(store.ingest_poll(vec![], ts(2_000)).is_empty());
    assert_eq!(
        store.held_count(),
        1,
        "must survive 2 consecutive absences under a limit of 3"
    );

    // Absence 3 meets the limit: now, and only now, is it removed.
    let changes = store.ingest_poll(vec![], ts(3_000));
    assert_eq!(changes.len(), 1);
    assert!(matches!(
        &changes[0],
        AlertChange::Expired { key, reason: ExpiryReason::AbsentFromPolls, .. } if key == "urn:oid:test.10"
    ));
    assert_eq!(store.held_count(), 0);
}

// -- drain_changes / active_alerts_with_keys ---------------------------------------

#[test]
fn drain_changes_returns_everything_since_the_last_drain_then_empties() {
    let mut store = AlertStore::new();
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.11",
            MessageType::Alert,
            vec![],
            "Flood Advisory",
            10_000,
        )],
        ts(0),
    );
    store.expire_stale(ts(0)); // not yet expired, but exercises accumulation from two call sites

    let drained = store.drain_changes();
    assert_eq!(drained.len(), 1);
    assert!(matches!(&drained[0], AlertChange::New { .. }));

    assert!(
        store.drain_changes().is_empty(),
        "must be empty immediately after a drain"
    );
}

#[test]
fn active_alerts_with_keys_pairs_stable_keys_with_current_content() {
    let mut store = AlertStore::new();
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.12.v1",
            MessageType::Alert,
            vec![],
            "Heat Advisory",
            10_000,
        )],
        ts(0),
    );
    store.ingest_poll(
        vec![make_alert(
            "urn:oid:test.12.v2",
            MessageType::Update,
            vec![reference_to("urn:oid:test.12.v1")],
            "Heat Advisory",
            20_000,
        )],
        ts(1_000),
    );

    let entries = store.active_alerts_with_keys(ts(1_000));
    assert_eq!(entries.len(), 1);
    let (key, alert) = entries[0];
    assert_eq!(
        key, "urn:oid:test.12.v1",
        "GeoJSON feature id must be the stable key"
    );
    assert_eq!(
        alert.id, "urn:oid:test.12.v2",
        "content must be the latest message"
    );
}
