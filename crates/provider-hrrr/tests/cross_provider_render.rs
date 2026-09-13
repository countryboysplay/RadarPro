//! S08 Part D: proves `forecast-core`'s abstraction actually holds --
//! decodes one real field from GEFS and one real field from HRRR, then
//! renders **both** through the exact same `forecast_core::gpu::render_forecast_grid`
//! call, with no provider-specific branching anywhere in this test's own
//! render call (the only per-provider code here is *fetching* each field,
//! which necessarily differs -- once each is a `forecast_core::grid::ForecastGrid`,
//! every remaining line is identical for both).
//!
//! This is the literal test of the S08 stage file's own rule: "if HRRR
//! requires provider-specific conditionals throughout the UI, improve the
//! abstraction rather than special-casing broadly." The two providers'
//! real grids are geometrically about as different as this workspace has
//! (GEFS: global, regular latitude/longitude; HRRR: CONUS-only, Lambert
//! Conformal Conic) -- if one render function handles both correctly, the
//! abstraction holds for a third, not-yet-written provider too.
//!
//! Like this workspace's other live-network tests, every fallible network
//! step prints a clear skip message and returns (passing) rather than
//! failing the suite on a machine/moment lacking connectivity; a message
//! that reaches this test's own render/pixel assertions is a genuine bug,
//! not a flake, if it then fails.
//!
//! Also saves each rendered frame as a PNG (mirroring `provider-gefs`'s and
//! `provider-hrrr`'s own `src/bin/harness.rs` binaries) so a human can
//! visually confirm both: `target/cross-provider-render/gefs_temperature_2m.png`
//! (a rectangle -- GEFS's regular lat/lon grid) and
//! `target/cross-provider-render/hrrr_temperature_2m.png` (a curved
//! trapezoid -- HRRR's real Lambert Conformal grid, rendered onto the same
//! flat-plane camera).

use forecast_core::ensemble::EnsembleStatistic;
use forecast_core::grid::{ForecastGrid, GridGeometry};
use forecast_core::provider::ForecastProvider;
use forecast_core::request::FieldRequest;
use forecast_core::variable::ForecastVariable;
use radar_render::camera::clip_to_world;

const RENDER_WIDTH: u32 = 512;
const RENDER_HEIGHT: u32 = 384;

/// The one, shared, provider-agnostic render call this test exists to
/// prove: no `if provider_id == "gefs"` anywhere in this function or its
/// callee (`forecast_core::gpu::render_forecast_grid`) -- only
/// `grid.geometry`'s own projection kind is ever branched on, inside
/// `forecast-core` itself.
async fn render_and_save(grid: &ForecastGrid, out_name: &str) -> Option<Vec<u8>> {
    let celsius_values = forecast_core::render::to_display_celsius(grid);
    let palette_lut = forecast_core::render::build_default_palette_lut();

    // Frame the camera on the grid's own real-world bounding box (widest
    // lon/lat span across all four corners), computed identically
    // regardless of projection kind via `GridGeometry::lon_lat_for_cell`.
    let (width, height) = (grid.geometry.width(), grid.geometry.height());
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
    let half_extent_lon = (((lon_max - lon_min) / 2.0) * 1.05) as f32;
    let half_extent_lat = (((lat_max - lat_min) / 2.0) * 1.05) as f32;

    let pixels = forecast_core::gpu::render_forecast_grid(
        &grid.geometry,
        &celsius_values,
        &palette_lut,
        forecast_core::render::DEFAULT_MIN_CELSIUS,
        forecast_core::render::DEFAULT_MAX_CELSIUS,
        clip_to_world((center_lon, center_lat), (half_extent_lon, half_extent_lat)),
        RENDER_WIDTH,
        RENDER_HEIGHT,
    )
    .await?;

    let out_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/cross-provider-render");
    if std::fs::create_dir_all(&out_dir).is_ok() {
        let out_path = out_dir.join(out_name);
        if let Err(e) = image::save_buffer(
            &out_path,
            &pixels,
            RENDER_WIDTH,
            RENDER_HEIGHT,
            image::ColorType::Rgba8,
        ) {
            println!("cross_provider_render: failed to save {out_name}: {e}");
        } else {
            println!("cross_provider_render: saved {}", out_path.display());
        }
    }

    Some(pixels)
}

#[tokio::test(flavor = "multi_thread")]
async fn gefs_and_hrrr_fields_both_render_through_the_same_forecast_core_call() {
    // --- Fetch one real field from each provider. This is the only
    // per-provider code in this test -- everything after this point is
    // identical for both. ---
    let gefs_field = match fetch_gefs_field().await {
        Some(field) => field,
        None => return,
    };
    let hrrr_field = match fetch_hrrr_field().await {
        Some(field) => field,
        None => return,
    };

    assert!(matches!(
        gefs_field.geometry,
        GridGeometry::RegularLatLon(_)
    ));
    assert!(matches!(
        hrrr_field.geometry,
        GridGeometry::LambertConformal(_)
    ));
    assert_eq!(gefs_field.provider_id, "gefs");
    assert_eq!(hrrr_field.provider_id, "hrrr");
    assert_eq!(gefs_field.variable, ForecastVariable::Temperature2m);
    assert_eq!(hrrr_field.variable, ForecastVariable::Temperature2m);

    // --- Render both through the exact same call. ---
    let Some(gefs_pixels) = render_and_save(&gefs_field, "gefs_temperature_2m.png").await else {
        println!(
            "SKIP render assertions: no GPU adapter available in this environment (expected on \
             CI with no GPU)."
        );
        return;
    };
    let Some(hrrr_pixels) = render_and_save(&hrrr_field, "hrrr_temperature_2m.png").await else {
        println!("SKIP render assertions: no GPU adapter available.");
        return;
    };

    let opaque_fraction = |pixels: &[u8]| {
        let total = pixels.as_chunks::<4>().0.len();
        let opaque = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 255)
            .count();
        opaque as f64 / total as f64
    };
    let gefs_opaque = opaque_fraction(&gefs_pixels);
    let hrrr_opaque = opaque_fraction(&hrrr_pixels);
    println!("GEFS opaque fraction: {gefs_opaque:.4}, HRRR opaque fraction: {hrrr_opaque:.4}");

    // Both renders must have produced a real image: some opaque (in-grid)
    // pixels and (for the framing margin used) some transparent
    // (out-of-grid) ones too -- the render pipeline actually did something
    // geometry-aware for both, not just cleared the target.
    assert!(gefs_opaque > 0.1, "GEFS render looks empty");
    assert!(hrrr_opaque > 0.1, "HRRR render looks empty");
    // Both cameras use the same 5%-margin framing convention (see
    // `render_and_save`), so GEFS's regular-lat-lon grid -- an axis-
    // aligned rectangle in lon/lat space -- should fill nearly the same
    // fraction of the frame regardless of margin rounding (~(1/1.05)^2 =~
    // 0.91 of the frame). HRRR's real Lambert Conformal grid is a curved
    // shape even in the *same* kind of lon/lat camera frame, so it must
    // leave a visibly larger transparent border. This numeric gap is the
    // actual signature of the two providers' genuinely different grid
    // geometries both being handled correctly by one shared function --
    // not a coincidence of matching hand-picked absolute thresholds.
    assert!(
        gefs_opaque > 0.85,
        "GEFS (regular lat/lon) should render as a near-complete rectangle, got {gefs_opaque}"
    );
    assert!(
        gefs_opaque - hrrr_opaque > 0.1,
        "HRRR (Lambert conformal) should show a visibly larger transparent border than GEFS's \
         near-rectangle, from its real curved shape: gefs={gefs_opaque} hrrr={hrrr_opaque}"
    );
}

async fn fetch_gefs_field() -> Option<ForecastGrid> {
    let provider = match provider_gefs::GefsProvider::default_bucket() {
        Ok(p) => p,
        Err(e) => {
            println!("SKIP: failed to build GEFS HTTP client: {e}");
            return None;
        }
    };
    let run = match provider.discover_latest_run(3).await {
        Ok(run) => run,
        Err(e) => {
            println!("SKIP: could not find a published GEFS run: {e}");
            return None;
        }
    };
    let request = FieldRequest::new(ForecastVariable::Temperature2m, 0)
        .with_ensemble(EnsembleStatistic::Control);
    match provider.fetch_field(&run, &request).await {
        Ok(field) => {
            // Crop to CONUS so both renders cover a comparable area (GEFS
            // is global; a full-globe render would dwarf HRRR's CONUS-only
            // domain in this side-by-side proof).
            field.subset(20.0, 55.0, 230.0, 300.0)
        }
        Err(e) => {
            println!("SKIP: failed to fetch/decode GEFS field: {e}");
            None
        }
    }
}

async fn fetch_hrrr_field() -> Option<ForecastGrid> {
    let provider = match provider_hrrr::HrrrProvider::default_bucket() {
        Ok(p) => p,
        Err(e) => {
            println!("SKIP: failed to build HRRR HTTP client: {e}");
            return None;
        }
    };
    let run = match provider.discover_latest_run(2).await {
        Ok(run) => run,
        Err(e) => {
            println!("SKIP: could not find a published HRRR run: {e}");
            return None;
        }
    };
    let request = FieldRequest::new(ForecastVariable::Temperature2m, 0);
    match provider.fetch_field(&run, &request).await {
        Ok(field) => Some(field),
        Err(e) => {
            println!("SKIP: failed to fetch/decode HRRR field: {e}");
            None
        }
    }
}
