//! Poll-race-safe alert lifecycle state management.
//!
//! See `docs/adr/0010-nws-alert-lifecycle-reconciliation.md` for the full
//! design rationale. This module's docs cover the mechanics; the ADR
//! covers *why*.
//!
//! # Identity across updates: stable key vs. latest message id
//!
//! Every CAP message -- including an `Update` that only refreshes an
//! existing warning's polygon/wording -- has its own, unique
//! [`crate::model::Alert::id`]. A naive "key everything by `Alert::id`"
//! design would therefore make a UI-visible feature (e.g. a MapLibre
//! polygon layer entry) change identity on every single update, which is
//! both unnecessary flicker for a UI and loses the fact that it is "the
//! same warning, continued."
//!
//! [`AlertStore`] instead separates two concepts:
//! - **[`AlertKey`]**: a stable identity, chosen once when an alert is
//!   first introduced to the store (its first message's own id) and never
//!   changed for the life of that held alert, even across many updates.
//!   This is what a consumer (a map layer, a details panel) should key its
//!   own UI state on.
//! - **the held alert's current content's own `id`**: rotates to the
//!   latest message's id on every update, since that is what a *future*
//!   update or cancel's `references[].identifier` will name next (CAP's
//!   `references` links to the immediately-preceding message, not to the
//!   original first message in a long update chain).
//!
//! An internal secondary index (`latest message id -> stable key`) is what
//! makes an incoming `Update`/`Cancel`'s `references` resolvable back to
//! the right held entry regardless of how many updates have happened
//! since the alert was first introduced.

use std::collections::HashMap;

use radar_types::Timestamp;

use crate::model::{Alert, MessageType};

/// A stable identity for a held alert, chosen once (its first message's
/// own [`crate::model::Alert::id`]) and unchanged across later updates.
/// See the module docs for why this is not simply "the current message's
/// id".
pub type AlertKey = String;

/// The default number of consecutive, independently-successful polls an
/// alert may be absent from (with no explicit `Cancel` and before its own
/// `expires` time) before [`AlertStore`] treats the absence itself as a
/// removal signal. See `docs/adr/0010-...` for why 3 was chosen.
pub const DEFAULT_ABSENCE_LIMIT: u32 = 3;

/// Why a held alert was removed from the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpiryReason {
    /// The alert's own `expires` timestamp is no longer in the future as
    /// of the `now` passed to [`AlertStore::ingest_poll`] or
    /// [`AlertStore::expire_stale`].
    TimeExpired,
    /// The alert was absent (no matching `id` observed) from the store's
    /// configured absence limit (see [`AlertStore::with_absence_limit`],
    /// default [`DEFAULT_ABSENCE_LIMIT`]) worth of consecutive
    /// [`AlertStore::ingest_poll`] calls in a row, with no explicit
    /// `Cancel` ever referencing it and its `expires` time not yet
    /// reached. See the ADR for why this requires *multiple independent
    /// polls* to agree, not just one.
    AbsentFromPolls,
}

/// One lifecycle event produced by [`AlertStore::ingest_poll`] or
/// [`AlertStore::expire_stale`], and accumulated for
/// [`AlertStore::drain_changes`].
#[derive(Debug, Clone, PartialEq)]
pub enum AlertChange {
    /// A previously-unseen alert appeared.
    New { key: AlertKey, alert: Alert },
    /// A currently-held alert's content was replaced by a newer message
    /// that referenced it (an `Update`, or an `Alert`-type "replacement"
    /// message that still carries a matching `references` entry -- see
    /// the ADR for why both are handled identically). `key` is unchanged
    /// from when the alert was first introduced.
    Updated { key: AlertKey, alert: Alert },
    /// A currently-held alert was explicitly cancelled by a `Cancel`
    /// message referencing it. `alert` is the content as last held, for a
    /// UI that wants to show what was cancelled.
    Cancelled { key: AlertKey, alert: Alert },
    /// A currently-held alert was removed without an explicit `Cancel`;
    /// see [`ExpiryReason`] for why.
    Expired {
        key: AlertKey,
        alert: Alert,
        reason: ExpiryReason,
    },
}

impl AlertChange {
    /// The stable key this change concerns, regardless of variant.
    pub fn key(&self) -> &AlertKey {
        match self {
            AlertChange::New { key, .. }
            | AlertChange::Updated { key, .. }
            | AlertChange::Cancelled { key, .. }
            | AlertChange::Expired { key, .. } => key,
        }
    }
}

/// One currently-held alert's bookkeeping state.
struct HeldAlert {
    /// The most recent message's own id -- what a future
    /// `references[].identifier` will name to link to this held alert.
    latest_message_id: String,
    /// The alert's content, as of the most recently applied message.
    alert: Alert,
    /// Consecutive [`AlertStore::ingest_poll`] calls in a row in which no
    /// incoming message's id matched `latest_message_id`. Reset to 0 on
    /// any poll where it is seen (new content or an idempotent repeat of
    /// the same message).
    consecutive_absences: u32,
}

/// Poll-race-safe holder of currently-active NWS alerts.
///
/// Feed each successive `/alerts/active` poll's parsed alerts to
/// [`ingest_poll`](AlertStore::ingest_poll). Query "what's active right
/// now" via [`active_alerts`](AlertStore::active_alerts) (time-filtered at
/// query time, not just at the last poll) and consume incremental UI
/// updates via [`drain_changes`](AlertStore::drain_changes).
///
/// See the module docs for the stable-key design and
/// `docs/adr/0010-nws-alert-lifecycle-reconciliation.md` for the full
/// removal-safety rationale.
pub struct AlertStore {
    held: HashMap<AlertKey, HeldAlert>,
    /// Secondary index: a message id (the latest one seen for some held
    /// alert) -> that alert's stable key. Used to resolve incoming
    /// `references[].identifier` values back to the right held entry.
    index_by_latest_message_id: HashMap<String, AlertKey>,
    absence_limit: u32,
    /// Changes not yet returned by `drain_changes`, accumulated by every
    /// mutating call.
    pending_changes: Vec<AlertChange>,
}

impl Default for AlertStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertStore {
    /// A new, empty store using [`DEFAULT_ABSENCE_LIMIT`].
    pub fn new() -> Self {
        Self::with_absence_limit(DEFAULT_ABSENCE_LIMIT)
    }

    /// A new, empty store with an explicit absence-limit override (mainly
    /// for tests that want to exercise the absence-based safety net
    /// without waiting through several real polls, or a caller with a
    /// deliberately different polling cadence than this crate's default
    /// assumption -- see the ADR).
    pub fn with_absence_limit(absence_limit: u32) -> Self {
        Self {
            held: HashMap::new(),
            index_by_latest_message_id: HashMap::new(),
            absence_limit,
            pending_changes: Vec::new(),
        }
    }

    /// The number of alerts currently held (active or not-yet-expired;
    /// does not itself filter by `now` -- see
    /// [`active_alerts`](Self::active_alerts) for that).
    pub fn held_count(&self) -> usize {
        self.held.len()
    }

    /// The currently-held content for `key`, regardless of whether it has
    /// time-expired as of any particular `now` (mainly for tests/
    /// diagnostics; UI consumers should use
    /// [`active_alerts`](Self::active_alerts)).
    pub fn get(&self, key: &str) -> Option<&Alert> {
        self.held.get(key).map(|h| &h.alert)
    }

    /// Every currently-held alert whose `expires` is still in the future
    /// as of `now`, in unspecified order.
    ///
    /// This check is applied here, at query time -- not only when a poll
    /// last ran -- so an alert that crosses its own `expires` between two
    /// polls (or between two calls to this method with no intervening
    /// poll at all) stops being reported as active immediately, rather
    /// than lingering until the next network poll happens to notice. This
    /// method never mutates the store or removes anything; call
    /// [`expire_stale`](Self::expire_stale) to actually reconcile
    /// (remove + report) time-based expirations as changes.
    pub fn active_alerts(&self, now: Timestamp) -> Vec<&Alert> {
        self.held
            .values()
            .filter(|h| h.alert.expires > now)
            .map(|h| &h.alert)
            .collect()
    }

    /// Same filtering as [`active_alerts`](Self::active_alerts), paired
    /// with each alert's stable [`AlertKey`] -- used by `crate::json`'s
    /// GeoJSON output, where the stable key (not the alert's own,
    /// update-rotating `id`) is what a map layer should use as each
    /// feature's persistent id.
    pub fn active_alerts_with_keys(&self, now: Timestamp) -> Vec<(&AlertKey, &Alert)> {
        self.held
            .iter()
            .filter(|(_, h)| h.alert.expires > now)
            .map(|(k, h)| (k, &h.alert))
            .collect()
    }

    /// Drain and return every [`AlertChange`] accumulated since the last
    /// call to this method (by [`ingest_poll`](Self::ingest_poll) and/or
    /// [`expire_stale`](Self::expire_stale)), for a UI that wants to react
    /// incrementally instead of re-rendering the full active set on every
    /// poll.
    pub fn drain_changes(&mut self) -> Vec<AlertChange> {
        std::mem::take(&mut self.pending_changes)
    }

    /// Reconcile any held alert whose `expires` has passed as of `now`,
    /// removing it and recording an [`AlertChange::Expired`] (reason
    /// [`ExpiryReason::TimeExpired`]) for both this call's return value
    /// and [`drain_changes`](Self::drain_changes).
    ///
    /// Safe and cheap to call frequently between polls (e.g. once per UI
    /// render tick) purely to keep the *change stream* -- not just
    /// [`active_alerts`](Self::active_alerts)'s own snapshot -- current
    /// with real time, independent of network poll cadence.
    pub fn expire_stale(&mut self, now: Timestamp) -> Vec<AlertChange> {
        let changes = self.expire_stale_without_draining(now);
        self.pending_changes.extend(changes.clone());
        changes
    }

    /// Ingest one successful poll's parsed alerts (the full current
    /// `/alerts/active` response, not a delta).
    ///
    /// Applies, in order: explicit `Cancel`s (authoritative removal of
    /// whatever they reference), `Update`/`Alert`-with-matching-`references`
    /// content replacement (see the module docs on "replacement"),
    /// genuinely new alerts, per-held-alert absence bookkeeping (the
    /// race-safety net -- see the ADR), and finally the same time-based
    /// expiration [`expire_stale`](Self::expire_stale) performs, so a
    /// single call fully reconciles one poll. Returns this call's changes
    /// (also accumulated for [`drain_changes`](Self::drain_changes)).
    ///
    /// `now` should be the current wall-clock time (UTC) at the moment
    /// this poll's response was received -- used only for the time-based
    /// expiration check, not for anything absence-related.
    pub fn ingest_poll(&mut self, alerts: Vec<Alert>, now: Timestamp) -> Vec<AlertChange> {
        let mut changes = Vec::new();
        // Every message id seen in *this* poll's response, regardless of
        // messageType -- used below to reset/advance absence counters.
        let mut seen_message_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for incoming in alerts {
            seen_message_ids.insert(incoming.id.clone());

            match incoming.message_type {
                MessageType::Cancel => {
                    self.apply_cancel(&incoming, &mut changes);
                }
                MessageType::Update | MessageType::Alert => {
                    self.apply_new_or_update(incoming, &mut changes);
                }
            }
        }

        // Absence bookkeeping: any held alert whose latest known message
        // id was not observed in this poll gets its absence counter
        // advanced; if it now meets/exceeds the absence limit (with no
        // explicit Cancel above already having removed it), that is the
        // secondary safety-net removal. A held alert *is* observed this
        // poll iff `seen_message_ids` contains its `latest_message_id` --
        // true both for an exact idempotent repeat and for an entry this
        // same call already rotated via `apply_new_or_update` (which
        // rotates `latest_message_id` to `incoming.id`, itself inserted
        // into `seen_message_ids` above).
        let mut newly_absent_beyond_limit = Vec::new();
        for (key, held) in self.held.iter_mut() {
            if seen_message_ids.contains(&held.latest_message_id) {
                held.consecutive_absences = 0;
            } else {
                held.consecutive_absences += 1;
                if held.consecutive_absences >= self.absence_limit {
                    newly_absent_beyond_limit.push(key.clone());
                }
            }
        }
        for key in newly_absent_beyond_limit {
            if let Some(held) = self.remove_held(&key) {
                changes.push(AlertChange::Expired {
                    key,
                    alert: held.alert,
                    reason: ExpiryReason::AbsentFromPolls,
                });
            }
        }

        // Finally, the same time-based expiration check `expire_stale`
        // performs on its own, so one `ingest_poll` call fully reconciles
        // this poll rather than requiring a separate `expire_stale` call
        // right after every poll.
        changes.extend(self.expire_stale_without_draining(now));

        self.pending_changes.extend(changes.clone());
        changes
    }

    /// Shared implementation for the time-based expiration sweep, used by
    /// both the public [`expire_stale`](Self::expire_stale) (which also
    /// records into `pending_changes` itself) and
    /// [`ingest_poll`](Self::ingest_poll) (which records the combined
    /// result once at the end, to keep one `ingest_poll` call's `changes`
    /// return value complete without double-appending to
    /// `pending_changes`).
    fn expire_stale_without_draining(&mut self, now: Timestamp) -> Vec<AlertChange> {
        let expired_keys: Vec<AlertKey> = self
            .held
            .iter()
            .filter(|(_, h)| h.alert.expires <= now)
            .map(|(k, _)| k.clone())
            .collect();
        let mut changes = Vec::with_capacity(expired_keys.len());
        for key in expired_keys {
            if let Some(held) = self.remove_held(&key) {
                changes.push(AlertChange::Expired {
                    key,
                    alert: held.alert,
                    reason: ExpiryReason::TimeExpired,
                });
            }
        }
        changes
    }

    /// Apply an explicit `Cancel` message: authoritative, immediate
    /// removal of every held alert it references. A `Cancel` referencing
    /// nothing currently held (e.g. this store never saw the original, or
    /// it already expired/was removed) is a silent no-op -- there is
    /// nothing to cancel, and that is not an error.
    fn apply_cancel(&mut self, incoming: &Alert, changes: &mut Vec<AlertChange>) {
        for reference in &incoming.references {
            if let Some(key) = self
                .index_by_latest_message_id
                .get(&reference.identifier)
                .cloned()
            {
                if let Some(held) = self.remove_held(&key) {
                    changes.push(AlertChange::Cancelled {
                        key,
                        alert: held.alert,
                    });
                }
            }
        }
    }

    /// Apply an `Alert`- or `Update`-type message: if any of its
    /// `references` names a currently-held alert's latest message id,
    /// this is a content replacement for that held alert (stable key
    /// unchanged, latest message id rotated -- see module docs). A
    /// message whose own id already *is* a held alert's latest message id
    /// (no references needed) is treated as an idempotent repeat of
    /// already-known content. Otherwise, it is a genuinely new alert.
    fn apply_new_or_update(&mut self, incoming: Alert, changes: &mut Vec<AlertChange>) {
        let matched_key = incoming
            .references
            .iter()
            .find_map(|reference| self.index_by_latest_message_id.get(&reference.identifier))
            .cloned();

        if let Some(key) = matched_key {
            if let Some(held) = self.held.get_mut(&key) {
                self.index_by_latest_message_id
                    .remove(&held.latest_message_id);
                held.latest_message_id = incoming.id.clone();
                held.consecutive_absences = 0;
                held.alert = incoming.clone();
                self.index_by_latest_message_id
                    .insert(held.latest_message_id.clone(), key.clone());
                changes.push(AlertChange::Updated {
                    key,
                    alert: incoming,
                });
                return;
            }
        }

        if let Some(existing_key) = self.index_by_latest_message_id.get(&incoming.id).cloned() {
            // Idempotent repeat of a message we already hold as the
            // latest content for some alert (e.g. the same poll response
            // observed twice in a row with no new message issued) --
            // refresh content (a no-op in practice) and reset the
            // absence counter, but do not report a spurious change.
            if let Some(held) = self.held.get_mut(&existing_key) {
                held.alert = incoming;
                held.consecutive_absences = 0;
            }
            return;
        }

        let key: AlertKey = incoming.id.clone();
        self.insert_new(key.clone(), incoming.clone());
        changes.push(AlertChange::New {
            key,
            alert: incoming,
        });
    }

    fn insert_new(&mut self, key: AlertKey, alert: Alert) {
        let latest_message_id = alert.id.clone();
        self.index_by_latest_message_id
            .insert(latest_message_id.clone(), key.clone());
        self.held.insert(
            key,
            HeldAlert {
                latest_message_id,
                alert,
                consecutive_absences: 0,
            },
        );
    }

    fn remove_held(&mut self, key: &str) -> Option<HeldAlert> {
        let held = self.held.remove(key)?;
        self.index_by_latest_message_id
            .remove(&held.latest_message_id);
        Some(held)
    }
}
