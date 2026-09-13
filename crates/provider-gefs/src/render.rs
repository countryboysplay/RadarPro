//! CPU-side, GPU-free preparation for [`crate::gpu`]: turns a decoded
//! [`crate::field::GriddedField`] into a display-unit value array plus a
//! palette lookup table -- fully unit-testable without a GPU, mirroring
//! `radar-render`'s `palette.rs`/`gpu.rs` split (everything CPU-testable
//! lives outside the `wgpu`-dependent module).
//!
//! [`crate::field::GriddedField::values`] itself is Kelvin (this field's
//! native unit) and is never touched here: [`to_display_celsius`] builds a
//! brand new `Vec`, so a display-unit choice never mutates source data
//! (GLOBAL_CONTRACT).

use crate::field::GriddedField;
use radar_render::palette::{build_palette_lut, PaletteStop};

/// Default palette domain, in Celsius, for 2m temperature. An informed UI
/// choice, not a scientifically-derived threshold set -- same status as
/// `radar-render::palette`'s own documented example ramps.
pub const DEFAULT_MIN_CELSIUS: f32 = -40.0;
pub const DEFAULT_MAX_CELSIUS: f32 = 40.0;

/// Matches `radar-render::palette::PALETTE_TEXEL_COUNT`'s reasoning: far
/// more resolution than the handful of stops need, cheap (1 KiB), smooth.
pub const PALETTE_TEXEL_COUNT: usize = 256;

/// RadarPro's own 2m-temperature color ramp: blue (cold) -> near-white
/// (around freezing) -> red (hot). This project's own original palette
/// choice (GLOBAL_CONTRACT: "its own branding... defaults... algorithms"),
/// deliberately distinct from any third-party tool's temperature ramp.
pub fn default_temperature_palette_stops() -> Vec<PaletteStop> {
    vec![
        PaletteStop::new(DEFAULT_MIN_CELSIUS, [40, 30, 120, 255]),
        PaletteStop::new(-20.0, [40, 90, 210, 255]),
        PaletteStop::new(0.0, [225, 225, 230, 255]),
        PaletteStop::new(20.0, [235, 190, 40, 255]),
        PaletteStop::new(DEFAULT_MAX_CELSIUS, [200, 30, 30, 255]),
    ]
}

/// Build the default temperature palette's LUT, ready for
/// [`crate::gpu::upload_palette`] (re-exported from `radar-render`).
pub fn build_default_palette_lut() -> Vec<[u8; 4]> {
    build_palette_lut(
        &default_temperature_palette_stops(),
        DEFAULT_MIN_CELSIUS,
        DEFAULT_MAX_CELSIUS,
        PALETTE_TEXEL_COUNT,
    )
}

/// Convert a decoded field's native-unit (Kelvin) values into a *new*
/// Celsius array for display. Panics only on the [`CanonicalField`] this
/// crate does not yet decode as temperature (unreachable today: the only
/// variant is `Temperature2m`) -- kept as a `debug_assert` rather than a
/// silent wrong conversion, since applying a Kelvin->Celsius offset to a
/// future non-temperature field would silently corrupt it.
///
/// [`CanonicalField`]: crate::field::CanonicalField
pub fn to_display_celsius(field: &GriddedField) -> Vec<f32> {
    debug_assert_eq!(
        field.unit, "K",
        "to_display_celsius assumes a Kelvin-native field"
    );
    field
        .values
        .iter()
        .map(|&kelvin| GriddedField::kelvin_to_celsius(kelvin))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ensemble::EnsembleIdentity;
    use crate::field::{CanonicalField, GridGeometry};
    use crate::time::UtcTimestamp;

    fn sample_field(values: Vec<f32>) -> GriddedField {
        GriddedField {
            field: CanonicalField::Temperature2m,
            unit: "K",
            run_time: UtcTimestamp::new(2026, 9, 12, 12, 0, 0),
            forecast_lead_hours: 0,
            valid_time: UtcTimestamp::new(2026, 9, 12, 12, 0, 0),
            ensemble: EnsembleIdentity::Control,
            geometry: GridGeometry {
                width: 2,
                height: 2,
                origin_lat_deg: 10.0,
                origin_lon_deg: 100.0,
                lat_step_deg: -1.0,
                lon_step_deg: 1.0,
            },
            values,
        }
    }

    #[test]
    fn to_display_celsius_converts_without_mutating_source() {
        let field = sample_field(vec![273.15, 373.15, 233.15, 313.15]);
        let original = field.values.clone();
        let celsius = to_display_celsius(&field);
        assert!((celsius[0] - 0.0).abs() < 1e-4);
        assert!((celsius[1] - 100.0).abs() < 1e-4);
        assert!((celsius[2] - -40.0).abs() < 1e-4);
        assert!((celsius[3] - 40.0).abs() < 1e-4);
        assert_eq!(field.values, original);
    }

    #[test]
    fn default_palette_lut_endpoints_match_first_and_last_stop() {
        let stops = default_temperature_palette_stops();
        let lut = build_default_palette_lut();
        assert_eq!(lut.first().copied(), Some(stops[0].color));
        assert_eq!(lut.last().copied(), Some(stops[stops.len() - 1].color));
    }

    #[test]
    fn default_palette_lut_is_the_documented_texel_count() {
        assert_eq!(build_default_palette_lut().len(), PALETTE_TEXEL_COUNT);
    }
}
