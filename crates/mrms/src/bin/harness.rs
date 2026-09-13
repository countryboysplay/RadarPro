//! S09 Phase 1 measurement/proof harness: a real, live end-to-end run of
//! this crate's whole pipeline, for *both* products this stage implements
//! -- discover the latest published MRMS snapshot -> fetch the whole
//! (gzip-wrapped) GRIB2 object -> gunzip -> decode (including missing-value
//! sentinel classification) -> print real min/max/mean + missing/coverage
//! stats -> render on a real GPU via `mrms::palette::render_mrms_grid`
//! (`forecast_core::gpu::render_forecast_grid`, the exact same shared
//! renderer GEFS/HRRR use) -> save one PNG per product.
//!
//! If no GPU adapter is available, prints a clear message and exits 0
//! (success) -- same convention as `provider-gefs`/`provider-hrrr`'s own
//! harnesses.
//!
//! Run with: `cargo run -p mrms --bin mrms-harness`

use mrms::client::MrmsClient;
use mrms::decode::decode_field;
use mrms::grid::MrmsGrid;
use mrms::keys::MrmsProduct;
use radar_render::camera::clip_to_world;

const RENDER_WIDTH: u32 = 1400;
const RENDER_HEIGHT: u32 = 700;

/// Bounded backward walk (in whole UTC days) for
/// [`MrmsClient::discover_latest_snapshot`] -- one day is already generous
/// given MRMS's ~2-minute cadence (a real snapshot should always exist
/// within the current or immediately prior UTC day).
const LOOKBACK_DAYS: u32 = 1;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("mrms harness: S09 Phase 1 MRMS national mosaic + precip-rate observation run");

    let client = match MrmsClient::default_bucket() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mrms harness: failed to build HTTP client: {e}");
            std::process::exit(1);
        }
    };

    for product in [
        MrmsProduct::ReflectivityQcComposite,
        MrmsProduct::PrecipRate,
    ] {
        run_one_product(&client, product).await;
    }
}

async fn run_one_product(client: &MrmsClient, product: MrmsProduct) {
    println!("\n=== {} ({}) ===", product.display_name(), product);

    let snapshot = match client
        .discover_latest_snapshot(product, LOOKBACK_DAYS)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mrms harness: {e}");
            eprintln!(
                "This harness needs live network access to the public noaa-mrms-pds bucket; \
                 skipping this product."
            );
            return;
        }
    };
    println!("Using real published snapshot: {snapshot}");

    let key = snapshot.object_key();
    let bytes = match client.fetch_and_decompress(&key).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("mrms harness: failed to fetch/gunzip {key}: {e}");
            return;
        }
    };
    println!("Fetched and decompressed {} bytes", bytes.len());

    let grid = match decode_field(&key, &bytes, product) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("mrms harness: failed to decode {key}: {e}");
            return;
        }
    };

    print_summary(&grid);

    let Some((center_lon, center_lat, half_extent_lon, half_extent_lat)) = camera_frame_for(&grid)
    else {
        eprintln!("mrms harness: could not compute a camera frame for this grid");
        return;
    };

    let pixels = mrms::palette::render_mrms_grid(
        &grid,
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
        "{opaque_count} of {} pixels are fully opaque",
        RENDER_WIDTH as usize * RENDER_HEIGHT as usize
    );

    match save_png(&pixels, product) {
        Ok(path) => println!("Saved reference frame to: {}", path.display()),
        Err(e) => eprintln!("mrms harness: failed to save reference PNG: {e}"),
    }

    println!(
        "\nThis is a real NOAA MRMS radar-mosaic OBSERVATION ({}, valid at {}), not model \
         forecast guidance.",
        product.display_name(),
        grid.valid_time
    );
}

fn print_summary(grid: &MrmsGrid) {
    let stats = grid.stats();
    println!("  valid (observation) time: {}", grid.valid_time);
    println!(
        "  grid: {}x{} cells (regular lat/lon, 0.01 deg spacing)",
        grid.geometry.width(),
        grid.geometry.height()
    );
    println!(
        "  min={:.3}{u} max={:.3}{u} mean={:.3}{u}  (computed over real values only)",
        stats.min,
        stats.max,
        stats.mean,
        u = grid.unit
    );
    let total = grid.values.len();
    println!(
        "  valid={} ({:.1}%)  missing(covered, no echo)={} ({:.1}%)  no_coverage={} ({:.1}%)",
        stats.valid_count,
        100.0 * stats.valid_count as f64 / total as f64,
        stats.missing_count,
        100.0 * stats.missing_count as f64 / total as f64,
        stats.no_coverage_count,
        100.0 * stats.no_coverage_count as f64 / total as f64,
    );
}

/// Frame the camera on this grid's own real-world bounding box (all four
/// corners, computed provider-agnostically via `GridGeometry::lon_lat_for_cell`
/// -- same approach `provider-hrrr`'s cross-provider test uses), with a
/// small margin so the frame's own edge lands just outside the grid.
fn camera_frame_for(grid: &MrmsGrid) -> Option<(f32, f32, f32, f32)> {
    let (width, height) = (grid.geometry.width(), grid.geometry.height());
    if width == 0 || height == 0 {
        return None;
    }
    let corners = [
        grid.geometry.lon_lat_for_cell(0, 0),
        grid.geometry.lon_lat_for_cell(0, width - 1),
        grid.geometry.lon_lat_for_cell(height - 1, 0),
        grid.geometry.lon_lat_for_cell(height - 1, width - 1),
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
    let half_extent_lon = (((lon_max - lon_min) / 2.0) * 1.02) as f32;
    let half_extent_lat = (((lat_max - lat_min) / 2.0) * 1.02) as f32;
    Some((center_lon, center_lat, half_extent_lon, half_extent_lat))
}

fn save_png(rgba: &[u8], product: MrmsProduct) -> Result<std::path::PathBuf, String> {
    let out_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/mrms-harness");
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let file_name = match product {
        MrmsProduct::ReflectivityQcComposite => "mrms_conus_reflectivity.png",
        MrmsProduct::PrecipRate => "mrms_conus_precip_rate.png",
    };
    let out_path = out_dir.join(file_name);

    image::save_buffer(
        &out_path,
        rgba,
        RENDER_WIDTH,
        RENDER_HEIGHT,
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;

    out_path.canonicalize().map_err(|e| e.to_string())
}
