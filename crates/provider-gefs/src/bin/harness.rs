//! S07 measurement/proof harness: a real, live end-to-end run of this
//! crate's whole pipeline -- discover a published GEFS run -> fetch three
//! real ensemble identities' `.idx` sidecars -> sparse Range GET one
//! GRIB2 message per identity -> decode -> crop to a regional (CONUS)
//! subset -> render on the GPU -> save one frame as a PNG so a human (or
//! this task's own verification) can confirm the render is geographically
//! and physically correct.
//!
//! If no GPU adapter is available, prints a clear message and exits 0
//! (success) -- mirrors `radar-render`'s `src/bin/harness.rs`: this binary
//! is meant to be run by a human on a machine with a real GPU (or here, in
//! CI with a software adapter); it must never fail a GPU-less run.
//!
//! Run with: `cargo run -p provider-gefs --bin provider-gefs-harness`

use provider_gefs::client::GefsClient;
use provider_gefs::decode::decode_field;
use provider_gefs::ensemble::EnsembleIdentity;
use provider_gefs::field::{CanonicalField, GriddedField};
use provider_gefs::gpu as gefs_gpu;
use provider_gefs::idx;
use provider_gefs::keys::{ForecastHour, MemberKey, ProductGroup};
use provider_gefs::render;
use radar_render::camera::clip_to_world;
use radar_render::gpu as rr_gpu;

const RENDER_WIDTH: u32 = 1024;
const RENDER_HEIGHT: u32 = 512;

// A regional (CONUS) bounding box, in the [0, 360) longitude convention
// this crate's `GridGeometry` uses -- the "viewport-aware/regional subset
// selection" the S07 stage file calls for, applied to the already-decoded
// grid (GRIB2 offers no sub-message seek point to avoid decoding the
// whole message in the first place; see `field::GriddedField::subset`'s
// doc comment).
const CONUS_LAT_MIN: f64 = 20.0;
const CONUS_LAT_MAX: f64 = 55.0;
const CONUS_LON_MIN: f64 = 230.0; // -130 deg E
const CONUS_LON_MAX: f64 = 300.0; // -60 deg E

#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("provider-gefs harness: S07 GEFS ensemble forecast proof-of-concept run");

    let client = match GefsClient::default_bucket() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("provider-gefs harness: failed to build HTTP client: {e}");
            std::process::exit(1);
        }
    };

    let run = match client
        .find_recent_run(ProductGroup::PGRB2S_P25, ForecastHour(0), 3)
        .await
    {
        Ok(run) => run,
        Err(e) => {
            eprintln!("provider-gefs harness: {e}");
            eprintln!(
                "This harness needs live network access to the public noaa-gefs-pds bucket; \
                 skipping."
            );
            std::process::exit(0);
        }
    };
    println!("Using real published GEFS run: {run}");

    let control = match fetch_and_decode(&client, run, MemberKey::Control).await {
        Ok(field) => field,
        Err(e) => {
            eprintln!("provider-gefs harness: failed to fetch/decode control member: {e}");
            std::process::exit(1);
        }
    };
    let member1 = match fetch_and_decode(&client, run, MemberKey::Perturbed(1)).await {
        Ok(field) => field,
        Err(e) => {
            eprintln!("provider-gefs harness: failed to fetch/decode perturbed member 1: {e}");
            std::process::exit(1);
        }
    };
    let mean = match fetch_and_decode(&client, run, MemberKey::Mean).await {
        Ok(field) => field,
        Err(e) => {
            eprintln!("provider-gefs harness: failed to fetch/decode ensemble mean: {e}");
            std::process::exit(1);
        }
    };

    for (label, field) in [
        ("gec00 (control)", &control),
        ("gep01 (member 1)", &member1),
        ("geavg (mean)", &mean),
    ] {
        print_field_summary(label, field);
    }

    // Ensemble spread at a fixed point (Oklahoma City, roughly), computed
    // directly from the three independently decoded fields above -- the
    // real "probabilistic angle" GEFS replaced WeatherNext 3 for for S07
    // (spread/member access, not just a single deterministic value).
    if let Some((row, col)) = control.geometry.nearest_cell(360.0 - 97.5, 35.5) {
        let c = control.value_at(row, col);
        let m1 = member1.value_at(row, col);
        let avg = mean.value_at(row, col);
        println!(
            "\nNear Oklahoma City (grid cell row={row}, col={col}): control={:.2}K \
             member1={:.2}K mean={:.2}K -- member1 differs from control by {:.2}K \
             (this is the ensemble spread S07 exists to prove access to)",
            c,
            m1,
            avg,
            (m1 - c).abs()
        );
    }

    // --- Regional subset (see this file's module doc) --------------------
    let Some(subset) = mean.subset(CONUS_LAT_MIN, CONUS_LAT_MAX, CONUS_LON_MIN, CONUS_LON_MAX)
    else {
        eprintln!("provider-gefs harness: CONUS bounding box did not overlap the decoded grid");
        std::process::exit(1);
    };
    println!(
        "\nCropped ensemble-mean field to a regional (CONUS) subset: {}x{} cells (from the full \
         {}x{} global grid)",
        subset.geometry.width, subset.geometry.height, mean.geometry.width, mean.geometry.height
    );

    // --- GPU render --------------------------------------------------------
    let Some(ctx) = rr_gpu::GpuContext::request().await else {
        println!(
            "\nskipped: no GPU adapter available in this environment. This is expected on CI \
             with no GPU; run this harness locally on a machine with one."
        );
        return;
    };
    println!(
        "GPU adapter: {} ({:?} backend, {:?})",
        ctx.adapter_info.name, ctx.adapter_info.backend, ctx.adapter_info.device_type
    );

    let celsius_values = render::to_display_celsius(&subset);
    let palette_lut = render::build_default_palette_lut();

    let grid_gpu = gefs_gpu::upload_grid(
        &ctx.device,
        &ctx.queue,
        subset.geometry.width,
        subset.geometry.height,
        &celsius_values,
    );
    let palette_gpu = rr_gpu::upload_palette(&ctx.device, &ctx.queue, &palette_lut);

    let pipeline = gefs_gpu::create_pipeline(&ctx.device, rr_gpu::RENDER_TARGET_FORMAT);

    let center_lon = (CONUS_LON_MIN + CONUS_LON_MAX) / 2.0;
    let center_lat = (CONUS_LAT_MIN + CONUS_LAT_MAX) / 2.0;
    let half_extent_lon = (CONUS_LON_MAX - CONUS_LON_MIN) / 2.0;
    let half_extent_lat = (CONUS_LAT_MAX - CONUS_LAT_MIN) / 2.0;

    let uniforms_gpu = gefs_gpu::UniformsGpu::new(
        &ctx.device,
        gefs_gpu::GpuUniforms {
            clip_to_world: clip_to_world(
                (center_lon as f32, center_lat as f32),
                (half_extent_lon as f32, half_extent_lat as f32),
            ),
            origin_lon_deg: subset.geometry.origin_lon_deg as f32,
            origin_lat_deg: subset.geometry.origin_lat_deg as f32,
            lon_step_deg: subset.geometry.lon_step_deg as f32,
            lat_step_deg: subset.geometry.lat_step_deg as f32,
            grid_width: subset.geometry.width,
            grid_height: subset.geometry.height,
            palette_min: render::DEFAULT_MIN_CELSIUS,
            palette_max: render::DEFAULT_MAX_CELSIUS,
        },
    );
    let bind_group = gefs_gpu::create_bind_group(
        &ctx.device,
        &pipeline.bind_group_layout,
        &uniforms_gpu,
        &grid_gpu,
        &palette_gpu,
    );
    let target = rr_gpu::create_render_target(&ctx.device, RENDER_WIDTH, RENDER_HEIGHT);

    rr_gpu::render_frame(
        &ctx.device,
        &ctx.queue,
        &pipeline.pipeline,
        &bind_group,
        &target,
    );
    rr_gpu::wait_for_gpu(&ctx.device);
    let pixels = rr_gpu::read_rgba8(&ctx.device, &ctx.queue, &target);

    let distinct_colors: std::collections::HashSet<[u8; 4]> =
        pixels.as_chunks::<4>().0.iter().copied().collect();
    println!(
        "Rendered {RENDER_WIDTH}x{RENDER_HEIGHT} frame with {} distinct RGBA colors",
        distinct_colors.len()
    );
    let opaque_count = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|c| c[3] == 255)
        .count();
    println!(
        "{opaque_count} of {} pixels are fully opaque (inside the CONUS grid)",
        RENDER_WIDTH as usize * RENDER_HEIGHT as usize
    );

    match save_png(&pixels, RENDER_WIDTH, RENDER_HEIGHT) {
        Ok(path) => println!("\nSaved reference frame to: {}", path.display()),
        Err(e) => eprintln!("\nprovider-gefs harness: failed to save reference PNG: {e}"),
    }

    println!(
        "\nThis is NOAA GEFS model forecast guidance (ensemble mean, {} valid at {}), not \
         observed radar.",
        subset.field.canonical_name(),
        subset.valid_time
    );
}

async fn fetch_and_decode(
    client: &GefsClient,
    run: provider_gefs::keys::RunReference,
    member: MemberKey,
) -> Result<GriddedField, String> {
    let field = CanonicalField::Temperature2m;
    let key =
        provider_gefs::keys::object_key(run, member, ProductGroup::PGRB2S_P25, ForecastHour(0));
    let idx_key =
        provider_gefs::keys::idx_key(run, member, ProductGroup::PGRB2S_P25, ForecastHour(0));

    let idx_text = client
        .fetch_idx_text(&idx_key)
        .await
        .map_err(|e| format!("fetch .idx for {member}: {e}"))?;
    let entries = idx::parse_idx(&idx_key, &idx_text).map_err(|e| format!("parse .idx: {e}"))?;
    let position = entries
        .iter()
        .position(|e| e.variable == field.idx_variable() && e.level == field.idx_level())
        .ok_or_else(|| {
            format!(
                "{} / {} not found in .idx for {member}",
                field.idx_variable(),
                field.idx_level()
            )
        })?;

    let content_length = if position + 1 == entries.len() {
        Some(
            client
                .content_length(&key)
                .await
                .map_err(|e| format!("HEAD {member}: {e}"))?,
        )
    } else {
        None
    };
    let (start, end) = idx::byte_range(&key, &entries, position, content_length)
        .map_err(|e| format!("byte_range: {e}"))?;

    println!(
        "Fetching {member} (byte range [{start}, {end}), {} bytes -- one field out of the \
         multi-megabyte file, not the whole thing)...",
        end - start
    );
    let bytes = client
        .fetch_byte_range(&key, start, end)
        .await
        .map_err(|e| format!("range GET {member}: {e}"))?;

    decode_field(&key, &bytes, field).map_err(|e| format!("decode {member}: {e}"))
}

fn print_field_summary(label: &str, field: &GriddedField) {
    println!(
        "\n{label}: {} ({}), ensemble={:?}",
        field.field.canonical_name(),
        field.unit,
        field.ensemble
    );
    println!(
        "  run/init time: {}   forecast lead: {}h   valid time: {}",
        field.run_time, field.forecast_lead_hours, field.valid_time
    );
    println!(
        "  grid: {}x{} cells, origin ({:.3}, {:.3}), step ({:.3}, {:.3})",
        field.geometry.width,
        field.geometry.height,
        field.geometry.origin_lat_deg,
        field.geometry.origin_lon_deg,
        field.geometry.lat_step_deg,
        field.geometry.lon_step_deg
    );
    let min = field.values.iter().cloned().fold(f32::INFINITY, f32::min);
    let max = field
        .values
        .iter()
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);
    let mean: f32 = field.values.iter().sum::<f32>() / field.values.len() as f32;
    println!(
        "  min={:.2}K ({:.1}C)  max={:.2}K ({:.1}C)  mean={:.2}K ({:.1}C)",
        min,
        GriddedField::kelvin_to_celsius(min),
        max,
        GriddedField::kelvin_to_celsius(max),
        mean,
        GriddedField::kelvin_to_celsius(mean)
    );
    debug_assert!(
        !matches!(field.ensemble, EnsembleIdentity::Member(0)),
        "member 0 is not a real GEFS identity"
    );
}

fn save_png(rgba: &[u8], width: u32, height: u32) -> Result<std::path::PathBuf, String> {
    let out_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/provider-gefs-harness");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let out_path = out_dir.join("gefs_conus_temperature_2m.png");

    image::save_buffer(&out_path, rgba, width, height, image::ColorType::Rgba8)
        .map_err(|e| e.to_string())?;

    out_path.canonicalize().map_err(|e| e.to_string())
}
