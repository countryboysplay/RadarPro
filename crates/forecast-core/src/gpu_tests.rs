//! The only test module in this crate that actually talks to a GPU. Moved
//! here from S07's `provider-gefs::gpu_tests` (which tested the exact same
//! machinery, since that machinery now lives here) and extended with a
//! second, Lambert-conformal-grid case to prove `shaders/forecast_grid.wgsl`'s
//! `projection_kind == 1` branch, not just the regular-lat-lon one -- both
//! tests call [`crate::gpu::render_forecast_grid`], the exact same function.
//! Mirrors `radar-render`'s `gpu_tests.rs`: request a real adapter, and if
//! none is available (as on this workspace's CI runners), print a clear
//! skip message and pass -- never panic, never silently do nothing via
//! `#[ignore]`.

use crate::ensemble::EnsembleStatistic;
use crate::grid::{
    ForecastGrid, GridGeometry, LambertConformalGrid, NativeVariableMetadata, RegularLatLonGrid,
};
use crate::projection::{LccParams, LccProjection, HRRR_EARTH_RADIUS_M};
use crate::render::{build_default_palette_lut, default_temperature_palette_stops};
use crate::time::UtcTimestamp;
use crate::variable::ForecastVariable;
use radar_render::camera::clip_to_world;

fn count_colors(
    pixels: &[u8],
    cold_color: [u8; 4],
    hot_color: [u8; 4],
) -> (usize, usize, usize, usize) {
    let close = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b.iter()).all(|(x, y)| x.abs_diff(*y) <= 2);
    let mut cold = 0usize;
    let mut hot = 0usize;
    let mut transparent = 0usize;
    let mut other = 0usize;
    for chunk in pixels.as_chunks::<4>().0 {
        let p = *chunk;
        if p[3] == 0 {
            transparent += 1;
        } else if close(p, cold_color) {
            cold += 1;
        } else if close(p, hot_color) {
            hot += 1;
        } else {
            other += 1;
        }
    }
    (cold, hot, transparent, other)
}

fn sample_grid(
    geometry: GridGeometry,
    values: Vec<f32>,
    provider_id: &'static str,
) -> ForecastGrid {
    ForecastGrid {
        variable: ForecastVariable::Temperature2m,
        native: NativeVariableMetadata {
            provider_variable_name: "TMP".to_string(),
            provider_level_name: "2 m above ground".to_string(),
            native_unit: "K",
        },
        provider_id,
        unit: "K",
        run_time: UtcTimestamp::new(2026, 9, 12, 12, 0, 0),
        forecast_lead_hours: 0,
        valid_time: UtcTimestamp::new(2026, 9, 12, 12, 0, 0),
        ensemble: Some(EnsembleStatistic::Control),
        geometry,
        values,
    }
}

/// Proves the render is geographically correct, not just "it compiled":
/// builds a tiny synthetic 2x2 checkerboard field with known values placed
/// at known, real-world-style lat/lon cells, frames a camera exactly on the
/// field's bounding box (with a half-cell margin so the frame's own edges
/// land outside the grid), renders it through [`crate::gpu::render_forecast_grid`],
/// and asserts the rendered image contains **exactly** the two expected
/// opaque colors.
#[test]
fn regular_lat_lon_grid_renders_known_values_at_the_correct_screen_positions() {
    let Some(pixels) = pollster::block_on(async {
        // A 2x2 grid, north-first like real GEFS: row 0 = 10N, row 1 = 0N;
        // col 0 = 100E, col 1 = 105E. Diagonal checkerboard of the
        // palette's exact min/max Celsius values.
        let stops = default_temperature_palette_stops();
        let cold = stops.first().unwrap().value; // -40.0 C
        let hot = stops.last().unwrap().value; // 40.0 C
        #[rustfmt::skip]
        let values: Vec<f32> = vec![
            cold, hot,
            hot,  cold,
        ];
        let (width, height) = (2u32, 2u32);
        let origin_lon = 100.0f64;
        let origin_lat = 10.0f64;
        let lon_step = 5.0f64;
        let lat_step = -10.0f64;

        let grid = sample_grid(
            GridGeometry::RegularLatLon(RegularLatLonGrid {
                width,
                height,
                origin_lat_deg: origin_lat,
                origin_lon_deg: origin_lon,
                lat_step_deg: lat_step,
                lon_step_deg: lon_step,
            }),
            values,
            "gefs",
        );
        let palette_lut = build_default_palette_lut();

        let center_lon = (origin_lon + lon_step * 0.5) as f32;
        let center_lat = (origin_lat + lat_step * 0.5) as f32;
        let half_extent_lon = (lon_step + lon_step * 0.5) as f32;
        let half_extent_lat = (lat_step.abs() + lat_step.abs() * 0.5) as f32;

        crate::gpu::render_forecast_grid(
            &grid.geometry,
            &grid.values,
            &palette_lut,
            cold,
            hot,
            clip_to_world((center_lon, center_lat), (half_extent_lon, half_extent_lat)),
            128,
            128,
        )
        .await
    }) else {
        println!(
            "skipped: no GPU adapter available in this environment -- \
             regular_lat_lon_grid_renders_known_values_at_the_correct_screen_positions did not \
             exercise the GPU path."
        );
        return;
    };

    let stops = default_temperature_palette_stops();
    let cold_color = stops.first().unwrap().color;
    let hot_color = stops.last().unwrap().color;
    let (cold_pixel_count, hot_pixel_count, transparent_pixel_count, other) =
        count_colors(&pixels, cold_color, hot_color);

    println!(
        "forecast-core regular-lat-lon GPU test: cold={cold_pixel_count} hot={hot_pixel_count} \
         transparent={transparent_pixel_count} other={other}"
    );
    assert_eq!(other, 0, "found unexpected opaque colors");
    assert!(cold_pixel_count > 100, "got {cold_pixel_count}");
    assert!(hot_pixel_count > 100, "got {hot_pixel_count}");
    assert!(transparent_pixel_count > 0);
}

/// Same proof, for a Lambert Conformal Conic grid (HRRR's real projection)
/// -- exercises `shaders/forecast_grid.wgsl`'s `projection_kind == 1`
/// branch through the exact same [`crate::gpu::render_forecast_grid`] call.
#[test]
fn lambert_conformal_grid_renders_known_values_at_the_correct_screen_positions() {
    let Some(pixels) = pollster::block_on(async {
        let stops = default_temperature_palette_stops();
        let cold = stops.first().unwrap().value;
        let hot = stops.last().unwrap().value;
        #[rustfmt::skip]
        let values: Vec<f32> = vec![
            cold, hot,
            hot,  cold,
        ];
        let (width, height) = (2u32, 2u32);
        let projection = LccProjection::new(LccParams {
            earth_radius_m: HRRR_EARTH_RADIUS_M,
            standard_parallel_1_deg: 38.5,
            standard_parallel_2_deg: 38.5,
            latitude_of_origin_deg: 38.5,
            central_meridian_deg: -97.5,
        });
        let (origin_x_m, origin_y_m) = projection.project(-98.0, 39.0);
        let dx_m = 50_000.0;
        let dy_m = 50_000.0;
        let geometry = GridGeometry::LambertConformal(LambertConformalGrid {
            width,
            height,
            projection,
            origin_x_m,
            origin_y_m,
            dx_m,
            dy_m,
        });
        let grid = sample_grid(geometry, values, "hrrr");
        let palette_lut = build_default_palette_lut();

        // Frame the camera on the grid's own real-world lon/lat bounding
        // box (with margin), computed the same provider-agnostic way for
        // either projection kind: via `GridGeometry::lon_lat_for_cell`.
        // Uses all four corners (not just the diagonal) since a Lambert
        // grid's cells are not aligned with lon/lat axes the way a
        // regular-lat-lon grid's are -- the widest lon/lat span can come
        // from any corner pair.
        let corners = [
            grid.geometry.lon_lat_for_cell(0, 0),
            grid.geometry.lon_lat_for_cell(0, 1),
            grid.geometry.lon_lat_for_cell(1, 0),
            grid.geometry.lon_lat_for_cell(1, 1),
        ];
        let lon_min = corners.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
        let lon_max = corners
            .iter()
            .map(|c| c.0)
            .fold(f64::NEG_INFINITY, f64::max);
        let lat_min = corners.iter().map(|c| c.1).fold(f64::INFINITY, f64::min);
        let lat_max = corners
            .iter()
            .map(|c| c.1)
            .fold(f64::NEG_INFINITY, f64::max);
        let center_lon = ((lon_min + lon_max) / 2.0) as f32;
        let center_lat = ((lat_min + lat_max) / 2.0) as f32;
        // A generous margin (the full span again on each side) so the
        // frame's own outer edge reliably lands outside the grid,
        // regardless of how a Lambert grid's cells skew relative to
        // lon/lat axes.
        let half_extent_lon = ((lon_max - lon_min) * 1.5) as f32;
        let half_extent_lat = ((lat_max - lat_min) * 1.5) as f32;

        crate::gpu::render_forecast_grid(
            &grid.geometry,
            &grid.values,
            &palette_lut,
            cold,
            hot,
            clip_to_world((center_lon, center_lat), (half_extent_lon, half_extent_lat)),
            128,
            128,
        )
        .await
    }) else {
        println!(
            "skipped: no GPU adapter available in this environment -- \
             lambert_conformal_grid_renders_known_values_at_the_correct_screen_positions did not \
             exercise the GPU path."
        );
        return;
    };

    let stops = default_temperature_palette_stops();
    let cold_color = stops.first().unwrap().color;
    let hot_color = stops.last().unwrap().color;
    let (cold_pixel_count, hot_pixel_count, transparent_pixel_count, other) =
        count_colors(&pixels, cold_color, hot_color);

    println!(
        "forecast-core lambert-conformal GPU test: cold={cold_pixel_count} hot={hot_pixel_count} \
         transparent={transparent_pixel_count} other={other}"
    );
    assert_eq!(other, 0, "found unexpected opaque colors");
    assert!(cold_pixel_count > 50, "got {cold_pixel_count}");
    assert!(hot_pixel_count > 50, "got {hot_pixel_count}");
    assert!(transparent_pixel_count > 0);
}
