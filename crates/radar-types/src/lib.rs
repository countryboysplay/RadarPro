//! `radar-types` — canonical shared domain types for RadarPro.
//!
//! This crate holds the polar radar domain model shared by the decoder
//! (`nexrad-level2`) and consumers such as `radar-cli`: `Volume -> Sweep ->
//! Radial -> Moment`, per `ARCHITECTURE.md` and `GLOBAL_CONTRACT.md`.
//!
//! # Design constraints (from `GLOBAL_CONTRACT.md`)
//!
//! - Polar geometry is preserved as long as practical: this crate does
//!   **not** convert azimuth/range/elevation to latitude/longitude. That
//!   projection is a later stage's job (`radar-geo`).
//! - Missing values and range folding are preserved as distinct, explicit
//!   states ([`GateValue::Missing`] / [`GateValue::RangeFolded`]), never
//!   silently converted to a numeric placeholder such as `0.0`.
//! - Internal time is UTC. [`Timestamp`] stores whole milliseconds since
//!   the Unix epoch and is never implicitly interpreted in local time.
//! - Per-moment scale/offset/gate-spacing metadata is retained rather than
//!   discarded after converting to physical units, since it documents how
//!   the physical values were derived from the wire encoding.

use std::fmt;

/// A UTC point in time, stored as whole milliseconds since the Unix epoch
/// (1970-01-01T00:00:00Z).
///
/// This crate intentionally does not depend on a calendar/date-time crate
/// (e.g. `chrono`): NEXRAD Level II only ever gives us "modified Julian
/// date" + "milliseconds past midnight" pairs, which convert directly to
/// an epoch-millisecond count, and the only other operation we need is
/// rendering that count back to a human-readable UTC civil date/time for
/// diagnostics (see [`Timestamp::to_civil_utc`]), which is plain calendar
/// arithmetic rather than anything NEXRAD-specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    millis_since_epoch: i64,
}

impl Timestamp {
    /// Construct a [`Timestamp`] directly from a count of milliseconds
    /// since the Unix epoch (UTC).
    pub const fn from_epoch_millis(millis_since_epoch: i64) -> Self {
        Self { millis_since_epoch }
    }

    /// Construct a [`Timestamp`] from NEXRAD's "modified Julian date" +
    /// "milliseconds past midnight" encoding, as used in both the Archive
    /// II Volume Header Record and every Generic/Message-31 header.
    ///
    /// Per the Archive II/User ICD, the modified Julian date counts days
    /// since 1970-01-01, where 1970-01-01 itself is day **1** (not day 0).
    /// Returns `None` on integer overflow (not reachable for any date
    /// representable within a `u32`, but checked rather than assumed since
    /// both inputs come from untrusted, network-sourced bytes).
    pub fn from_nexrad_date_time(modified_julian_date: u32, millis_of_day: u32) -> Option<Self> {
        let days_since_epoch = i64::from(modified_julian_date).checked_sub(1)?;
        let millis_since_epoch = days_since_epoch
            .checked_mul(MILLIS_PER_DAY)?
            .checked_add(i64::from(millis_of_day))?;
        Some(Self { millis_since_epoch })
    }

    /// Milliseconds since the Unix epoch (UTC).
    pub const fn epoch_millis(&self) -> i64 {
        self.millis_since_epoch
    }

    /// Break this timestamp down into UTC civil date/time components, for
    /// display and diagnostics.
    pub fn to_civil_utc(&self) -> CivilDateTime {
        let millis = self.millis_since_epoch;
        // Split into a day count and a non-negative remainder in
        // [0, MILLIS_PER_DAY), using `div_euclid`/`rem_euclid` so that
        // timestamps before the epoch (negative `millis`) still produce a
        // correctly-rounded-down day count rather than truncating toward
        // zero.
        let days = millis.div_euclid(MILLIS_PER_DAY);
        let millis_of_day = millis.rem_euclid(MILLIS_PER_DAY);

        let (year, month, day) = civil_from_days(days);

        let hour = millis_of_day / 3_600_000;
        let minute = (millis_of_day / 60_000) % 60;
        let second = (millis_of_day / 1_000) % 60;
        let millisecond = millis_of_day % 1_000;

        CivilDateTime {
            year,
            month,
            day,
            hour: hour as u32,
            minute: minute as u32,
            second: second as u32,
            millisecond: millisecond as u32,
        }
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_civil_utc())
    }
}

const MILLIS_PER_DAY: i64 = 86_400_000;

/// A UTC calendar date/time broken into civil components, for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivilDateTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
}

impl fmt::Display for CivilDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} UTC",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millisecond
        )
    }
}

/// Convert a day count since the Unix epoch (1970-01-01 = day 0) into a
/// proleptic-Gregorian (year, month, day) civil date.
///
/// This is Howard Hinnant's `civil_from_days` algorithm
/// (<http://howardhinnant.github.io/date_algorithms.html>), a public-domain,
/// well-tested piece of plain calendar arithmetic (correct for all `i64`
/// day counts representable here) — not NEXRAD-specific behavior.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468; // shift epoch from 1970-01-01 to 0000-03-01
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// A radar site's identity and physical location, as reported by the
/// volume's own metadata (the "VOL" data constant block), not looked up
/// from a hardcoded site database.
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    /// Four-letter NEXRAD/ICAO site identifier, e.g. `"KTLX"`, taken from
    /// the Archive II Volume Header Record.
    pub icao: String,
    /// Latitude in decimal degrees, WGS84, from the volume's "VOL" data
    /// constant block.
    pub latitude_deg: f64,
    /// Longitude in decimal degrees, WGS84, from the volume's "VOL" data
    /// constant block.
    pub longitude_deg: f64,
    /// Height of the radar site above mean sea level, in meters, from the
    /// volume's "VOL" data constant block ("Site Height"). This is not the
    /// tower/antenna height.
    pub height_m: f64,
}

impl Site {
    /// Construct a new [`Site`]. No range validation is performed here;
    /// validating decoded fields is the decoder's responsibility.
    pub fn new(
        icao: impl Into<String>,
        latitude_deg: f64,
        longitude_deg: f64,
        height_m: f64,
    ) -> Self {
        Self {
            icao: icao.into(),
            latitude_deg,
            longitude_deg,
            height_m,
        }
    }
}

/// A fully decoded NEXRAD Level II volume scan: one radar site sweeping
/// through a Volume Coverage Pattern (VCP), preserved in native polar
/// geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    /// The radar site that produced this volume.
    pub site: Site,
    /// Volume start time (UTC), from the Archive II Volume Header Record.
    pub start_time: Timestamp,
    /// Volume Coverage Pattern number in effect for this volume, from the
    /// "VOL" data constant block.
    pub volume_coverage_pattern: u16,
    /// Sweeps (elevation cuts) in the order they were scanned.
    pub sweeps: Vec<Sweep>,
}

/// One elevation cut (sweep) of a volume scan: a full rotation at a
/// (nominally) constant elevation angle.
#[derive(Debug, Clone, PartialEq)]
pub struct Sweep {
    /// Elevation number within the volume scan (1-based).
    pub elevation_number: u8,
    /// Nominal elevation angle in degrees for this sweep, taken from the
    /// first radial in the sweep. Individual radials can vary slightly
    /// from this nominal angle; each [`Radial`] preserves its own exact
    /// elevation angle.
    pub elevation_angle_deg: f32,
    /// Radials that make up this sweep, in scan order.
    pub radials: Vec<Radial>,
}

/// Azimuth resolution (spacing between adjacent radials), from Message 31's
/// "Azimuth Resolution Spacing" field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzimuthResolution {
    /// 0.5 degrees between radials ("super resolution").
    Half,
    /// 1.0 degree between radials.
    One,
}

impl AzimuthResolution {
    /// The nominal spacing between radials, in degrees.
    pub const fn degrees(self) -> f32 {
        match self {
            AzimuthResolution::Half => 0.5,
            AzimuthResolution::One => 1.0,
        }
    }
}

/// The meaning of a radial's position within the elevation/volume scan,
/// from Message 31's "Radial Status" field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadialStatusKind {
    StartOfElevation,
    Intermediate,
    EndOfElevation,
    StartOfVolume,
    EndOfVolume,
    /// Start of elevation, and this elevation is also the last one in the
    /// VCP.
    StartOfElevationLastInVcp,
}

/// A radial's status, decoded from Message 31's "Radial Status" byte: the
/// low bits give the [`RadialStatusKind`] and the high bit (0x80) flags the
/// "bad data" variant of that same meaning, per the Archive II/User ICD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RadialStatus {
    pub kind: RadialStatusKind,
    /// Set when the high bit (0x80) of the wire value was set, indicating
    /// the RDA flagged this radial's status as a "bad data" variant.
    pub bad_data: bool,
}

/// One radial (a single azimuth sweep at one elevation angle): the
/// smallest unit of scan geometry, carrying zero or more decoded moments.
#[derive(Debug, Clone, PartialEq)]
pub struct Radial {
    /// Azimuth number within the elevation scan (1-720).
    pub azimuth_number: u16,
    /// Azimuth angle in degrees (0.0-359.956055), clockwise from true
    /// north.
    pub azimuth_angle_deg: f32,
    /// Azimuth resolution (spacing between radials) in effect for this
    /// radial.
    pub azimuth_resolution: AzimuthResolution,
    /// Elevation angle in degrees for this specific radial (-7.0 to 70.0).
    pub elevation_angle_deg: f32,
    /// This radial's position within the elevation/volume scan.
    pub radial_status: RadialStatus,
    /// Collection time (UTC) for this radial.
    pub collection_time: Timestamp,
    /// Moments present on this radial, keyed by moment kind. A moment
    /// absent from this map was not present on the wire for this radial
    /// (the Data Block Pointer for it was zero).
    pub moments: MomentMap,
}

/// The digital weather moments this stage decodes from Message 31's Data
/// Moment blocks.
///
/// Deliberately excludes `"CFP"` (clutter filter power removed) and KDP
/// (a derived product, not a wire moment at all) — both out of scope for
/// this stage per `GLOBAL_CONTRACT.md`/S01.
///
/// Declaration order matches the S01 scope order ("REF first, then VEL,
/// SW, ZDR, CC, PHI"), which also determines the derived [`Ord`] used to
/// keep [`MomentMap`] iteration in that same stable order.
/// [`MomentKind::StormRelativeVelocity`] was added later (S11 Phase 2b) at
/// the **end** of the enum specifically so it does not renumber/reorder any
/// of the original six variants relative to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MomentKind {
    /// Reflectivity ("REF" on the wire).
    Reflectivity,
    /// Radial velocity ("VEL" on the wire).
    Velocity,
    /// Spectrum width ("SW " on the wire).
    SpectrumWidth,
    /// Differential reflectivity ("ZDR" on the wire).
    DifferentialReflectivity,
    /// Correlation coefficient ("RHO" on the wire; named `CC`/
    /// `CorrelationCoefficient` here to match `GLOBAL_CONTRACT.md`'s own
    /// naming for this moment).
    CorrelationCoefficient,
    /// Differential phase ("PHI" on the wire).
    DifferentialPhase,
    /// Storm-Relative Velocity: a *derived* product (base radial velocity
    /// with a uniform storm-motion vector's radial component subtracted,
    /// see the `radar-geo` crate's `storm_relative_velocity` module), never
    /// present in real NEXRAD Archive II wire data -- computed at render
    /// time from an
    /// already-decoded [`MomentKind::Velocity`] moment. Kept as its own
    /// first-class, independently-selectable `MomentKind` (reusing VEL's
    /// units/physical range via a copy of its color table under a
    /// distinct id) rather than a display option layered on
    /// [`MomentKind::Velocity`], so a rendered/selected SRV product is
    /// never conflated with the real decoded VEL moment it was built
    /// from. Added at the end of this enum -- see the enum's own doc
    /// comment.
    StormRelativeVelocity,
}

impl MomentKind {
    /// This moment's canonical short wire-style name, matching the Archive
    /// II Data Moment identifiers `RADAR_TECHNICAL.md` documents (`REF`,
    /// `VEL`, `SW`, `ZDR`, `CC`, `PHI`) -- `CC` rather than the wire's own
    /// `"RHO"` spelling, matching this enum's own
    /// [`MomentKind::CorrelationCoefficient`] naming (see that variant's
    /// doc comment).
    ///
    /// Added narrowly for S05: `radar-render`'s original color-table
    /// format and `radar-web`'s wasm API both need a stable, short,
    /// human-readable way to name a moment kind in JSON/JS, and that
    /// mapping belongs once on `MomentKind` itself rather than duplicated
    /// ad hoc in each of those crates.
    ///
    /// `"SRV"` ([`MomentKind::StormRelativeVelocity`]) is a **synthesized-
    /// only** code: no real NEXRAD Archive II volume ever contains an
    /// "SRV" moment on the wire (SRV is derived at render time from an
    /// already-decoded VEL moment, never decoded itself -- see that
    /// variant's doc comment). This code exists solely as this app's own
    /// internal selection/UI identifier, reusing the same
    /// `wire_code`/`from_wire_code` round-trip every other moment already
    /// uses end-to-end for selection plumbing (color-table lookup, the
    /// wasm API's moment parameters, the UI's moment picker) -- it is not
    /// a claim that "SRV" appears in any ICD/Archive II data structure.
    pub const fn wire_code(self) -> &'static str {
        match self {
            MomentKind::Reflectivity => "REF",
            MomentKind::Velocity => "VEL",
            MomentKind::SpectrumWidth => "SW",
            MomentKind::DifferentialReflectivity => "ZDR",
            MomentKind::CorrelationCoefficient => "CC",
            MomentKind::DifferentialPhase => "PHI",
            MomentKind::StormRelativeVelocity => "SRV",
        }
    }

    /// Parse [`MomentKind::wire_code`]'s output back into a [`MomentKind`],
    /// or `None` for any other string. Case-sensitive (wire codes are
    /// always upper-case) -- a caller accepting less-trusted input (e.g. a
    /// user-edited color-table file, or a moment code from JS) must treat
    /// `None` as a validation error, never guess via case-folding.
    pub fn from_wire_code(code: &str) -> Option<Self> {
        match code {
            "REF" => Some(MomentKind::Reflectivity),
            "VEL" => Some(MomentKind::Velocity),
            "SW" => Some(MomentKind::SpectrumWidth),
            "ZDR" => Some(MomentKind::DifferentialReflectivity),
            "CC" => Some(MomentKind::CorrelationCoefficient),
            "PHI" => Some(MomentKind::DifferentialPhase),
            "SRV" => Some(MomentKind::StormRelativeVelocity),
            _ => None,
        }
    }
}

impl fmt::Display for MomentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.wire_code())
    }
}

/// A single gate's decoded value for a moment, preserving "missing" and
/// "range-folded" as distinct states rather than collapsing them into a
/// numeric placeholder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateValue {
    /// Wire value 0: below signal threshold / no data.
    Missing,
    /// Wire value 1: range-folded (ambiguous range).
    RangeFolded,
    /// Wire value >= 2, converted to physical units via this moment's
    /// scale/offset.
    Value(f32),
}

/// One moment's decoded data for a single radial, retaining the wire
/// metadata (gate spacing, scale/offset) that the physical values were
/// derived from.
#[derive(Debug, Clone, PartialEq)]
pub struct Moment {
    /// Range from the radar to the first gate, in kilometers.
    pub first_gate_range_km: f32,
    /// Distance between successive gate centers, in kilometers.
    pub gate_spacing_km: f32,
    /// Scale used to convert raw wire values (>= 2) to physical units:
    /// `physical = (raw - offset) / scale`.
    pub scale: f32,
    /// Offset used to convert raw wire values (>= 2) to physical units:
    /// `physical = (raw - offset) / scale`.
    pub offset: f32,
    /// Decoded gate values, in range order (index 0 = first gate).
    pub gates: Vec<GateValue>,
}

/// Moments present on a [`Radial`], keyed by [`MomentKind`] and kept in a
/// stable (REF, VEL, SW, ZDR, CC, PHI) iteration order.
pub type MomentMap = std::collections::BTreeMap<MomentKind, Moment>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_new_stores_fields_unchanged() {
        let site = Site::new("KTLX", 35.3333, -97.2778, 370.0);

        assert_eq!(site.icao, "KTLX");
        assert_eq!(site.latitude_deg, 35.3333);
        assert_eq!(site.longitude_deg, -97.2778);
        assert_eq!(site.height_m, 370.0);
    }

    #[test]
    fn timestamp_from_epoch_millis_round_trips() {
        let ts = Timestamp::from_epoch_millis(1_717_200_000_000);
        assert_eq!(ts.epoch_millis(), 1_717_200_000_000);
    }

    #[test]
    fn timestamp_epoch_is_1970_01_01() {
        let ts = Timestamp::from_epoch_millis(0);
        let civil = ts.to_civil_utc();
        assert_eq!(civil.year, 1970);
        assert_eq!(civil.month, 1);
        assert_eq!(civil.day, 1);
        assert_eq!(civil.hour, 0);
        assert_eq!(civil.minute, 0);
        assert_eq!(civil.second, 0);
    }

    /// Ground truth from `KTLX20240601_000353_V06`'s Volume Header Record:
    /// modified Julian date 19876, milliseconds-of-day 233941, which is
    /// documented (and independently verified via calendar arithmetic) to
    /// be 2024-06-01 00:03:53.941 UTC.
    #[test]
    fn timestamp_from_nexrad_date_time_matches_ktlx_fixture_ground_truth() {
        let ts = Timestamp::from_nexrad_date_time(19876, 233_941).expect("no overflow");
        let civil = ts.to_civil_utc();

        assert_eq!(civil.year, 2024);
        assert_eq!(civil.month, 6);
        assert_eq!(civil.day, 1);
        assert_eq!(civil.hour, 0);
        assert_eq!(civil.minute, 3);
        assert_eq!(civil.second, 53);
        assert_eq!(civil.millisecond, 941);
    }

    #[test]
    fn timestamp_from_nexrad_date_time_epoch_day_one_is_1970_01_01() {
        // Per the ICD, modified Julian date 1 = 1970-01-01 (day 1, not day
        // 0), with zero milliseconds past midnight.
        let ts = Timestamp::from_nexrad_date_time(1, 0).expect("no overflow");
        let civil = ts.to_civil_utc();

        assert_eq!(civil.year, 1970);
        assert_eq!(civil.month, 1);
        assert_eq!(civil.day, 1);
    }

    #[test]
    fn timestamp_from_nexrad_date_time_never_panics_on_extreme_input() {
        // `u32::MAX` days/millis is nonsensical input, but the widest
        // values these wire fields can ever carry still fit in an `i64`
        // millisecond count without overflow; the checked arithmetic in
        // `from_nexrad_date_time` exists as defense-in-depth against
        // future-proofing (e.g. a wider input type), not because this
        // exact overflow is reachable today. What matters is that
        // untrusted, attacker-controlled input never panics.
        let result = Timestamp::from_nexrad_date_time(u32::MAX, u32::MAX);
        assert!(result.is_some());
        let _ = result.unwrap().to_civil_utc();
    }

    #[test]
    fn azimuth_resolution_degrees() {
        assert_eq!(AzimuthResolution::Half.degrees(), 0.5);
        assert_eq!(AzimuthResolution::One.degrees(), 1.0);
    }

    #[test]
    fn moment_kind_ordering_matches_s01_scope_order() {
        let mut kinds = vec![
            MomentKind::DifferentialPhase,
            MomentKind::Reflectivity,
            MomentKind::CorrelationCoefficient,
            MomentKind::Velocity,
            MomentKind::DifferentialReflectivity,
            MomentKind::SpectrumWidth,
        ];
        kinds.sort();

        assert_eq!(
            kinds,
            vec![
                MomentKind::Reflectivity,
                MomentKind::Velocity,
                MomentKind::SpectrumWidth,
                MomentKind::DifferentialReflectivity,
                MomentKind::CorrelationCoefficient,
                MomentKind::DifferentialPhase,
            ]
        );
    }

    #[test]
    fn gate_value_variants_are_distinct() {
        assert_ne!(GateValue::Missing, GateValue::RangeFolded);
        match GateValue::Value(12.5) {
            GateValue::Value(v) => assert_eq!(v, 12.5),
            _ => panic!("expected Value"),
        }
    }

    #[test]
    fn moment_kind_wire_code_round_trips_for_every_variant() {
        let all = [
            MomentKind::Reflectivity,
            MomentKind::Velocity,
            MomentKind::SpectrumWidth,
            MomentKind::DifferentialReflectivity,
            MomentKind::CorrelationCoefficient,
            MomentKind::DifferentialPhase,
            MomentKind::StormRelativeVelocity,
        ];
        for kind in all {
            let code = kind.wire_code();
            assert_eq!(MomentKind::from_wire_code(code), Some(kind));
            assert_eq!(kind.to_string(), code);
        }
    }

    #[test]
    fn moment_kind_from_wire_code_rejects_unknown_and_wrong_case() {
        assert_eq!(MomentKind::from_wire_code("RHO"), None);
        assert_eq!(MomentKind::from_wire_code("ref"), None);
        assert_eq!(MomentKind::from_wire_code(""), None);
    }
}
