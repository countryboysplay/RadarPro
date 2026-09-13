//! S07/S08 measurement/proof harness: a real, live end-to-end run of this
//! crate's whole pipeline -- discover a published GEFS run -> fetch three
//! real ensemble identities' `.idx` sidecars -> sparse Range GET one
//! GRIB2 message per identity -> decode -> crop to a regional (CONUS)
//! subset -> render on the GPU via `forecast_core::gpu::render_forecast_grid`
//! (S08: the exact same call `provider-hrrr`'s own harness uses) -> save
//! one frame as a PNG so a human (or this task's own verification) can
//! confirm the render is geographically and physically correct.
//!
//! If no GPU adapter is available, prints a clear message and exits 0
//! (success).
//!
//! Run with: `cargo run -p provider-gefs --bin provider-gefs-harness`

use forecast_core::ensemble::EnsembleStatistic;
use forecast_core::grid::ForecastGrid;
use forecast_core::provider::ForecastProvider;
use forecast_core::request::FieldRequest;
use forecast_core::variable::ForecastVariable;
use provider_gefs::keys::RunReference;
use provider_gefs::GefsProvider;
use radar_render::camera::clip_to_world;

const RENDER_WIDTH: u32 = 1024;
const RENDER_HEIGHT: u32 = 512;

// A regional (CONUS) bounding box, in the [0, 360) longitude convention
// this crate's grid geometry uses -- the "viewport-aware/regional subset
// selection" the S07 stage file calls for, applied to the already-decoded
// grid (GRIB2 offers no sub-message seek point to avoid decoding the
// whole message in the first place).
const CONUS_LAT_MIN: f64 = 20.0;
const CONUS_LAT_MAX: f64 = 55.0;
const CONUS_LON_MIN: f64 = 230.0; // -130 deg E
const CONUS_LON_MAX: f64 = 300.0; // -60 deg E

#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("provider-gefs harness: S07/S08 GEFS ensemble forecast provider run");

    let provider = match GefsProvider::default_bucket() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("provider-gefs harness: failed to build HTTP client: {e}");
            std::process::exit(1);
        }
    };

    let run = match provider.discover_latest_run(3).await {
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

    let control = match fetch(&provider, run, EnsembleStatistic::Control).await {
        Ok(field) => field,
        Err(e) => {
            eprintln!("provider-gefs harness: failed to fetch/decode control member: {e}");
            std::process::exit(1);
        }
    };
    let member1 = match fetch(&provider, run, EnsembleStatistic::Member(1)).await {
        Ok(field) => field,
        Err(e) => {
            eprintln!("provider-gefs harness: failed to fetch/decode perturbed member 1: {e}");
            std::process::exit(1);
        }
    };
    let mean = match fetch(&provider, run, EnsembleStatistic::Mean).await {
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
    // real "probabilistic angle" GEFS replaced WeatherNext 3 for S07
    // (spread/member access, not just a single deterministic value).
    if let (Some(point), Some((row, col))) = (
        control.point_forecast(360.0 - 97.5, 35.5),
        control.geometry.nearest_cell(360.0 - 97.5, 35.5),
    ) {
        let m1 = member1.value_at(row, col);
        let avg = mean.value_at(row, col);
        println!(
            "\nNear Oklahoma City: control={:.2}K member1={:.2}K mean={:.2}K -- member1 differs \
             from control by {:.2}K (this is the ensemble spread S07 exists to prove access to)",
            point.value,
            m1,
            avg,
            (m1 - point.value).abs()
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
        subset.geometry.width(),
        subset.geometry.height(),
        mean.geometry.width(),
        mean.geometry.height()
    );

    // --- GPU render (S08: the exact same forecast-core call path
    // provider-hrrr's harness uses) ----------------------------------------
    let celsius_values = forecast_core::render::to_display_celsius(&subset);
    let palette_lut = forecast_core::render::build_default_palette_lut();

    let center_lon = ((CONUS_LON_MIN + CONUS_LON_MAX) / 2.0) as f32;
    let center_lat = ((CONUS_LAT_MIN + CONUS_LAT_MAX) / 2.0) as f32;
    let half_extent_lon = ((CONUS_LON_MAX - CONUS_LON_MIN) / 2.0) as f32;
    let half_extent_lat = ((CONUS_LAT_MAX - CONUS_LAT_MIN) / 2.0) as f32;

    let pixels = forecast_core::gpu::render_forecast_grid(
        &subset,
        &celsius_values,
        &palette_lut,
        forecast_core::render::DEFAULT_MIN_CELSIUS,
        forecast_core::render::DEFAULT_MAX_CELSIUS,
        clip_to_world((center_lon, center_lat), (half_extent_lon, half_extent_lat)),
        RENDER_WIDTH,
        RENDER_HEIGHT,
    )
    .await;

    let Some(pixels) = pixels else {
        println!(
            "\nskipped: no GPU adapter available in this environment. This is expected on CI \
             with no GPU; run this harness locally on a machine with one."
        );
        return;
    };

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
        subset.variable.canonical_name(),
        subset.valid_time
    );
}

async fn fetch(
    provider: &GefsProvider,
    run: RunReference,
    statistic: EnsembleStatistic,
) -> Result<ForecastGrid, provider_gefs::GefsError> {
    let request = FieldRequest::new(ForecastVariable::Temperature2m, 0).with_ensemble(statistic);
    provider.fetch_field(&run, &request).await
}

fn print_field_summary(label: &str, field: &ForecastGrid) {
    println!(
        "\n{label}: {} ({}), ensemble={:?}",
        field.variable.canonical_name(),
        field.unit,
        field.ensemble
    );
    println!(
        "  run/init time: {}   forecast lead: {}h   valid time: {}",
        field.run_time, field.forecast_lead_hours, field.valid_time
    );
    println!(
        "  grid: {}x{} cells",
        field.geometry.width(),
        field.geometry.height()
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
        ForecastGrid::kelvin_to_celsius(min),
        max,
        ForecastGrid::kelvin_to_celsius(max),
        mean,
        ForecastGrid::kelvin_to_celsius(mean)
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
