//! The one test in this crate that actually talks to a GPU. Every other
//! test (in `sweep_buffers`, `lookup_texture`, `palette`, `camera`) is
//! pure CPU logic and needs no GPU at all.
//!
//! Per the S03 task: this test must request a real `wgpu` adapter and, if
//! one is available, actually run the render pipeline once and assert
//! something meaningful about the output. If no adapter is available (as
//! on the GitHub Actions CI runners this workspace's `cargo test --all`
//! runs on), it must print a clear, visible skip message and pass — never
//! panic, never silently do nothing via `#[ignore]`.

use crate::camera::clip_to_world;
use crate::gpu::{self, GpuUniforms};
use crate::lookup_texture::{build_radial_lookup, DEFAULT_LOOKUP_TEXEL_COUNT};
use crate::palette::{
    build_palette_lut, default_ref_palette_stops, DEFAULT_REF_MAX_DBZ, DEFAULT_REF_MIN_DBZ,
};
use crate::sweep_buffers::build_sweep_buffers;
use radar_types::{
    AzimuthResolution, GateValue, Moment, MomentKind, Radial, RadialStatus, RadialStatusKind,
    Sweep, Timestamp,
};
use std::collections::BTreeMap;

/// A small synthetic sweep, deliberately varied so a correct render
/// cannot come out as one uniform color: even-numbered radials carry a
/// mid-range reflectivity value (should reach the visible part of the
/// palette ramp) and odd-numbered radials are entirely missing gates
/// (should render fully transparent), and gates before the "signal
/// floor" stay transparent too.
fn synthetic_test_sweep() -> Sweep {
    const GATE_COUNT: usize = 40;
    const GATE_SPACING_KM: f32 = 1.0;
    const FIRST_GATE_RANGE_KM: f32 = 1.0;

    let mut radials = Vec::with_capacity(360);
    for az in 0..360u16 {
        let gates: Vec<GateValue> = (0..GATE_COUNT)
            .map(|gate_index| {
                if az % 2 == 0 {
                    // A visible mid-ramp value from roughly gate 10 onward,
                    // transparent-floor value before that.
                    if gate_index < 10 {
                        GateValue::Value(-5.0)
                    } else {
                        GateValue::Value(35.0)
                    }
                } else {
                    GateValue::Missing
                }
            })
            .collect();

        let mut moments = BTreeMap::new();
        moments.insert(
            MomentKind::Reflectivity,
            Moment {
                first_gate_range_km: FIRST_GATE_RANGE_KM,
                gate_spacing_km: GATE_SPACING_KM,
                scale: 1.0,
                offset: 0.0,
                gates,
            },
        );

        radials.push(Radial {
            azimuth_number: az + 1,
            azimuth_angle_deg: f32::from(az),
            azimuth_resolution: AzimuthResolution::One,
            elevation_angle_deg: 0.5,
            radial_status: RadialStatus {
                kind: RadialStatusKind::Intermediate,
                bad_data: false,
            },
            collection_time: Timestamp::from_epoch_millis(0),
            moments,
        });
    }

    Sweep {
        elevation_number: 1,
        elevation_angle_deg: 0.5,
        radials,
    }
}

#[test]
fn render_pipeline_produces_a_non_uniform_image_when_a_gpu_is_available() {
    let Some(ctx) = pollster::block_on(gpu::GpuContext::request()) else {
        println!(
            "skipped: no GPU adapter available in this environment — render_pipeline_produces_a_non_uniform_image_when_a_gpu_is_available did not exercise the GPU path."
        );
        return;
    };
    println!(
        "radar-render GPU test running against adapter: {} ({:?}, {:?})",
        ctx.adapter_info.name, ctx.adapter_info.backend, ctx.adapter_info.device_type
    );

    let sweep = synthetic_test_sweep();
    let buffer_data = build_sweep_buffers(&sweep, MomentKind::Reflectivity);
    assert!(
        !buffer_data.radial_meta.is_empty(),
        "synthetic test sweep must produce non-empty GPU buffers"
    );

    let lookup_table = build_radial_lookup(
        &buffer_data.source_radials,
        AzimuthResolution::One,
        DEFAULT_LOOKUP_TEXEL_COUNT,
    );
    let palette_lut = build_palette_lut(
        &default_ref_palette_stops(),
        DEFAULT_REF_MIN_DBZ,
        DEFAULT_REF_MAX_DBZ,
        crate::palette::PALETTE_TEXEL_COUNT,
    );

    let sweep_gpu = gpu::upload_sweep(&ctx.device, &ctx.queue, &buffer_data, &lookup_table);
    let palette_gpu = gpu::upload_palette(&ctx.device, &ctx.queue, &palette_lut);

    let max_range_km = FIRST_GATE_RANGE_PLUS_SPAN_KM;
    let uniforms_gpu = gpu::UniformsGpu::new(
        &ctx.device,
        GpuUniforms {
            clip_to_world: clip_to_world((0.0, 0.0), (max_range_km, max_range_km)),
            site_max_range_km: max_range_km,
            lookup_texel_count: DEFAULT_LOOKUP_TEXEL_COUNT,
            palette_min_dbz: DEFAULT_REF_MIN_DBZ,
            palette_max_dbz: DEFAULT_REF_MAX_DBZ,
            _pad: [0; 4],
        },
    );

    let pipeline = gpu::create_pipeline(&ctx.device, gpu::RENDER_TARGET_FORMAT);
    let bind_group = gpu::create_bind_group(
        &ctx.device,
        &pipeline.bind_group_layout,
        &uniforms_gpu,
        &sweep_gpu,
        &palette_gpu,
    );
    let target = gpu::create_render_target(&ctx.device, 128, 128);

    gpu::render_frame(
        &ctx.device,
        &ctx.queue,
        &pipeline.pipeline,
        &bind_group,
        &target,
    );
    gpu::wait_for_gpu(&ctx.device);
    let pixels = gpu::read_rgba8(&ctx.device, &ctx.queue, &target);

    assert_eq!(pixels.len(), 128 * 128 * 4);

    let distinct_colors: std::collections::HashSet<[u8; 4]> = pixels
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect();
    println!(
        "radar-render GPU test: rendered {}x{} frame with {} distinct RGBA colors",
        target.width,
        target.height,
        distinct_colors.len()
    );
    assert!(
        distinct_colors.len() > 1,
        "rendered frame was a single uniform color ({distinct_colors:?}) — nothing was actually drawn"
    );

    // The even-azimuth radials' visible gates (35.0 dBZ, well inside the
    // green/yellow band) should produce at least one fully-opaque pixel,
    // proving a "valid" branch of the shader was actually reached (not
    // just the transparent-clear background from missing radials/gates).
    let has_opaque_pixel = pixels.chunks_exact(4).any(|p| p[3] == 255);
    assert!(
        has_opaque_pixel,
        "expected at least one fully-opaque (valid-gate) pixel in the rendered frame"
    );
}

const FIRST_GATE_RANGE_PLUS_SPAN_KM: f32 = 1.0 + 40.0 * 1.0; // first_gate_range + gate_count * spacing
