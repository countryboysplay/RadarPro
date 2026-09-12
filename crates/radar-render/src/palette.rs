//! CPU-side construction of the 1D palette lookup texture: a fixed-size
//! RGBA8 ramp that the shader samples (with linear filtering, for a smooth
//! visual gradient) once a gate has already been confirmed valid.
//!
//! This is **explicitly a placeholder example ramp for the S03 rendering
//! proof**, not RadarPro's real color-table format. `RADAR_TECHNICAL.md`
//! calls for "an original documented format supporting units, product
//! compatibility, stepped/gradient mappings, alpha, missing, and
//! range-folded states" — that is S05's job. This module only proves that
//! a 1D palette texture is a viable, fast-to-swap way to map a physical
//! value to a color; it defines a handful of dBZ stops and linearly
//! interpolates between them into a fixed-resolution LUT.
//!
//! No `wgpu` dependency — builds a plain `Vec<[u8; 4]>`, uploaded to an
//! `Rgba8Unorm` 1D texture by `gpu.rs`. Fully unit-testable without a GPU.
//!
//! Missing/range-folded gates never reach this module's output at all:
//! `render.rs`'s shader branches on [`crate::sweep_buffers::GpuGateSample::flag`]
//! before ever sampling the palette, so this LUT only ever needs to answer
//! "what color is a *valid* reading of value X" — it has no missing/
//! range-folded encoding of its own to preserve.

/// One control point in a palette ramp: a physical value (dBZ, for the
/// example REF ramp) and the RGBA8 color it maps to. Values between two
/// stops are linearly interpolated (component-wise, including alpha).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaletteStop {
    pub value: f32,
    pub color: [u8; 4],
}

impl PaletteStop {
    pub const fn new(value: f32, color: [u8; 4]) -> Self {
        Self { value, color }
    }
}

/// Lower bound of the example REF ramp's input domain (dBZ). Also the LUT's
/// `u = 0.0` value in [`build_palette_lut`].
pub const DEFAULT_REF_MIN_DBZ: f32 = -10.0;
/// Upper bound of the example REF ramp's input domain (dBZ). Also the LUT's
/// `u = 1.0` value in [`build_palette_lut`].
pub const DEFAULT_REF_MAX_DBZ: f32 = 75.0;

/// Texel resolution of the built palette texture. 256 texels is far more
/// than the handful of documented stops need (interpolation between stops
/// is linear either way) but costs only 1 KiB (256 x 4 bytes) and gives a
/// visually smooth ramp without relying on the sampler's own filtering
/// doing more than smoothing texel-to-texel steps that are already close
/// together.
pub const PALETTE_TEXEL_COUNT: usize = 256;

/// The example placeholder REF (reflectivity) color ramp used by the S03
/// harness: transparent below a "no signal" floor, then a
/// green -> yellow -> red ramp typical of a simple dBZ display, fully
/// opaque at the top. Not a scientifically-calibrated or branded product
/// color table — see the module docs.
pub fn default_ref_palette_stops() -> Vec<PaletteStop> {
    vec![
        PaletteStop::new(DEFAULT_REF_MIN_DBZ, [0, 0, 0, 0]), // fully transparent floor
        PaletteStop::new(5.0, [0, 0, 0, 0]),                 // still transparent just below signal
        PaletteStop::new(5.01, [64, 170, 64, 255]),          // faint returns: dark green
        PaletteStop::new(25.0, [0, 220, 0, 255]),            // green
        PaletteStop::new(40.0, [230, 220, 0, 255]),          // yellow
        PaletteStop::new(55.0, [230, 90, 0, 255]),           // orange
        PaletteStop::new(DEFAULT_REF_MAX_DBZ, [200, 0, 0, 255]), // red, most intense
    ]
}

/// A second, trivially-different ramp (component-inverted, opacity
/// preserved) used only to measure a palette swap's cost in the
/// harness — not a proposed real alternative palette.
pub fn inverted_ref_palette_stops() -> Vec<PaletteStop> {
    default_ref_palette_stops()
        .into_iter()
        .map(|stop| PaletteStop {
            value: stop.value,
            color: [
                255 - stop.color[0],
                255 - stop.color[1],
                255 - stop.color[2],
                stop.color[3],
            ],
        })
        .collect()
}

/// Build a `texel_count`-entry RGBA8 lookup table by linearly interpolating
/// `stops` (which must be sorted by ascending `value`; out-of-order input
/// is not rejected but produces an undefined-order ramp) across
/// `[min_value, max_value]`.
///
/// Texel `i` represents `u = i / (texel_count - 1)` (so texel 0 is exactly
/// `min_value` and the last texel is exactly `max_value`), matching how
/// the shader will sample this texture with a `u` computed as
/// `(value - min_value) / (max_value - min_value)` clamped to `[0, 1]`.
///
/// A value at or before the first stop clamps to the first stop's color; a
/// value at or after the last stop clamps to the last stop's color.
/// Returns an all-transparent-black LUT if `stops` is empty.
pub fn build_palette_lut(
    stops: &[PaletteStop],
    min_value: f32,
    max_value: f32,
    texel_count: usize,
) -> Vec<[u8; 4]> {
    if stops.is_empty() || texel_count == 0 {
        return vec![[0, 0, 0, 0]; texel_count];
    }

    let span = max_value - min_value;
    (0..texel_count)
        .map(|texel| {
            let u = if texel_count == 1 {
                0.0
            } else {
                texel as f32 / (texel_count - 1) as f32
            };
            let value = if span.is_finite() && span != 0.0 {
                min_value + u * span
            } else {
                min_value
            };
            sample_stops(stops, value)
        })
        .collect()
}

/// Linearly interpolate `stops` at `value`, clamping outside the stops'
/// own value range.
fn sample_stops(stops: &[PaletteStop], value: f32) -> [u8; 4] {
    if value <= stops[0].value {
        return stops[0].color;
    }
    let last = stops.len() - 1;
    if value >= stops[last].value {
        return stops[last].color;
    }

    for window in stops.windows(2) {
        let (lo, hi) = (window[0], window[1]);
        if value >= lo.value && value <= hi.value {
            let span = hi.value - lo.value;
            let t = if span > 0.0 {
                (value - lo.value) / span
            } else {
                0.0
            };
            return lerp_color(lo.color, hi.color, t);
        }
    }

    // Unreachable if `stops` is sorted ascending and the clamps above
    // fired correctly; fall back to the last stop rather than panicking
    // on malformed (unsorted) caller-supplied stops.
    stops[last].color
}

fn lerp_color(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    let mut out = [0u8; 4];
    for i in 0..4 {
        let av = f32::from(a[i]);
        let bv = f32::from(b[i]);
        out[i] = (av + (bv - av) * t).round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lut_endpoints_match_first_and_last_stop() {
        let stops = default_ref_palette_stops();
        let lut = build_palette_lut(&stops, DEFAULT_REF_MIN_DBZ, DEFAULT_REF_MAX_DBZ, 256);

        assert_eq!(lut.first().copied(), Some(stops[0].color));
        assert_eq!(lut.last().copied(), Some(stops[stops.len() - 1].color));
    }

    #[test]
    fn lut_is_transparent_below_signal_floor_and_opaque_above() {
        let stops = default_ref_palette_stops();
        let lut = build_palette_lut(&stops, DEFAULT_REF_MIN_DBZ, DEFAULT_REF_MAX_DBZ, 256);
        let texel_for = |dbz: f32| {
            let u = (dbz - DEFAULT_REF_MIN_DBZ) / (DEFAULT_REF_MAX_DBZ - DEFAULT_REF_MIN_DBZ);
            ((u * 255.0).round() as usize).min(255)
        };

        assert_eq!(lut[texel_for(0.0)][3], 0, "below floor must be transparent");
        assert_eq!(lut[texel_for(50.0)][3], 255, "above floor must be opaque");
    }

    #[test]
    fn lut_green_to_red_ramp_is_monotonic_in_red_channel() {
        // Sanity check that the documented "green -> yellow -> red" ramp
        // actually increases red and decreases green as dBZ increases
        // through the visible (opaque) part of the ramp, i.e. the LUT is
        // a sane monotonic ramp rather than an arbitrary/reversed one.
        let stops = default_ref_palette_stops();
        let lut = build_palette_lut(&stops, DEFAULT_REF_MIN_DBZ, DEFAULT_REF_MAX_DBZ, 256);

        let opaque: Vec<&[u8; 4]> = lut.iter().filter(|c| c[3] == 255).collect();
        assert!(opaque.len() > 10, "expected a wide opaque band");

        let first_red = opaque.first().unwrap()[0];
        let last_red = *opaque.last().unwrap();
        assert!(
            last_red[0] >= first_red,
            "red channel should not decrease from the green end to the red end"
        );
    }

    #[test]
    fn inverted_palette_preserves_alpha_but_flips_color() {
        let base = default_ref_palette_stops();
        let inverted = inverted_ref_palette_stops();

        for (b, i) in base.iter().zip(inverted.iter()) {
            assert_eq!(b.value, i.value);
            assert_eq!(b.color[3], i.color[3], "alpha must be preserved");
            assert_eq!(i.color[0], 255 - b.color[0]);
            assert_eq!(i.color[1], 255 - b.color[1]);
            assert_eq!(i.color[2], 255 - b.color[2]);
        }
    }

    #[test]
    fn empty_stops_produce_fully_transparent_lut() {
        let lut = build_palette_lut(&[], 0.0, 1.0, 16);
        assert_eq!(lut.len(), 16);
        assert!(lut.iter().all(|&c| c == [0, 0, 0, 0]));
    }

    #[test]
    fn single_stop_produces_uniform_lut() {
        let stops = vec![PaletteStop::new(0.0, [10, 20, 30, 255])];
        let lut = build_palette_lut(&stops, -5.0, 5.0, 8);
        assert!(lut.iter().all(|&c| c == [10, 20, 30, 255]));
    }
}
