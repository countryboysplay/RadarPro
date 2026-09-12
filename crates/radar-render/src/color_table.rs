//! RadarPro's original, documented, user-editable color-table format:
//! parsing, validation, and building the existing RGBA8 LUT format
//! [`gpu::upload_palette`]/[`gpu::update_palette`] already consume, for any
//! [`MomentKind`] rather than the single hardcoded REF ramp
//! [`crate::palette`] shipped as an S03 placeholder.
//!
//! See `COLOR_TABLE_FORMAT.md` (crate root) for the full field-by-field
//! spec and a worked example, and `docs/adr/0009-original-color-table-format.md`
//! for the design rationale. This module's job is the Rust side of that
//! spec: [`ColorTable::from_json`] parses and validates untrusted,
//! possibly-hand-edited JSON (never panics -- returns [`ColorTableError`]),
//! and [`build_lut_from_table`] turns a validated table into the same
//! `Vec<[u8; 4]>` LUT shape [`crate::palette::build_palette_lut`] already
//! produces for the placeholder REF ramp, generalizing (not duplicating)
//! that module's interpolation primitives -- see [`crate::palette::sample_stops`]
//! and [`crate::palette::sample_stops_stepped`], both reused here.
//!
//! [`gpu::upload_palette`]: crate::gpu::upload_palette
//! [`gpu::update_palette`]: crate::gpu::update_palette
//! [`MomentKind`]: radar_types::MomentKind
//!
//! # Structural missing/range-folded safety
//!
//! `missing_color` and `range_folded_color` are dedicated top-level fields,
//! never entries in [`ColorTable::stops`]. This is a type-level guarantee,
//! not merely a convention: [`build_lut_from_table`] (and every sampling
//! function it calls) only ever reads `stops`/`domain`/`mode`/`cyclic` --
//! there is no code path by which a *physical value* lookup could produce
//! `missing_color`/`range_folded_color`, because those two fields are
//! never passed into the interpolation/stepping functions at all. This
//! mirrors (and is the format-level analogue of) how
//! `sweep_buffers::GpuGateSample::flag` and `shaders/radar_sweep.wgsl`
//! already branch on missing/range-folded *before* ever sampling the
//! palette texture -- the active renderer does not even reach this
//! module's LUT for those gates.
//!
//! One consequence worth documenting explicitly: this structural
//! separation does *not* forbid a table from choosing the *same visual
//! appearance* for `missing_color` and a low-end value stop -- e.g. every
//! built-in default in `color_tables/` uses fully-transparent
//! `[0, 0, 0, 0]` for `missing_color`, and REF/SW's own ramps also use
//! `[0, 0, 0, 0]` for their sub-threshold "floor" stops. That is an
//! intentional, sensible design choice (a below-threshold real reading and
//! "no data" both render as invisible), not an ambiguity: the underlying
//! `GateValue`/`GpuGateSample::flag` distinction survives perfectly
//! through the separate flag channel regardless of what color anything
//! maps to. What *is* validated is that `missing_color` and
//! `range_folded_color` differ **from each other** (see
//! [`ColorTableError::MissingColorEqualsRangeFoldedColor`]), so a legend
//! can always show them as two distinct states.

use crate::palette::{sample_stops, sample_stops_stepped, PaletteStop};
use radar_types::MomentKind;
use std::fmt;

/// The format version this module reads/writes. Bumped only if the JSON
/// shape changes incompatibly; [`ColorTable::validate`] rejects any other
/// value rather than guessing at forward/backward compatibility.
pub const CURRENT_FORMAT_VERSION: u32 = 1;

/// One control point in a color table: reuses [`crate::palette::PaletteStop`]
/// directly (same `{value, color}` shape, already `Serialize`/
/// `Deserialize`) rather than defining a second, duplicate stop type.
pub type ColorStop = PaletteStop;

/// How [`ColorTable::stops`] maps a physical value to a color between
/// control points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorTableMode {
    /// Discrete color per value bucket, held unchanged until the next
    /// stop's threshold -- the traditional NWS-style reflectivity look.
    /// See [`crate::palette::sample_stops_stepped`].
    Stepped,
    /// Continuous linear interpolation between consecutive stops. See
    /// [`crate::palette::sample_stops`].
    Gradient,
}

/// The physical-value range this table's LUT covers: texel 0 of the built
/// LUT corresponds to `min`, and the last texel to `max` -- matching
/// [`crate::palette::build_palette_lut`]'s own `min_value`/`max_value`
/// convention.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorTableDomain {
    pub min: f32,
    pub max: f32,
}

/// A parsed, validated color table: see `COLOR_TABLE_FORMAT.md` for the
/// full field-by-field spec and a worked example.
///
/// Construct via [`ColorTable::from_json`] (the only way to get one from
/// untrusted/user-edited input -- it always validates) or
/// [`default_color_table`] (this crate's own built-in defaults, one per
/// [`MomentKind`], also validated -- see that function's docs for why an
/// `expect` there is not the "never panic on untrusted input" rule being
/// violated).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorTable {
    /// Must equal [`CURRENT_FORMAT_VERSION`].
    pub format_version: u32,
    /// Human-readable display name, e.g. `"RadarPro Default Reflectivity"`.
    pub name: String,
    /// Which moment(s) this table is compatible with, as
    /// [`MomentKind::wire_code`] strings (e.g. `["REF"]`). A table may
    /// name more than one moment when the same ramp is meaningfully shared
    /// (e.g. a generic diverging ramp usable for more than one signed
    /// quantity); every built-in default in `color_tables/` names exactly
    /// one.
    pub moments: Vec<String>,
    /// The physical unit `stops`' values are expressed in (e.g. `"dBZ"`,
    /// `"m/s"`, `"dB"`, `"deg"`, `"dimensionless"`) -- documentation only
    /// (this module does not convert units), but load-bearing
    /// documentation: `GLOBAL_CONTRACT.md`'s "display-unit changes never
    /// mutate source data" means a UI must always know what unit a table's
    /// numbers are already in.
    pub units: String,
    pub mode: ColorTableMode,
    pub domain: ColorTableDomain,
    /// Whether `domain`/`stops` wrap around (e.g. PHI's 0-360 degree
    /// differential phase, where 360 and 0 are the same physical angle).
    /// When `true`, a value outside `[domain.min, domain.max)` is wrapped
    /// into that range with `rem_euclid` before sampling, and (in
    /// [`ColorTableMode::Gradient`] mode) the ramp interpolates smoothly
    /// across the `domain.max`/`domain.min` seam instead of clamping to
    /// the nearest endpoint stop. Defaults to `false` when absent from the
    /// JSON (every non-cyclic quantity: dBZ, m/s, dB, dimensionless).
    #[serde(default)]
    pub cyclic: bool,
    /// Control points, which [`ColorTable::validate`] requires to be
    /// non-empty, each with a finite `value` within `[domain.min,
    /// domain.max]`, sorted strictly ascending by `value`.
    pub stops: Vec<ColorStop>,
    /// Color for a gate flagged [`radar_types::GateValue::Missing`] (below
    /// signal threshold / no data). Never produced by sampling `stops` --
    /// see the module docs' "Structural missing/range-folded safety".
    pub missing_color: [u8; 4],
    /// Color for a gate flagged [`radar_types::GateValue::RangeFolded`]
    /// (ambiguous range). Never produced by sampling `stops`, and
    /// validated to differ from `missing_color` -- see the module docs.
    pub range_folded_color: [u8; 4],
}

impl ColorTable {
    /// Parse and validate a color table from a JSON string. This is the
    /// one entry point untrusted input (a user-pasted/uploaded file) must
    /// go through: malformed JSON, an unsupported format version, or any
    /// [`ColorTable::validate`] failure returns a structured
    /// [`ColorTableError`] -- this function never panics on malformed
    /// input, per `GLOBAL_CONTRACT.md`'s "no uncontrolled panics on
    /// malformed input" (written about remote radar data, but the same
    /// discipline applies to a local user-edited file: it is still
    /// untrusted input).
    pub fn from_json(json: &str) -> Result<Self, ColorTableError> {
        let table: ColorTable = serde_json::from_str(json)?;
        table.validate()?;
        Ok(table)
    }

    /// Serialize back to the same JSON shape [`ColorTable::from_json`]
    /// reads -- used by `radar-web`'s wasm API to hand an active table
    /// back to a browser UI for display/editing.
    pub fn to_json_pretty(&self) -> Result<String, ColorTableError> {
        serde_json::to_string_pretty(self).map_err(ColorTableError::Json)
    }

    /// This table's `moments` strings resolved to real [`MomentKind`]
    /// values. Only meaningful after [`ColorTable::validate`] has
    /// succeeded (via [`ColorTable::from_json`]/[`default_color_table`]) --
    /// an unresolvable entry is silently skipped here rather than erroring,
    /// since `validate` is the single place that rejects an unknown moment
    /// string; this is a convenience for already-validated tables.
    pub fn resolved_moments(&self) -> Vec<MomentKind> {
        self.moments
            .iter()
            .filter_map(|code| MomentKind::from_wire_code(code))
            .collect()
    }

    /// Run every structural/content check this format requires. Called
    /// automatically by [`ColorTable::from_json`]; exposed separately so a
    /// caller that built a [`ColorTable`] programmatically (rather than
    /// from JSON) can still validate it before use.
    pub fn validate(&self) -> Result<(), ColorTableError> {
        if self.format_version != CURRENT_FORMAT_VERSION {
            return Err(ColorTableError::UnsupportedFormatVersion(
                self.format_version,
            ));
        }
        if self.name.trim().is_empty() {
            return Err(ColorTableError::EmptyName);
        }
        if self.moments.is_empty() {
            return Err(ColorTableError::EmptyMoments);
        }
        for code in &self.moments {
            if MomentKind::from_wire_code(code).is_none() {
                return Err(ColorTableError::UnknownMomentKind(code.clone()));
            }
        }
        if self.units.trim().is_empty() {
            return Err(ColorTableError::EmptyUnits);
        }
        if !(self.domain.min.is_finite()
            && self.domain.max.is_finite()
            && self.domain.min < self.domain.max)
        {
            return Err(ColorTableError::InvalidDomain {
                min: self.domain.min,
                max: self.domain.max,
            });
        }
        if self.stops.is_empty() {
            return Err(ColorTableError::EmptyStops);
        }
        let mut previous_value: Option<f32> = None;
        for (index, stop) in self.stops.iter().enumerate() {
            if !stop.value.is_finite() {
                return Err(ColorTableError::NonFiniteStopValue {
                    index,
                    value: stop.value,
                });
            }
            if let Some(previous) = previous_value {
                if stop.value <= previous {
                    return Err(ColorTableError::StopsNotStrictlyAscending { index });
                }
            }
            previous_value = Some(stop.value);
            if stop.value < self.domain.min || stop.value > self.domain.max {
                return Err(ColorTableError::StopValueOutOfDomain {
                    index,
                    value: stop.value,
                });
            }
        }
        if self.missing_color == self.range_folded_color {
            return Err(ColorTableError::MissingColorEqualsRangeFoldedColor);
        }
        Ok(())
    }
}

/// Every way [`ColorTable::from_json`]/[`ColorTable::validate`] can reject
/// a table, plus a human-readable [`fmt::Display`] message for each --
/// never a panic, per this module's "untrusted input" discipline.
#[derive(Debug)]
pub enum ColorTableError {
    /// The input was not valid JSON, or did not match this format's shape
    /// (wrong field type, missing required field, or an unknown field
    /// rejected by `#[serde(deny_unknown_fields)]` -- deliberately strict,
    /// so a typo in a hand-edited file is a load error, not a silently
    /// ignored field).
    Json(serde_json::Error),
    /// `format_version` was present and well-typed but not
    /// [`CURRENT_FORMAT_VERSION`].
    UnsupportedFormatVersion(u32),
    /// `name` was empty (or all whitespace).
    EmptyName,
    /// `moments` was an empty array.
    EmptyMoments,
    /// A `moments` entry did not match any [`MomentKind::wire_code`].
    UnknownMomentKind(String),
    /// `units` was empty (or all whitespace).
    EmptyUnits,
    /// `domain.min`/`domain.max` were not both finite with `min < max`.
    InvalidDomain { min: f32, max: f32 },
    /// `stops` was an empty array.
    EmptyStops,
    /// A stop's `value` was `NaN` or infinite.
    NonFiniteStopValue { index: usize, value: f32 },
    /// `stops` was not sorted strictly ascending by `value` (a duplicate
    /// value counts as a violation too -- every stop must have a distinct
    /// value).
    StopsNotStrictlyAscending { index: usize },
    /// A stop's `value` fell outside `[domain.min, domain.max]`.
    StopValueOutOfDomain { index: usize, value: f32 },
    /// `missing_color` and `range_folded_color` were identical -- a legend
    /// could never show them as two distinct states.
    MissingColorEqualsRangeFoldedColor,
}

impl fmt::Display for ColorTableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorTableError::Json(e) => write!(f, "invalid color-table JSON: {e}"),
            ColorTableError::UnsupportedFormatVersion(v) => write!(
                f,
                "unsupported color-table format_version {v} (expected {CURRENT_FORMAT_VERSION})"
            ),
            ColorTableError::EmptyName => write!(f, "color-table \"name\" must not be empty"),
            ColorTableError::EmptyMoments => {
                write!(f, "color-table \"moments\" must name at least one moment")
            }
            ColorTableError::UnknownMomentKind(code) => {
                write!(f, "unknown moment code {code:?} in \"moments\"")
            }
            ColorTableError::EmptyUnits => write!(f, "color-table \"units\" must not be empty"),
            ColorTableError::InvalidDomain { min, max } => write!(
                f,
                "invalid \"domain\": min ({min}) must be finite and less than max ({max})"
            ),
            ColorTableError::EmptyStops => {
                write!(f, "color-table \"stops\" must contain at least one stop")
            }
            ColorTableError::NonFiniteStopValue { index, value } => {
                write!(f, "stops[{index}].value ({value}) must be a finite number")
            }
            ColorTableError::StopsNotStrictlyAscending { index } => write!(
                f,
                "stops[{index}].value must be strictly greater than the previous stop's value"
            ),
            ColorTableError::StopValueOutOfDomain { index, value } => {
                write!(f, "stops[{index}].value ({value}) falls outside \"domain\"")
            }
            ColorTableError::MissingColorEqualsRangeFoldedColor => write!(
                f,
                "\"missing_color\" and \"range_folded_color\" must differ from each other"
            ),
        }
    }
}

impl std::error::Error for ColorTableError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ColorTableError::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for ColorTableError {
    fn from(e: serde_json::Error) -> Self {
        ColorTableError::Json(e)
    }
}

/// This crate's own built-in default table for `kind`, embedded at compile
/// time from `color_tables/*.json` (the same files documented and shipped
/// as this format's worked examples -- there is exactly one copy of each
/// default, not a Rust literal duplicating a JSON example elsewhere).
///
/// The `expect` here is not a "trust untrusted input" violation: these six
/// files are developer-authored, compiled into the binary, and covered by
/// this module's own tests (`default_color_table_is_valid_for_every_moment_kind`)
/// -- a parse/validation failure here is a build-time programmer error to
/// be caught immediately, not a possible runtime outcome of loading a
/// user-supplied file (that path is [`ColorTable::from_json`], which
/// always returns a `Result`).
pub fn default_color_table(kind: MomentKind) -> ColorTable {
    let json: &str = match kind {
        MomentKind::Reflectivity => include_str!("../color_tables/reflectivity.json"),
        MomentKind::Velocity => include_str!("../color_tables/velocity.json"),
        MomentKind::SpectrumWidth => include_str!("../color_tables/spectrum_width.json"),
        MomentKind::DifferentialReflectivity => {
            include_str!("../color_tables/differential_reflectivity.json")
        }
        MomentKind::CorrelationCoefficient => {
            include_str!("../color_tables/correlation_coefficient.json")
        }
        MomentKind::DifferentialPhase => include_str!("../color_tables/differential_phase.json"),
    };
    ColorTable::from_json(json).expect(
        "built-in default color table must be valid JSON satisfying this crate's own format",
    )
}

/// Normalize `value` into `[min, max)` by wrapping (`rem_euclid`), for
/// [`ColorTable::cyclic`] sampling. Returns `value` unchanged if `max <=
/// min` (degenerate domain; [`ColorTable::validate`] already rejects this,
/// so real callers never hit that branch).
fn wrap_into_domain(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if span > 0.0 {
        min + (value - min).rem_euclid(span)
    } else {
        value
    }
}

/// [`ColorTableMode::Gradient`] sampling for a [`ColorTable::cyclic`]
/// table: wraps `value` into `[min, max)`, then interpolates against
/// `stops` *extended* with one virtual copy of the last stop shifted
/// `span` below the first stop's value, and one virtual copy of the first
/// stop shifted `span` above the last stop's value. That extension is what
/// makes the ramp interpolate smoothly across the `max`/`min` seam
/// (e.g. PHI's 350 degrees blending toward 360 == 0 degrees) instead of
/// clamping flatly to whichever endpoint stop is nearest, which is what
/// plain [`sample_stops`] would otherwise do at the domain edges.
///
/// Reuses [`sample_stops`] itself for the actual interpolation -- this
/// function's only new logic is building the two virtual boundary stops.
fn sample_cyclic_gradient(stops: &[ColorStop], value: f32, min: f32, max: f32) -> [u8; 4] {
    let span = max - min;
    let span_is_positive = span.is_finite() && span > 0.0;
    if stops.len() < 2 || !span_is_positive {
        return sample_stops(stops, value);
    }
    let wrapped = wrap_into_domain(value, min, max);
    let first = stops[0];
    let last = stops[stops.len() - 1];
    let mut extended = Vec::with_capacity(stops.len() + 2);
    extended.push(ColorStop::new(first.value - span, last.color));
    extended.extend_from_slice(stops);
    extended.push(ColorStop::new(last.value + span, first.color));
    sample_stops(&extended, wrapped)
}

/// [`ColorTableMode::Stepped`] sampling for a [`ColorTable::cyclic`]
/// table: [`sample_stops_stepped`] already "holds" the last stop's color
/// all the way to `domain.max`, so the only cyclic-specific behavior
/// needed is wrapping an out-of-domain input value back into range before
/// stepping (e.g. 370 degrees behaves as 10 degrees, not as "clamp to the
/// stop nearest 360").
fn sample_cyclic_stepped(stops: &[ColorStop], value: f32, min: f32, max: f32) -> [u8; 4] {
    sample_stops_stepped(stops, wrap_into_domain(value, min, max))
}

/// Map one physical `value` through `table` to an RGBA8 color, dispatching
/// on [`ColorTable::mode`] and [`ColorTable::cyclic`]. Never consulted for
/// a missing/range-folded gate -- see the module docs.
fn sample_value(table: &ColorTable, value: f32) -> [u8; 4] {
    let ColorTableDomain { min, max } = table.domain;
    match (table.mode, table.cyclic) {
        (ColorTableMode::Gradient, false) => sample_stops(&table.stops, value),
        (ColorTableMode::Gradient, true) => sample_cyclic_gradient(&table.stops, value, min, max),
        (ColorTableMode::Stepped, false) => sample_stops_stepped(&table.stops, value),
        (ColorTableMode::Stepped, true) => sample_cyclic_stepped(&table.stops, value, min, max),
    }
}

/// Build a `texel_count`-entry RGBA8 LUT from `table`, in exactly the
/// shape [`crate::gpu::upload_palette`]/[`crate::gpu::update_palette`]
/// already expect (the same `Vec<[u8; 4]>` [`crate::palette::build_palette_lut`]
/// produces) -- a generalization of that function to any
/// [`ColorTable`] (any moment, either [`ColorTableMode`], optionally
/// [`ColorTable::cyclic`]) instead of one hardcoded REF gradient.
///
/// Texel `i` represents `u = i / (texel_count - 1)`, mapped to a physical
/// value `table.domain.min + u * (domain.max - domain.min)` -- identical
/// texel-to-value convention to [`crate::palette::build_palette_lut`], so
/// the shader's existing `(value - palette_min) / (palette_max -
/// palette_min)` sampling math needs no change to consume a table-built
/// LUT instead of the placeholder ramp.
///
/// Returns an all-transparent-black LUT if `table.stops` is empty (never
/// true for a [`ColorTable::validate`]-passed table) or `texel_count` is
/// `0`, mirroring [`crate::palette::build_palette_lut`]'s own degenerate-
/// input behavior.
pub fn build_lut_from_table(table: &ColorTable, texel_count: usize) -> Vec<[u8; 4]> {
    if table.stops.is_empty() || texel_count == 0 {
        return vec![[0, 0, 0, 0]; texel_count];
    }
    let ColorTableDomain { min, max } = table.domain;
    let span = max - min;
    (0..texel_count)
        .map(|texel| {
            let u = if texel_count == 1 {
                0.0
            } else {
                texel as f32 / (texel_count - 1) as f32
            };
            let value = if span.is_finite() && span != 0.0 {
                min + u * span
            } else {
                min
            };
            sample_value(table, value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::lerp_color;

    const ALL_MOMENT_KINDS: [MomentKind; 6] = [
        MomentKind::Reflectivity,
        MomentKind::Velocity,
        MomentKind::SpectrumWidth,
        MomentKind::DifferentialReflectivity,
        MomentKind::CorrelationCoefficient,
        MomentKind::DifferentialPhase,
    ];

    // --- built-in defaults: valid for every moment kind ------------------

    #[test]
    fn default_color_table_is_valid_for_every_moment_kind() {
        for kind in ALL_MOMENT_KINDS {
            let table = default_color_table(kind);
            assert!(table.validate().is_ok());
            assert_eq!(table.resolved_moments(), vec![kind]);
            assert!(!table.stops.is_empty());
        }
    }

    #[test]
    fn default_reflectivity_table_matches_the_original_placeholder_ramp() {
        // The REF default must be a faithful port of the S03 placeholder
        // ramp (`palette::default_ref_palette_stops`), not a fresh
        // reinterpretation -- same domain, same stop count/values/colors,
        // so switching S04/S05's REF rendering from the old hardcoded path
        // to this table changes nothing visually.
        use crate::palette::{default_ref_palette_stops, DEFAULT_REF_MAX_DBZ, DEFAULT_REF_MIN_DBZ};

        let table = default_color_table(MomentKind::Reflectivity);
        assert_eq!(table.domain.min, DEFAULT_REF_MIN_DBZ);
        assert_eq!(table.domain.max, DEFAULT_REF_MAX_DBZ);
        assert_eq!(table.stops, default_ref_palette_stops());
    }

    #[test]
    fn default_differential_phase_table_is_cyclic() {
        let table = default_color_table(MomentKind::DifferentialPhase);
        assert!(table.cyclic);
        assert_eq!(
            table.domain,
            ColorTableDomain {
                min: 0.0,
                max: 360.0
            }
        );
    }

    #[test]
    fn default_lut_is_non_uniform_for_every_moment_kind() {
        // A degenerate all-one-color LUT would be a real (if unlikely)
        // authoring mistake in a shipped default; this guards against it.
        for kind in ALL_MOMENT_KINDS {
            let table = default_color_table(kind);
            let lut = build_lut_from_table(&table, 256);
            let distinct: std::collections::HashSet<[u8; 4]> = lut.into_iter().collect();
            assert!(
                distinct.len() > 1,
                "{kind} default color table produced a uniform LUT"
            );
        }
    }

    // --- parsing/validation: malformed input must error, never panic ----

    #[test]
    fn from_json_rejects_invalid_json_syntax() {
        let err = ColorTable::from_json("{ not json").unwrap_err();
        assert!(matches!(err, ColorTableError::Json(_)));
    }

    #[test]
    fn from_json_rejects_unknown_field_typo() {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../color_tables/reflectivity.json")).unwrap();
        value["stopz"] = value["stops"].clone(); // typo'd field name
        let err = ColorTable::from_json(&value.to_string()).unwrap_err();
        assert!(matches!(err, ColorTableError::Json(_)));
    }

    fn minimal_valid_table() -> ColorTable {
        ColorTable {
            format_version: CURRENT_FORMAT_VERSION,
            name: "Test Table".to_string(),
            moments: vec!["REF".to_string()],
            units: "dBZ".to_string(),
            mode: ColorTableMode::Gradient,
            domain: ColorTableDomain {
                min: 0.0,
                max: 10.0,
            },
            cyclic: false,
            stops: vec![
                ColorStop::new(0.0, [0, 0, 0, 255]),
                ColorStop::new(10.0, [255, 255, 255, 255]),
            ],
            missing_color: [0, 0, 0, 0],
            range_folded_color: [255, 0, 255, 255],
        }
    }

    #[test]
    fn validate_accepts_a_well_formed_table() {
        assert!(minimal_valid_table().validate().is_ok());
    }

    #[test]
    fn validate_rejects_unsupported_format_version() {
        let mut table = minimal_valid_table();
        table.format_version = 999;
        assert!(matches!(
            table.validate(),
            Err(ColorTableError::UnsupportedFormatVersion(999))
        ));
    }

    #[test]
    fn validate_rejects_empty_name() {
        let mut table = minimal_valid_table();
        table.name = "   ".to_string();
        assert!(matches!(table.validate(), Err(ColorTableError::EmptyName)));
    }

    #[test]
    fn validate_rejects_empty_moments() {
        let mut table = minimal_valid_table();
        table.moments = vec![];
        assert!(matches!(
            table.validate(),
            Err(ColorTableError::EmptyMoments)
        ));
    }

    #[test]
    fn validate_rejects_unknown_moment_kind() {
        let mut table = minimal_valid_table();
        table.moments = vec!["RHO".to_string()]; // wire uses RHO; this format uses CC
        assert!(matches!(
            table.validate(),
            Err(ColorTableError::UnknownMomentKind(code)) if code == "RHO"
        ));
    }

    #[test]
    fn validate_rejects_empty_units() {
        let mut table = minimal_valid_table();
        table.units = "".to_string();
        assert!(matches!(table.validate(), Err(ColorTableError::EmptyUnits)));
    }

    #[test]
    fn validate_rejects_invalid_domain() {
        let mut table = minimal_valid_table();
        table.domain = ColorTableDomain {
            min: 10.0,
            max: 10.0,
        };
        assert!(matches!(
            table.validate(),
            Err(ColorTableError::InvalidDomain { .. })
        ));

        let mut table = minimal_valid_table();
        table.domain = ColorTableDomain {
            min: f32::NAN,
            max: 10.0,
        };
        assert!(matches!(
            table.validate(),
            Err(ColorTableError::InvalidDomain { .. })
        ));
    }

    #[test]
    fn validate_rejects_empty_stops() {
        let mut table = minimal_valid_table();
        table.stops = vec![];
        assert!(matches!(table.validate(), Err(ColorTableError::EmptyStops)));
    }

    #[test]
    fn validate_rejects_non_finite_stop_value() {
        let mut table = minimal_valid_table();
        table.stops[0].value = f32::NAN;
        assert!(matches!(
            table.validate(),
            Err(ColorTableError::NonFiniteStopValue { index: 0, .. })
        ));
    }

    #[test]
    fn validate_rejects_unsorted_stops() {
        let mut table = minimal_valid_table();
        table.stops = vec![
            ColorStop::new(5.0, [0, 0, 0, 255]),
            ColorStop::new(2.0, [255, 255, 255, 255]),
        ];
        assert!(matches!(
            table.validate(),
            Err(ColorTableError::StopsNotStrictlyAscending { index: 1 })
        ));
    }

    #[test]
    fn validate_rejects_duplicate_stop_values() {
        let mut table = minimal_valid_table();
        table.stops = vec![
            ColorStop::new(5.0, [0, 0, 0, 255]),
            ColorStop::new(5.0, [255, 255, 255, 255]),
        ];
        assert!(matches!(
            table.validate(),
            Err(ColorTableError::StopsNotStrictlyAscending { index: 1 })
        ));
    }

    #[test]
    fn validate_rejects_stop_value_outside_domain() {
        let mut table = minimal_valid_table();
        table.stops.push(ColorStop::new(999.0, [1, 2, 3, 4]));
        assert!(matches!(
            table.validate(),
            Err(ColorTableError::StopValueOutOfDomain { index: 2, .. })
        ));
    }

    #[test]
    fn validate_rejects_missing_color_equal_to_range_folded_color() {
        let mut table = minimal_valid_table();
        table.range_folded_color = table.missing_color;
        assert!(matches!(
            table.validate(),
            Err(ColorTableError::MissingColorEqualsRangeFoldedColor)
        ));
    }

    #[test]
    fn from_json_round_trips_through_to_json_pretty() {
        let table = minimal_valid_table();
        let json = table.to_json_pretty().expect("serializes");
        let parsed = ColorTable::from_json(&json).expect("re-parses its own output");
        assert_eq!(table, parsed);
    }

    // --- stepped vs gradient interpolation at known sample points -------

    #[test]
    fn gradient_mode_interpolates_linearly_between_stops() {
        let mut table = minimal_valid_table();
        table.mode = ColorTableMode::Gradient;
        table.stops = vec![
            ColorStop::new(0.0, [0, 0, 0, 255]),
            ColorStop::new(10.0, [200, 0, 0, 255]),
        ];
        assert_eq!(sample_value(&table, 0.0), [0, 0, 0, 255]);
        assert_eq!(sample_value(&table, 10.0), [200, 0, 0, 255]);
        assert_eq!(
            sample_value(&table, 5.0),
            lerp_color([0, 0, 0, 255], [200, 0, 0, 255], 0.5)
        );
        // Outside the stop range: clamps.
        assert_eq!(sample_value(&table, -5.0), [0, 0, 0, 255]);
        assert_eq!(sample_value(&table, 15.0), [200, 0, 0, 255]);
    }

    #[test]
    fn stepped_mode_holds_color_with_no_interpolation() {
        let mut table = minimal_valid_table();
        table.mode = ColorTableMode::Stepped;
        table.stops = vec![
            ColorStop::new(0.0, [0, 0, 0, 255]),
            ColorStop::new(5.0, [100, 0, 0, 255]),
            ColorStop::new(10.0, [200, 0, 0, 255]),
        ];
        assert_eq!(sample_value(&table, 0.0), [0, 0, 0, 255]);
        assert_eq!(sample_value(&table, 4.999), [0, 0, 0, 255]);
        assert_eq!(sample_value(&table, 5.0), [100, 0, 0, 255]);
        assert_eq!(sample_value(&table, 9.999), [100, 0, 0, 255]);
        assert_eq!(sample_value(&table, 10.0), [200, 0, 0, 255]);
    }

    // --- cyclic wraparound ------------------------------------------------

    #[test]
    fn cyclic_gradient_wraps_smoothly_across_the_domain_seam() {
        let mut table = minimal_valid_table();
        table.mode = ColorTableMode::Gradient;
        table.cyclic = true;
        table.domain = ColorTableDomain { min: 0.0, max: 4.0 };
        table.stops = vec![
            ColorStop::new(0.0, [0, 0, 0, 255]),
            ColorStop::new(2.0, [200, 0, 0, 255]),
        ];
        // Exactly at the wrap point: value 4.0 wraps to 0.0 -> first stop's
        // color, exactly matching sampling at 0.0 itself (a seamless loop,
        // not a hard clamp to whichever endpoint is "nearest").
        assert_eq!(sample_value(&table, 0.0), sample_value(&table, 4.0));
        assert_eq!(sample_value(&table, 8.0), sample_value(&table, 0.0)); // wraps twice
                                                                          // Halfway through the wrap segment (value 3.0, between the last
                                                                          // stop at 2.0 and the virtual wrapped first stop at 2.0 + 4.0 = 6.0):
                                                                          // t = (3.0 - 2.0) / (6.0 - 2.0) = 0.25.
        let expected = lerp_color([200, 0, 0, 255], [0, 0, 0, 255], 0.25);
        assert_eq!(sample_value(&table, 3.0), expected);
    }

    #[test]
    fn cyclic_stepped_wraps_the_input_value_before_stepping() {
        let mut table = minimal_valid_table();
        table.mode = ColorTableMode::Stepped;
        table.cyclic = true;
        table.domain = ColorTableDomain {
            min: 0.0,
            max: 10.0,
        };
        table.stops = vec![
            ColorStop::new(0.0, [1, 1, 1, 255]),
            ColorStop::new(5.0, [2, 2, 2, 255]),
        ];
        // 12.0 wraps to 2.0 (12.0 mod 10.0), which steps to the first stop's color.
        assert_eq!(sample_value(&table, 12.0), [1, 1, 1, 255]);
        // 17.0 wraps to 7.0, which steps to the second stop's color.
        assert_eq!(sample_value(&table, 17.0), [2, 2, 2, 255]);
    }

    // --- build_lut_from_table: degenerate input --------------------------

    #[test]
    fn build_lut_from_table_zero_texels_is_empty() {
        let table = minimal_valid_table();
        assert!(build_lut_from_table(&table, 0).is_empty());
    }

    #[test]
    fn build_lut_from_table_endpoints_match_first_and_last_stop() {
        let table = minimal_valid_table();
        let lut = build_lut_from_table(&table, 100);
        assert_eq!(lut.first().copied(), Some(table.stops[0].color));
        assert_eq!(
            lut.last().copied(),
            Some(table.stops[table.stops.len() - 1].color)
        );
    }

    // --- structural guarantee: missing/range-folded colors are never
    // reachable by sampling `stops` at any physical value ------------------

    /// This is the format's core safety property, demonstrated positively
    /// (not just documented): for a table whose `missing_color`/
    /// `range_folded_color` are chosen distinct from every stop's color, a
    /// full-resolution LUT built purely from `stops`/`domain`/`mode`/
    /// `cyclic` (exactly what `build_lut_from_table` does -- the same code
    /// path every real render uses) never produces either sentinel color
    /// at any texel. Combined with `build_lut_from_table`'s signature
    /// (it does not even take `missing_color`/`range_folded_color` as
    /// input), this shows there is no possible physical-value lookup that
    /// could yield a missing/range-folded color -- those are only ever
    /// assigned via the separate `radar_types::GateValue`/
    /// `sweep_buffers::GpuGateSample::flag` state check, never through this
    /// module's value-sampling path.
    ///
    /// A table's `missing_color` deliberately *may* coincide with a stop's
    /// color for a different, intentional reason (e.g. a shared
    /// fully-transparent "no signal" appearance) -- see the module docs and
    /// `default_reflectivity_table_matches_the_original_placeholder_ramp`,
    /// which covers that case; this test instead covers a table where the
    /// two are deliberately kept visually distinct, to demonstrate the
    /// structural guarantee holds for such tables too.
    #[test]
    fn missing_and_range_folded_colors_never_appear_in_a_sampled_lut() {
        let mut table = minimal_valid_table();
        table.mode = ColorTableMode::Gradient;
        table.domain = ColorTableDomain {
            min: -10.0,
            max: 10.0,
        };
        table.stops = vec![
            ColorStop::new(-10.0, [10, 20, 30, 255]),
            ColorStop::new(0.0, [100, 150, 200, 255]),
            ColorStop::new(10.0, [250, 240, 230, 255]),
        ];
        // Deliberately distinctive, unmistakable sentinel colors that do
        // not appear anywhere in `stops` above.
        table.missing_color = [1, 2, 3, 4];
        table.range_folded_color = [253, 252, 251, 250];
        assert!(table.validate().is_ok());

        for mode in [ColorTableMode::Gradient, ColorTableMode::Stepped] {
            for cyclic in [false, true] {
                let mut t = table.clone();
                t.mode = mode;
                t.cyclic = cyclic;
                let lut = build_lut_from_table(&t, 4096);
                assert!(
                    !lut.contains(&t.missing_color),
                    "mode={mode:?} cyclic={cyclic}: missing_color leaked into the sampled LUT"
                );
                assert!(
                    !lut.contains(&t.range_folded_color),
                    "mode={mode:?} cyclic={cyclic}: range_folded_color leaked into the sampled LUT"
                );
            }
        }
    }
}
