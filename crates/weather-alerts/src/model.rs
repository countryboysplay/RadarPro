//! The normalized alert domain model (S06's field list, plus `message_type`
//! and `references`, which lifecycle handling needs -- see the crate root
//! docs).

use radar_types::Timestamp;

/// A single [lon, lat] position, matching the `[lon, lat]` pair convention
/// `radar-web`'s `range_rings_geojson` already uses for GeoJSON-bound
/// coordinates (GeoJSON itself is always `[longitude, latitude]`, the
/// opposite order from the more common "lat, lon" spoken convention).
pub type LonLat = [f64; 2];

/// A single closed linear ring: a `Polygon`'s exterior boundary or one of
/// its holes, per the GeoJSON `Polygon` geometry spec. Not validated here
/// to be closed (first point == last point) or non-self-intersecting --
/// [`crate::parse`] validates the structural minimum (a ring needs at
/// least 4 positions) but does not re-implement a full geometry validator;
/// see that module's docs.
pub type Ring = Vec<LonLat>;

/// A parsed alert's geometry, normalized from GeoJSON's `Polygon` /
/// `MultiPolygon` geometry types. A `null` GeoJSON geometry (common for
/// alerts issued by UGC/zone code rather than a drawn polygon -- confirmed
/// empirically: roughly 3 out of 4 alerts in a real national
/// `/alerts/active` snapshot fetched during development had `null`
/// geometry) is represented as `Alert::geometry` being `None`, not as an
/// [`AlertGeometry`] variant -- there is no "shape" to represent, and this
/// is normal, not an error condition.
#[derive(Debug, Clone, PartialEq)]
pub enum AlertGeometry {
    /// A single polygon: one exterior ring followed by zero or more holes,
    /// exactly as GeoJSON's `Polygon.coordinates` shape carries it.
    Polygon(Vec<Ring>),
    /// Multiple, distinct polygons (each itself exterior-plus-holes),
    /// exactly as GeoJSON's `MultiPolygon.coordinates` shape carries it --
    /// each inner `Vec<Ring>` stays a separate polygon, never flattened
    /// into one ring list.
    MultiPolygon(Vec<Vec<Ring>>),
}

/// CAP `messageType`: what kind of message this is with respect to any
/// alert(s) it [`Alert::references`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    /// A new alert (or, when it carries a non-empty `references` list, a
    /// message that replaces/continues a previously-referenced alert under
    /// a fresh CAP identifier -- see `crate::store`'s "replacement" design
    /// note).
    Alert,
    /// Updated content for a previously-issued, still-active alert
    /// (`references` names the message being updated).
    Update,
    /// Explicit cancellation of a previously-issued alert (`references`
    /// names the message being cancelled). Authoritative: see
    /// `crate::store` for why an explicit `Cancel` is never
    /// second-guessed.
    Cancel,
}

impl MessageType {
    /// The exact wire string this variant round-trips from/to (CAP
    /// `messageType`), used by `crate::json`'s output encoding.
    pub const fn as_str(self) -> &'static str {
        match self {
            MessageType::Alert => "Alert",
            MessageType::Update => "Update",
            MessageType::Cancel => "Cancel",
        }
    }
}

/// CAP `severity`. `Unknown` is a valid, common value (not an error) --
/// confirmed empirically in a real national feed snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Unknown,
    Minor,
    Moderate,
    Severe,
    Extreme,
}

impl Severity {
    /// The exact wire string this variant round-trips from/to (CAP
    /// `severity`), used by `crate::json`'s output encoding.
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Unknown => "Unknown",
            Severity::Minor => "Minor",
            Severity::Moderate => "Moderate",
            Severity::Severe => "Severe",
            Severity::Extreme => "Extreme",
        }
    }
}

/// CAP `certainty`. `Unknown` is a valid, common value (not an error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Certainty {
    Unknown,
    Unlikely,
    Possible,
    Likely,
    Observed,
}

impl Certainty {
    /// The exact wire string this variant round-trips from/to (CAP
    /// `certainty`), used by `crate::json`'s output encoding.
    pub const fn as_str(self) -> &'static str {
        match self {
            Certainty::Unknown => "Unknown",
            Certainty::Unlikely => "Unlikely",
            Certainty::Possible => "Possible",
            Certainty::Likely => "Likely",
            Certainty::Observed => "Observed",
        }
    }
}

/// CAP `urgency`. `Unknown` is a valid, common value (not an error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Unknown,
    Past,
    Future,
    Expected,
    Immediate,
}

impl Urgency {
    /// The exact wire string this variant round-trips from/to (CAP
    /// `urgency`), used by `crate::json`'s output encoding.
    pub const fn as_str(self) -> &'static str {
        match self {
            Urgency::Unknown => "Unknown",
            Urgency::Past => "Past",
            Urgency::Future => "Future",
            Urgency::Expected => "Expected",
            Urgency::Immediate => "Immediate",
        }
    }
}

/// One entry of CAP's `references` array: identifies a previously-issued
/// message this alert updates, replaces, or cancels.
///
/// Only `identifier` (the bare CAP identifier, matching [`Alert::id`]'s
/// format -- confirmed against real data, see [`Alert::id`]'s docs) is
/// used for lifecycle linking in `crate::store`; `sender`/`sent` are kept
/// only as attribution/diagnostic context, matching
/// `DATA_SOURCES.md`'s "preserve ... attribution" instruction.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertReference {
    pub identifier: String,
    pub sender: String,
    pub sent: Timestamp,
}

/// A normalized NWS alert, parsed from one GeoJSON `Feature` of an
/// `/alerts/active` `FeatureCollection` response.
///
/// # Field provenance and normalization decisions
///
/// Every decision below was made against a real `/alerts/active` response
/// fetched during development (both a narrow `?area=OK` query and the full
/// national feed), not guessed from memory -- see this crate's
/// `tests/fixtures/README.md` for the fetch details.
///
/// - **`id`**: CAP's bare `identifier`, taken from GeoJSON `properties.id`
///   (e.g. `"urn:oid:2.49.0.1.840.0.3d35929f...001.1"`) -- **not**
///   `properties["@id"]`/the GeoJSON `Feature.id`, which is the
///   dereferenceable API URL
///   (`"https://api.weather.gov/alerts/urn:oid:..."`) built by prefixing
///   the same bare identifier. `properties.references[].identifier` (see
///   [`AlertReference`]) is confirmed to use this same bare form, which is
///   what makes it usable directly as [`crate::store::AlertStore`]'s
///   lifecycle-linking key without stripping a URL prefix first.
/// - **`issued`**: CAP `sent` (when the message was issued), per this
///   stage's own suggested mapping.
/// - **`expires`**: CAP `expires` -- the mandatory, always-present (in
///   every real `Actual`-status alert observed) technical validity period
///   of *this message*, and the field [`crate::store::AlertStore`] treats
///   as authoritative for time-based expiration (see [`Alert::ends`]'s
///   docs for why `ends` is deliberately *not* used for that).
/// - **`ends`**: CAP `ends` -- optional, and semantically distinct from
///   `expires`: it is the forecaster's estimate of when the *underlying
///   hazard* (not the message's own validity) is expected to end. Kept as
///   its own field for display/attribution, but never substituted for
///   `expires` when deciding whether a held alert has lifecycle-expired.
///   Empirically, of 247 real alerts in a national snapshot that carried a
///   non-null `ends`, 152 had `ends != expires` -- and always with
///   `ends` *after* `expires` (e.g. a Small Craft Advisory with
///   `expires` at T+~9h but `ends` at T+~34h): NWS's own contract for such
///   a long-duration hazard is to *re-issue* an update before `expires`
///   lapses, not to leave the original message's technical validity
///   window covering the full hazard duration. Using `ends` for
///   expiration would therefore keep displaying an alert as active for
///   hours past the point its own issuing message says it is no longer
///   technically valid, silently relying on a value CAP itself does not
///   guarantee is even present.
/// - **`sender`/`sender_id`**: CAP's `senderName` (human-readable
///   attribution, e.g. `"NWS Norman OK"`, suited to on-screen display) and
///   `sender` (a stable identifier, in practice an email address, e.g.
///   `"w-nws.webmaster@noaa.gov"`) are both preserved rather than
///   collapsing to one field -- `DATA_SOURCES.md` asks this stage to
///   preserve attribution, and the two serve different purposes.
/// - **`affected_areas`**: CAP `areaDesc` is a *semicolon*-separated list
///   of human-readable area names, confirmed empirically (e.g.
///   `"Lipscomb, TX; Ochiltree, TX"`) -- **not** comma-separated: a comma
///   is also used *inside* a single area's own name (the
///   `"<county/zone>, <state>"` format, e.g. a single-area alert with
///   `areaDesc == "Canadian, OK"`), so splitting on `,` would incorrectly
///   fragment a single area name. [`Alert::affected_areas`] is therefore
///   `Vec<String>` split on `;` and trimmed, each element kept intact
///   (comma included).
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    /// This message's own bare CAP identifier. See the struct docs for why
    /// this is `properties.id`, not `properties["@id"]`.
    pub id: String,
    pub message_type: MessageType,
    /// Previously-issued message(s) this one updates, replaces, or
    /// cancels. Empty for a genuinely new alert.
    pub references: Vec<AlertReference>,

    pub event: String,
    pub severity: Severity,
    pub certainty: Certainty,
    pub urgency: Urgency,

    /// Human-readable issuing-office attribution (CAP `senderName`).
    pub sender: String,
    /// Stable sender identifier (CAP `sender`; in practice an email
    /// address).
    pub sender_id: String,

    /// CAP `sent`: when this message was issued.
    pub issued: Timestamp,
    /// CAP `effective`: when this message's information becomes effective
    /// (can be later than `issued` for an advance watch/outlook).
    pub effective: Timestamp,
    /// CAP `expires`: this message's own technical validity period. See
    /// the struct docs for why this (not `ends`) is authoritative for
    /// lifecycle expiration.
    pub expires: Timestamp,
    /// CAP `ends`, if present: the forecaster's estimate of when the
    /// underlying hazard itself is expected to end. See the struct docs
    /// for why this is not used for lifecycle expiration.
    pub ends: Option<Timestamp>,

    /// CAP `headline`; `null` in the source feed is common (e.g. system
    /// test/keepalive messages).
    pub headline: Option<String>,
    /// CAP `description`; `null` in the source feed is possible.
    pub description: Option<String>,
    /// CAP `instruction`; `null` in the source feed is common (not every
    /// alert carries protective-action guidance).
    pub instruction: Option<String>,

    /// CAP `areaDesc`, split on `;` and trimmed. See the struct docs for
    /// why this is not split on `,`.
    pub affected_areas: Vec<String>,

    /// This alert's shape, or `None` for a `null` GeoJSON geometry (an
    /// alert issued by zone/UGC code with no drawn polygon -- common, not
    /// an error; see [`AlertGeometry`]'s docs).
    pub geometry: Option<AlertGeometry>,
}
