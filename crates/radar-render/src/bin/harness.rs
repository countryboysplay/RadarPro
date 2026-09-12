//! S03 measurement harness: proves the storage-buffer + lookup-texture GPU
//! representation renders a real decoded NEXRAD sweep, and measures GPU
//! upload time, frame time/FPS, palette-switch time, and an estimated
//! VRAM footprint.
//!
//! If no GPU adapter is available, prints a clear message and exits 0
//! (success) — this binary is meant to be run by a human on a machine
//! with a real GPU; it must never fail CI, which has none.
//!
//! Run with: `cargo run -p radar-render --bin harness`

use radar_render::camera::clip_to_world;
use radar_render::gpu::{self, GpuUniforms};
use radar_render::lookup_texture::{build_radial_lookup, DEFAULT_LOOKUP_TEXEL_COUNT};
use radar_render::palette::{
    build_palette_lut, default_ref_palette_stops, inverted_ref_palette_stops, DEFAULT_REF_MAX_DBZ,
    DEFAULT_REF_MIN_DBZ, PALETTE_TEXEL_COUNT,
};
use radar_render::sweep_buffers::{build_sweep_buffers, GpuGateSample, GpuRadialMeta};
use radar_types::{AzimuthResolution, MomentKind, Sweep, Volume};
use std::time::{Duration, Instant};

const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/nexrad-level2/KTLX20240601_000353_V06"
);
const RENDER_TARGET_SIZE: u32 = 1024;
const FRAME_COUNT: usize = 120;

fn main() {
    println!("radar-render harness: S03 GPU renderer prototype measurement run");
    println!("fixture: {FIXTURE_PATH}");

    let ctx = match pollster::block_on(gpu::GpuContext::request()) {
        Some(ctx) => ctx,
        None => {
            println!(
                "skipped: no GPU adapter available in this environment. \
                 This is expected on CI; run this harness locally on a machine with a GPU."
            );
            return;
        }
    };
    println!(
        "GPU adapter: {} ({:?} backend, {:?})",
        ctx.adapter_info.name, ctx.adapter_info.backend, ctx.adapter_info.device_type
    );

    let volume = match decode_fixture() {
        Ok(volume) => volume,
        Err(message) => {
            eprintln!("radar-render harness: {message}");
            std::process::exit(1);
        }
    };
    println!(
        "Decoded volume: site {} ({:.4}, {:.4}), {} sweeps",
        volume.site.icao,
        volume.site.latitude_deg,
        volume.site.longitude_deg,
        volume.sweeps.len()
    );

    let sweep = match pick_lowest_elevation_sweep_with_moment(&volume, MomentKind::Reflectivity) {
        Some(sweep) => sweep,
        None => {
            eprintln!("radar-render harness: no sweep in this volume carries a REF moment.");
            std::process::exit(1);
        }
    };
    println!(
        "Selected sweep: elevation {:.2} deg, {} radials",
        sweep.elevation_angle_deg,
        sweep.radials.len()
    );

    // --- Step: build CPU-side representation + upload to GPU, timed ----

    let upload_start = Instant::now();
    let buffer_data = build_sweep_buffers(sweep, MomentKind::Reflectivity);
    let azimuth_resolution = sweep
        .radials
        .first()
        .map(|r| r.azimuth_resolution)
        .unwrap_or(AzimuthResolution::One);
    let lookup_table = build_radial_lookup(
        &buffer_data.source_radials,
        azimuth_resolution,
        DEFAULT_LOOKUP_TEXEL_COUNT,
    );
    let palette_lut = build_palette_lut(
        &default_ref_palette_stops(),
        DEFAULT_REF_MIN_DBZ,
        DEFAULT_REF_MAX_DBZ,
        PALETTE_TEXEL_COUNT,
    );

    let sweep_gpu = gpu::upload_sweep(&ctx.device, &ctx.queue, &buffer_data, &lookup_table);
    let palette_gpu = gpu::upload_palette(&ctx.device, &ctx.queue, &palette_lut);

    let max_range_km = farthest_gate_edge_km(&buffer_data.radial_meta);
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
    let target = gpu::create_render_target(&ctx.device, RENDER_TARGET_SIZE, RENDER_TARGET_SIZE);
    gpu::wait_for_gpu(&ctx.device);
    let upload_duration = upload_start.elapsed();

    println!();
    println!(
        "-- Radials in buffer: {} (of {} total in sweep)",
        buffer_data.radial_meta.len(),
        sweep.radials.len()
    );
    println!("-- Gate samples: {}", buffer_data.gate_samples.len());
    println!("-- Sweep max range (farthest gate far edge): {max_range_km:.2} km");
    println!(
        "GPU upload time (CPU buffer build + wgpu upload + pipeline/target setup): {:.3} ms",
        upload_duration.as_secs_f64() * 1000.0
    );

    // --- Step: render FRAME_COUNT frames with a varying camera, timed --

    let mut frame_durations = Vec::with_capacity(FRAME_COUNT);
    for i in 0..FRAME_COUNT {
        let t = i as f32 / FRAME_COUNT as f32;
        // Simulated pan/zoom: the camera orbits and breathes in/out, so
        // every frame uploads a different `clip_to_world` (a new uniform
        // buffer write) rather than rendering the same static view
        // `FRAME_COUNT` times.
        let pan_x = (t * std::f32::consts::TAU).sin() * max_range_km * 0.2;
        let pan_y = (t * std::f32::consts::TAU * 0.5).cos() * max_range_km * 0.2;
        let zoom = 1.0 + 0.3 * (t * std::f32::consts::TAU * 2.0).sin();
        let half_extent = max_range_km * zoom;

        let frame_start = Instant::now();
        uniforms_gpu.update(
            &ctx.queue,
            GpuUniforms {
                clip_to_world: clip_to_world((pan_x, pan_y), (half_extent, half_extent)),
                site_max_range_km: max_range_km,
                lookup_texel_count: DEFAULT_LOOKUP_TEXEL_COUNT,
                palette_min_dbz: DEFAULT_REF_MIN_DBZ,
                palette_max_dbz: DEFAULT_REF_MAX_DBZ,
                _pad: [0; 4],
            },
        );
        gpu::render_frame(
            &ctx.device,
            &ctx.queue,
            &pipeline.pipeline,
            &bind_group,
            &target,
        );
        gpu::wait_for_gpu(&ctx.device);
        frame_durations.push(frame_start.elapsed());
    }

    print_frame_stats(&frame_durations);

    // --- Step: palette-switch time (swap + re-render) -------------------

    let inverted_lut = build_palette_lut(
        &inverted_ref_palette_stops(),
        DEFAULT_REF_MIN_DBZ,
        DEFAULT_REF_MAX_DBZ,
        PALETTE_TEXEL_COUNT,
    );
    let swap_start = Instant::now();
    gpu::update_palette(&ctx.queue, &palette_gpu, &inverted_lut);
    gpu::render_frame(
        &ctx.device,
        &ctx.queue,
        &pipeline.pipeline,
        &bind_group,
        &target,
    );
    gpu::wait_for_gpu(&ctx.device);
    let swap_duration = swap_start.elapsed();
    println!();
    println!(
        "Palette-switch time (texture rewrite + one re-render): {:.3} ms",
        swap_duration.as_secs_f64() * 1000.0
    );

    // Switch back to the default palette for the saved reference frame.
    gpu::update_palette(&ctx.queue, &palette_gpu, &palette_lut);

    // --- Step: estimated VRAM footprint ---------------------------------

    let uniform_bytes = std::mem::size_of::<GpuUniforms>() as u64;
    let render_target_bytes = u64::from(RENDER_TARGET_SIZE) * u64::from(RENDER_TARGET_SIZE) * 4;
    let total_bytes =
        sweep_gpu.byte_size + palette_gpu.byte_size + uniform_bytes + render_target_bytes;
    println!();
    println!(
        "Estimated VRAM footprint (sum of allocations this harness made, not a driver query):"
    );
    println!(
        "  sweep storage buffers + lookup texture: {:.1} KiB",
        sweep_gpu.byte_size as f64 / 1024.0
    );
    println!(
        "  palette texture:                        {:.3} KiB",
        palette_gpu.byte_size as f64 / 1024.0
    );
    println!("  uniform buffer:                         {uniform_bytes} bytes");
    println!(
        "  off-screen render target ({RENDER_TARGET_SIZE}x{RENDER_TARGET_SIZE} RGBA8): {:.1} KiB",
        render_target_bytes as f64 / 1024.0
    );
    println!("  total: {:.2} MiB", total_bytes as f64 / (1024.0 * 1024.0));

    // --- Step: render + save one reference frame as a PNG ---------------

    gpu::render_frame(
        &ctx.device,
        &ctx.queue,
        &pipeline.pipeline,
        &bind_group,
        &target,
    );
    gpu::wait_for_gpu(&ctx.device);
    let pixels = gpu::read_rgba8(&ctx.device, &ctx.queue, &target);

    match save_png(&pixels, RENDER_TARGET_SIZE, RENDER_TARGET_SIZE) {
        Ok(path) => println!("\nSaved reference frame to: {}", path.display()),
        Err(e) => eprintln!("\nradar-render harness: failed to save reference PNG: {e}"),
    }

    // Documented struct-size sanity check so this file stays honest about
    // what `byte_size` actually counted, visible in harness output too.
    debug_assert_eq!(std::mem::size_of::<GpuRadialMeta>(), 24);
    debug_assert_eq!(std::mem::size_of::<GpuGateSample>(), 8);
}

fn decode_fixture() -> Result<Volume, String> {
    let bytes = std::fs::read(FIXTURE_PATH)
        .map_err(|e| format!("failed to read fixture {FIXTURE_PATH}: {e}"))?;
    nexrad_level2::decode_volume(&bytes).map_err(|e| format!("failed to decode fixture: {e}"))
}

fn pick_lowest_elevation_sweep_with_moment(volume: &Volume, moment: MomentKind) -> Option<&Sweep> {
    volume
        .sweeps
        .iter()
        .filter(|sweep| {
            sweep
                .radials
                .iter()
                .any(|radial| radial.moments.contains_key(&moment))
        })
        .min_by(|a, b| {
            a.elevation_angle_deg
                .partial_cmp(&b.elevation_angle_deg)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// The farthest gate's far edge across every radial in `radial_meta`, in
/// km — used as `site_max_range_km` so the default camera frames the
/// whole sweep with no wasted transparent margin.
fn farthest_gate_edge_km(radial_meta: &[GpuRadialMeta]) -> f32 {
    radial_meta
        .iter()
        .map(|r| r.first_gate_range_km + r.gate_spacing_km * (r.gate_count as f32 - 0.5))
        .fold(0.0f32, f32::max)
        .max(1.0) // never zero/negative, even for a degenerate empty sweep
}

fn print_frame_stats(durations: &[Duration]) {
    let millis: Vec<f64> = durations.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    let sum: f64 = millis.iter().sum();
    let avg = sum / millis.len() as f64;
    let min = millis.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = millis.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    println!();
    println!(
        "Rendered {} frames with a per-frame varying camera (simulated pan/zoom):",
        durations.len()
    );
    println!("  avg frame time: {avg:.3} ms  (~{:.1} FPS)", 1000.0 / avg);
    println!("  min frame time: {min:.3} ms");
    println!(
        "  max frame time: {max:.3} ms  (~{:.1} FPS worst-case)",
        1000.0 / max
    );
    println!(
        "  (each measurement is CPU-submit + a blocking wait for GPU completion — a \
         conservative, non-pipelined measurement; there is no swapchain in this \
         off-screen harness to double/triple-buffer against.)"
    );
}

fn save_png(rgba: &[u8], width: u32, height: u32) -> Result<std::path::PathBuf, String> {
    let out_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/radar-render-harness");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let out_path = out_dir.join("ktlx_ref_sweep.png");

    image::save_buffer(&out_path, rgba, width, height, image::ColorType::Rgba8)
        .map_err(|e| e.to_string())?;

    out_path.canonicalize().map_err(|e| e.to_string())
}
