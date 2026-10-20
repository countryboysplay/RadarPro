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
        crate::keys::MrmsProduct::MeshMax60min => ramp_palette(&[
            (0.0, [0, 0, 0, 0]), // no hail: transparent
            (2.0, [120, 200, 255, 230]),
            (19.0, [40, 200, 90, 255]), // ~0.75 in
            (25.0, [230, 220, 0, 255]), // 1 in, severe threshold
            (44.0, [230, 120, 0, 255]), // 1.75 in
            (51.0, [220, 0, 0, 255]),   // 2 in, significant
            (76.0, [200, 0, 220, 255]), // 3 in+
        ]),
        crate::keys::MrmsProduct::EchoTop18 => ramp_palette(&[
            (0.0, [0, 0, 0, 0]), // no echo: transparent
            (1.0, [120, 200, 255, 200]),
            (6.0, [40, 130, 240, 255]),
            (10.0, [40, 200, 90, 255]),
            (13.0, [230, 220, 0, 255]),
            (16.0, [220, 0, 0, 255]),
            (20.0, [200, 0, 220, 255]),
        ]),
        crate::keys::MrmsProduct::Vil => ramp_palette(&[
            (0.0, [0, 0, 0, 0]), // none: transparent
            (1.0, [120, 200, 255, 200]),
            (10.0, [40, 200, 90, 255]),
            (30.0, [230, 220, 0, 255]),
            (50.0, [230, 120, 0, 255]),
            (70.0, [220, 0, 0, 255]),
            (90.0, [200, 0, 220, 255]),
        ]),
        crate::keys::MrmsProduct::Qpe01h | crate::keys::MrmsProduct::Qpe03h => ramp_palette(&[
            (0.0, [0, 0, 0, 0]), // no rain: transparent
            (0.25, [120, 200, 255, 220]),
            (2.5, [40, 130, 240, 255]),
            (10.0, [40, 200, 90, 255]),
            (25.0, [230, 220, 0, 255]),
            (50.0, [230, 120, 0, 255]),
            (75.0, [220, 0, 0, 255]),
            (100.0, [200, 0, 220, 255]),
        ]),
        // Category codes, so flat bands of +-0.5 around each code (the LUT
        // interpolates between stops). Rain greens, snow blue, hail magenta.
        crate::keys::MrmsProduct::PrecipFlag => ramp_palette(&[
            (0.5, [0, 0, 0, 0]), // none: transparent
            (0.5, [60, 170, 70, 200]),
            (1.5, [60, 170, 70, 200]), // 1 warm stratiform rain
            (1.5, [0, 0, 0, 0]),
            (2.5, [0, 0, 0, 0]),
            (2.5, [80, 150, 255, 220]),
            (3.5, [80, 150, 255, 220]), // 3 snow
            (3.5, [0, 0, 0, 0]),
            (5.5, [0, 0, 0, 0]),
            (5.5, [240, 200, 30, 230]),
            (6.5, [240, 200, 30, 230]), // 6 convective rain
            (6.5, [220, 60, 220, 240]),
            (7.5, [220, 60, 220, 240]), // 7 rain with hail
            (7.5, [0, 0, 0, 0]),
            (9.5, [0, 0, 0, 0]),
            (9.5, [40, 120, 60, 200]),
            (10.5, [40, 120, 60, 200]), // 10 cold stratiform rain
            (10.5, [0, 0, 0, 0]),
            (90.5, [0, 0, 0, 0]),
            (90.5, [60, 170, 70, 200]),
            (91.5, [60, 170, 70, 200]), // 91 tropical stratiform rain
            (91.5, [0, 0, 0, 0]),
            (95.5, [0, 0, 0, 0]),
            (95.5, [240, 200, 30, 230]),
            (96.5, [240, 200, 30, 230]), // 96 tropical convective rain
        ]),
        crate::keys::MrmsProduct::LightningCg5min => ramp_palette(&[
            (0.0, [0, 0, 0, 0]),       // no flashes: transparent
            (0.5, [230, 220, 0, 255]), // ~1 flash per cell
            (2.0, [230, 120, 0, 255]),
            (5.0, [220, 0, 0, 255]),
            (10.0, [255, 255, 255, 255]),
        ]),
    }
}

/// Build a palette whose domain spans the first to last stop. Same
/// convention as the precip-rate ramp: the first stop is transparent so
/// "zero"/no-data cells vanish.
fn ramp_palette(stops: &[(f32, [u8; 4])]) -> MrmsPalette {
    let stops: Vec<PaletteStop> = stops.iter().map(|&(v, c)| PaletteStop::new(v, c)).collect();
    let min = stops[0].value;
    let max = stops[stops.len() - 1].value;
    MrmsPalette {
        lut: build_palette_lut(&stops, min, max, PALETTE_TEXEL_COUNT),
        min,
        max,
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
        let table = default_color_table(MomentKind::Reflectivity);
        assert_eq!(palette.min, table.domain.min);
        assert_eq!(palette.max, table.domain.max);
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

    #[test]
    fn precip_type_codes_each_land_on_their_own_color() {
        let palette = palette_for(crate::keys::MrmsProduct::PrecipFlag);
        // Both texels around each code, as the GPU's linear sampler reads them.
        let texels = |code: f32| {
            let u =
                (code - palette.min) / (palette.max - palette.min) * (palette.lut.len() - 1) as f32;
            [
                palette.lut[u.floor() as usize],
                palette.lut[u.ceil() as usize],
            ]
        };
        let [none, _] = texels(0.0);
        assert_eq!(none[3], 0);
        for code in [1.0, 3.0, 6.0, 7.0, 10.0, 91.0, 96.0] {
            let [a, b] = texels(code);
            assert_eq!(a, b, "code {code} straddles a band edge");
            assert!(a[3] > 0, "code {code} is transparent");
        }
        assert_ne!(texels(3.0)[0], texels(1.0)[0]); // snow vs rain
        assert_ne!(texels(7.0)[0], texels(6.0)[0]); // hail vs convective
    }
}
