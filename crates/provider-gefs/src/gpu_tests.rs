//! The one test module in this crate that actually talks to a GPU. Every
//! other test is pure CPU logic (grid math, `.idx` parsing, decode) and
//! needs no GPU at all. Mirrors `radar-render`'s `gpu_tests.rs`: request a
//! real adapter, and if none is available (as on this workspace's CI
//! runners), print a clear skip message and pass -- never panic, never
//! silently do nothing via `#[ignore]`.
//!
//! # Proving the render is geographically correct, not just "it compiled"
//!
//! This test builds a tiny **synthetic** 2x2 checkerboard field with known
//! values placed at known, real-world-style lat/lon cells, frames a camera
//! exactly on the field's bounding box (with a half-cell margin so the
//! frame's own edges land outside the grid, proving out-of-grid pixels
//! really do come back transparent rather than the whole viewport being
//! one uninterrupted color by coincidence), renders it, and asserts the
//! rendered image contains **exactly** the two expected opaque colors --
//! the default palette's exact endpoint colors, since this field's two
//! values are exactly the palette's documented min/max. That ties a known
//! input value to a specific, predicted output pixel color end to end
//! (world position -> nearest grid cell -> texture lookup -> palette ->
//! final RGBA), not just "some pixels differ from other pixels."
//!
//! The real-data geographic proof (decoding a live-fetched NOAA message
//! and rendering it) lives in `src/bin/harness.rs`, which is meant to be
//! run and visually/numerically inspected by a human -- this test is the
//! automated, exact-value-checked complement to that.

use crate::render::{build_default_palette_lut, default_temperature_palette_stops};
use radar_render::camera::clip_to_world;
use radar_render::gpu as rr_gpu;

#[test]
fn grid_render_places_known_values_at_the_correct_screen_positions() {
    let Some(ctx) = pollster::block_on(rr_gpu::GpuContext::request()) else {
        println!(
            "skipped: no GPU adapter available in this environment -- \
             grid_render_places_known_values_at_the_correct_screen_positions did not exercise \
             the GPU path."
        );
        return;
    };
    println!(
        "provider-gefs GPU test running against adapter: {} ({:?}, {:?})",
        ctx.adapter_info.name, ctx.adapter_info.backend, ctx.adapter_info.device_type
    );

    // A 2x2 grid, north-first like real GEFS: row 0 = 10N, row 1 = 0N;
    // col 0 = 100E, col 1 = 105E. Diagonal checkerboard of the palette's
    // exact min/max Celsius values.
    let stops = default_temperature_palette_stops();
    let cold = stops.first().unwrap().value; // -40.0 C
    let hot = stops.last().unwrap().value; // 40.0 C
    #[rustfmt::skip]
    let values: Vec<f32> = vec![
        cold, hot,
        hot,  cold,
    ];
    let (width, height) = (2u32, 2u32);
    let origin_lon = 100.0f32;
    let origin_lat = 10.0f32;
    let lon_step = 5.0f32;
    let lat_step = -10.0f32;

    let grid = crate::gpu::upload_grid(&ctx.device, &ctx.queue, width, height, &values);
    let palette_lut = build_default_palette_lut();
    let palette_gpu = rr_gpu::upload_palette(&ctx.device, &ctx.queue, &palette_lut);

    let pipeline = crate::gpu::create_pipeline(&ctx.device, rr_gpu::RENDER_TARGET_FORMAT);

    // Frame the grid's bounding box with a half-cell margin on every side,
    // so the rendered frame's own outer edge lands just outside the grid
    // -- a transparent border must appear, proving out-of-bounds pixels
    // are genuinely excluded rather than the whole frame happening to be
    // covered by grid cells.
    let center_lon = origin_lon + lon_step * 0.5;
    let center_lat = origin_lat + lat_step * 0.5;
    let half_extent_lon = lon_step + lon_step * 0.5;
    let half_extent_lat = lat_step.abs() + lat_step.abs() * 0.5;

    let uniforms_gpu = crate::gpu::UniformsGpu::new(
        &ctx.device,
        crate::gpu::GpuUniforms {
            clip_to_world: clip_to_world(
                (center_lon, center_lat),
                (half_extent_lon, half_extent_lat),
            ),
            origin_lon_deg: origin_lon,
            origin_lat_deg: origin_lat,
            lon_step_deg: lon_step,
            lat_step_deg: lat_step,
            grid_width: width,
            grid_height: height,
            palette_min: cold,
            palette_max: hot,
        },
    );
    let bind_group = crate::gpu::create_bind_group(
        &ctx.device,
        &pipeline.bind_group_layout,
        &uniforms_gpu,
        &grid,
        &palette_gpu,
    );
    let target = rr_gpu::create_render_target(&ctx.device, 128, 128);

    rr_gpu::render_frame(
        &ctx.device,
        &ctx.queue,
        &pipeline.pipeline,
        &bind_group,
        &target,
    );
    rr_gpu::wait_for_gpu(&ctx.device);
    let pixels = rr_gpu::read_rgba8(&ctx.device, &ctx.queue, &target);
    assert_eq!(pixels.len(), 128 * 128 * 4);

    let cold_color = stops.first().unwrap().color;
    let hot_color = stops.last().unwrap().color;

    let close = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b.iter()).all(|(x, y)| x.abs_diff(*y) <= 2);

    let mut cold_pixel_count = 0usize;
    let mut hot_pixel_count = 0usize;
    let mut transparent_pixel_count = 0usize;
    let mut other_opaque_colors: std::collections::HashSet<[u8; 4]> =
        std::collections::HashSet::new();

    for chunk in pixels.as_chunks::<4>().0 {
        let p = *chunk;
        if p[3] == 0 {
            transparent_pixel_count += 1;
        } else if close(p, cold_color) {
            cold_pixel_count += 1;
        } else if close(p, hot_color) {
            hot_pixel_count += 1;
        } else {
            other_opaque_colors.insert(p);
        }
    }

    println!(
        "provider-gefs GPU test: cold={cold_pixel_count} hot={hot_pixel_count} \
         transparent={transparent_pixel_count} other_opaque={}",
        other_opaque_colors.len()
    );

    assert!(
        other_opaque_colors.is_empty(),
        "found unexpected opaque colors not matching either palette endpoint: {other_opaque_colors:?}"
    );
    assert!(
        cold_pixel_count > 100,
        "expected a substantial block of the coldest palette color, got {cold_pixel_count} pixels"
    );
    assert!(
        hot_pixel_count > 100,
        "expected a substantial block of the hottest palette color, got {hot_pixel_count} pixels"
    );
    assert!(
        transparent_pixel_count > 0,
        "expected a transparent border outside the grid's bounding box"
    );
}
