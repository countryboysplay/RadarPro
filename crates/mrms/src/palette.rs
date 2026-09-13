//! Color palettes for this crate's two products.
//!
//! Reflectivity reuses NEXRAD REF's existing, already-documented color
//! table (`radar_render::color_table::default_color_table`) rather than
//! inventing a new one -- MRMS's `MergedReflectivityQCComposite` is the
//! same physical quantity (dBZ) `radar_types::MomentKind::Reflectivity`
//! already has a validated ramp for (S05, `docs/adr/0009-original-color-table-format.md`).
//!
//! `PrecipRate` (mm/hr) has no existing color table anywhere in this
//! codebase (checked: `crates/radar-render/color_tables/*.json` covers only
//! the six polar-radar `MomentKind`s, none of them precipitation rate) --
//! per this stage's brief, "a simple documented one is fine if not". Rather
//! than force a mm/hr quantity through `radar_render::color_table::ColorTable`
//! (which validates its `moments` field against `radar_types::MomentKind`,
//! a polar-radar-moment vocabulary MRMS's gridded precipitation rate isn't
//! part of), this reuses that same crate's lower-level, moment-agnostic
//! [`radar_render::palette::build_palette_lut`]/[`radar_render::palette::PaletteStop`]
//! primitives directly -- the exact machinery the REF ramp itself is built
//! from, just without the `ColorTable`/`MomentKind` wrapper this quantity
//! doesn't fit.

use forecast_core::grid::GridGeometry;
use radar_render::color_table::{build_lut_from_table, default_color_table};
use radar_render::palette::{build_palette_lut, PaletteStop, PALETTE_TEXEL_COUNT};
use radar_types::MomentKind;

/// A built palette LUT plus the physical-value domain it covers (texel 0 ->
/// `min`, last texel -> `max`) -- everything
/// `forecast_core::gpu::render_forecast_grid` needs for its
/// `palette_lut`/`palette_min`/`palette_max` parameters.
pub struct MrmsPalette {
    pub lut: Vec<[u8; 4]>,
    pub min: f32,
    pub max: f32,
}

/// Reflectivity palette: the exact NEXRAD REF ramp, unmodified.
pub fn reflectivity_palette() -> MrmsPalette {
    let table = default_color_table(MomentKind::Reflectivity);
    let lut = build_lut_from_table(&table, PALETTE_TEXEL_COUNT);
    MrmsPalette {
        lut,
        min: table.domain.min,
        max: table.domain.max,
    }
}

/// Lower bound of this crate's `PrecipRate` ramp (mm/hr). Values at or
/// below this (including this crate's `NoCoverage`/`Missing`-fill sentinel,
/// which callers should set to something `<= PRECIP_RATE_MIN_MM_PER_HOUR`)
/// clamp to the first stop's fully-transparent color.
pub const PRECIP_RATE_MIN_MM_PER_HOUR: f32 = 0.0;
/// Upper bound of this crate's `PrecipRate` ramp (mm/hr) -- chosen above
/// every real value observed during this stage's live verification (max
/// 175 mm/hr for an extreme convective cell), with headroom for other
/// events.
pub const PRECIP_RATE_MAX_MM_PER_HOUR: f32 = 200.0;

/// A simple, documented precipitation-rate ramp (mm/hr): fully transparent
/// at/just above zero (no rain), then a conventional light-blue -> green ->
/// yellow -> orange -> red -> magenta progression as intensity increases --
/// not scientifically calibrated or branded, matching this stage's own
/// "a simple documented one is fine" bar for a first cut.
pub fn precip_rate_palette_stops() -> Vec<PaletteStop> {
    vec![
        PaletteStop::new(PRECIP_RATE_MIN_MM_PER_HOUR, [0, 0, 0, 0]), // no rain: transparent
        PaletteStop::new(0.1, [120, 200, 255, 220]),                 // trace: light blue
        PaletteStop::new(2.0, [40, 130, 240, 255]),                  // light rain: blue
        PaletteStop::new(8.0, [40, 200, 90, 255]),                   // moderate: green
        PaletteStop::new(20.0, [230, 220, 0, 255]),                  // heavy: yellow
        PaletteStop::new(40.0, [230, 120, 0, 255]),                  // very heavy: orange
        PaletteStop::new(80.0, [220, 0, 0, 255]),                    // intense convective: red
        PaletteStop::new(PRECIP_RATE_MAX_MM_PER_HOUR, [200, 0, 220, 255]), // extreme: magenta
    ]
}

pub fn precip_rate_palette() -> MrmsPalette {
    let stops = precip_rate_palette_stops();
    let lut = build_palette_lut(
        &stops,
        PRECIP_RATE_MIN_MM_PER_HOUR,
        PRECIP_RATE_MAX_MM_PER_HOUR,
        PALETTE_TEXEL_COUNT,
    );
    MrmsPalette {
        lut,
        min: PRECIP_RATE_MIN_MM_PER_HOUR,
        max: PRECIP_RATE_MAX_MM_PER_HOUR,
    }
}

/// The palette for `product`.
pub fn palette_for(product: crate::keys::MrmsProduct) -> MrmsPalette {
    match product {
        crate::keys::MrmsProduct::ReflectivityQcComposite => reflectivity_palette(),
        crate::keys::MrmsProduct::PrecipRate => precip_rate_palette(),
    }
}

/// Render `grid` on a real GPU, if one is available -- thin convenience
/// wrapper over `forecast_core::gpu::render_forecast_grid` (the exact same
/// shared renderer GEFS/HRRR use, reused unmodified: no new pipeline/
/// shader/palette-upload code) that picks `fill_value` as one step below
/// the palette's own domain minimum, so every `NoCoverage`/`Missing` cell
/// clamps to that palette's first, fully-transparent stop -- the same
/// "clamp below domain renders transparent" convention the REF ramp (and
/// every other palette in this codebase) already documents, applied
/// deliberately rather than by accident.
pub async fn render_mrms_grid(
    grid: &crate::grid::MrmsGrid,
    clip_to_world: radar_render::camera::Mat4,
    render_width: u32,
    render_height: u32,
) -> Option<Vec<u8>> {
    let palette = palette_for(grid.product);
    let fill_value = palette.min - 1.0;
    let display_values = grid.display_values(fill_value);
    let geometry: &GridGeometry = &grid.geometry;
    forecast_core::gpu::render_forecast_grid(
        geometry,
        &display_values,
        &palette.lut,
        palette.min,
        palette.max,
        clip_to_world,
        render_width,
        render_height,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflectivity_palette_matches_the_shared_ref_ramp_domain() {
        let palette = reflectivity_palette();
        assert_eq!(palette.min, radar_render::palette::DEFAULT_REF_MIN_DBZ);
        assert_eq!(palette.max, radar_render::palette::DEFAULT_REF_MAX_DBZ);
        assert_eq!(palette.lut.len(), PALETTE_TEXEL_COUNT);
    }

    #[test]
    fn precip_rate_palette_is_transparent_at_zero_and_opaque_at_high_rates() {
        let palette = precip_rate_palette();
        assert_eq!(palette.lut.first().copied(), Some([0, 0, 0, 0]));
        assert_eq!(
            palette.lut.last().copied().map(|c| c[3]),
            Some(255),
            "the most intense rate stop must be fully opaque"
        );
    }

    #[test]
    fn precip_rate_palette_is_non_uniform() {
        let palette = precip_rate_palette();
        let distinct: std::collections::HashSet<[u8; 4]> = palette.lut.iter().copied().collect();
        assert!(distinct.len() > 1);
    }
}
