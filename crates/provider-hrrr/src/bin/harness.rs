//! S08 measurement/proof harness: a real, live end-to-end run of this
//! crate's whole pipeline -- discover a published HRRR run -> fetch a real
//! 2m-temperature `.idx` sidecar -> sparse Range GET one GRIB2 message ->
//! decode (Lambert Conformal grid) -> crop to a regional subset -> render
//! on the GPU via `forecast_core::gpu::render_forecast_grid` (the exact
//! same call `provider-gefs`'s own harness uses) -> save one frame as a
//! PNG.
//!
//! If no GPU adapter is available, prints a clear message and exits 0.
//!
//! Run with: `cargo run -p provider-hrrr --bin provider-hrrr-harness`

use forecast_core::provider::ForecastProvider;
use forecast_core::request::FieldRequest;
use forecast_core::variable::ForecastVariable;
use provider_hrrr::HrrrProvider;
use radar_render::camera::clip_to_world;

const RENDER_WIDTH: u32 = 1024;
const RENDER_HEIGHT: u32 = 512;

// A regional (CONUS) bounding box, in the [0, 360) longitude convention
// this crate's grid geometry uses -- matches `provider-gefs`'s own harness
// box, so the two rendered PNGs are directly, visually comparable.
const CONUS_LAT_MIN: f64 = 20.0;
const CONUS_LAT_MAX: f64 = 55.0;
const CONUS_LON_MIN: f64 = 230.0; // -130 deg E
const CONUS_LON_MAX: f64 = 300.0; // -60 deg E

#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("provider-hrrr harness: S08 HRRR forecast provider proof-of-concept run");

    let provider = match HrrrProvider::default_bucket() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("provider-hrrr harness: failed to build HTTP client: {e}");
            std::process::exit(1);
        }
    };

    let run = match provider.discover_latest_run(2).await {
        Ok(run) => run,
        Err(e) => {
            eprintln!("provider-hrrr harness: {e}");
            eprintln!(
                "This harness needs live network access to the public noaa-hrrr-bdp-pds \
                 bucket; skipping."
            );
            std::process::exit(0);
        }
    };
    println!("Using real published HRRR run: {run}");

    let request = FieldRequest::new(ForecastVariable::Temperature2m, 0);
    let field = match provider.fetch_field(&run, &request).await {
        Ok(field) => field,
        Err(e) => {
            eprintln!("provider-hrrr harness: failed to fetch/decode 2m temperature: {e}");
            std::process::exit(1);
        }
    };

    println!(
        "\n{}: {} ({}), ensemble={:?} (deterministic -- HRRR has no ensemble)",
        field.variable.canonical_name(),
        field.native.provider_variable_name,
        field.unit,
        field.ensemble
    );
    println!(
        "  run/init time: {}   forecast lead: {}h   valid time: {}",
        field.run_time, field.forecast_lead_hours, field.valid_time
    );
    println!(
        "  grid: {}x{} cells (Lambert Conformal Conic, ~3km CONUS)",
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
        forecast_core::grid::ForecastGrid::kelvin_to_celsius(min),
        max,
        forecast_core::grid::ForecastGrid::kelvin_to_celsius(max),
        mean,
        forecast_core::grid::ForecastGrid::kelvin_to_celsius(mean)
    );

    let Some(subset) = field.subset(CONUS_LAT_MIN, CONUS_LAT_MAX, CONUS_LON_MIN, CONUS_LON_MAX)
    else {
        eprintln!("provider-hrrr harness: CONUS bounding box did not overlap the decoded grid");
        std::process::exit(1);
    };
    println!(
        "\nCropped field to a regional subset: {}x{} cells (from the full {}x{} grid -- HRRR's \
         native domain is already CONUS-only, so this crop mostly trims a small border)",
        subset.geometry.width(),
        subset.geometry.height(),
        field.geometry.width(),
        field.geometry.height()
    );

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
        "{opaque_count} of {} pixels are fully opaque (inside the grid)",
        RENDER_WIDTH as usize * RENDER_HEIGHT as usize
    );

    match save_png(&pixels, RENDER_WIDTH, RENDER_HEIGHT) {
        Ok(path) => println!("\nSaved reference frame to: {}", path.display()),
        Err(e) => eprintln!("\nprovider-hrrr harness: failed to save reference PNG: {e}"),
    }

    println!(
        "\nThis is NOAA HRRR model forecast guidance ({}, valid at {}), not observed radar.",
        subset.variable.canonical_name(),
        subset.valid_time
    );
}

fn save_png(rgba: &[u8], width: u32, height: u32) -> Result<std::path::PathBuf, String> {
    let out_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/provider-hrrr-harness");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let out_path = out_dir.join("hrrr_conus_temperature_2m.png");

    image::save_buffer(&out_path, rgba, width, height, image::ColorType::Rgba8)
        .map_err(|e| e.to_string())?;

    out_path.canonicalize().map_err(|e| e.to_string())
}
