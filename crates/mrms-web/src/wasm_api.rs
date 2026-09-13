//! Browser/wasm-bindgen glue for `mrms-web` -- see this crate's `src/lib.rs`
//! module docs for why this glue lives in its own adapter crate and why its
//! naming never borrows forecast-flavored vocabulary from `forecast-web`.
//!
//! # API shape
//!
//! [`MrmsHandle`] wraps one [`mrms::client::MrmsClient`], bound to exactly
//! one [`mrms::keys::MrmsProduct`] selected at construction (mirroring
//! `forecast-web::wasm_api::ProviderHandle`'s "one handle, one concrete
//! backend" shape -- a caller wanting both products constructs two
//! handles). Every fallible method returns `Result<String, JsValue>`/
//! rejects a `Promise` with a `JsValue` error string
//! (`JsValue::from_str(&e.to_string())`), and every method returning
//! structured data returns a JSON string for the caller to `JSON.parse` --
//! same convention `forecast-web`/`weather-alerts`'s `wasm_api` modules
//! already established. The one exception is
//! [`MrmsHandle::render_current_grid`], which returns raw RGBA8 pixel bytes
//! as a `Uint8Array` (wasm-bindgen has no reason to JSON-encode a pixel
//! buffer).
//!
//! `discoverLatestSnapshot`/`fetchSnapshot`/`renderCurrentGrid` are all
//! genuinely async (network fetch, GPU readback) and bridged to a JS
//! `Promise` via [`wasm_bindgen_futures::future_to_promise`]. Because that
//! bridge needs a `'static` future, it cannot borrow `&self` across the
//! `.await` the way a plain async method would -- [`MrmsHandle`] instead
//! holds its mutable state (`last_snapshot`, `last_grid`) behind
//! `Rc<RefCell<..>>`, cloned into the future, with every borrow scoped to
//! end *before* the next `.await` (never held across one) so a caller
//! invoking two methods on the same handle back-to-back, without awaiting
//! the first `Promise`, can never trip a `RefCell` double-borrow panic --
//! identical discipline to `forecast-web::wasm_api::ProviderHandle`.
//!
//! `last_grid` is stored as `Rc<mrms::grid::MrmsGrid>`, not a bare
//! `MrmsGrid` the way `forecast-web` stores its (much smaller) `ForecastGrid`
//! -- see [`MrmsState`]'s own doc comment for why: an MRMS CONUS grid is
//! 7000x3500 = 24.5 million cells, roughly 20x GEFS's/HRRR's own grid size,
//! and `render_current_grid` needs to read it across an `.await` boundary on
//! every single render call (a UI scrubbing a live timeline could call this
//! many times per fetched snapshot). Cloning an `Rc` is O(1); cloning the
//! whole `Vec<MrmsCellValue>` on every render call would not be -- the
//! boundary-crossing cost this project's own GPU-render call path already
//! goes out of its way to keep cheap (see `forecast_core::gpu`'s module doc
//! on `read_rgba8_async`) would otherwise be dwarfed by an avoidable
//! ~200MB CPU-side copy on the Rust side of that same call.
//!
//! # Product independence
//!
//! Each [`MrmsHandle`] owns exactly one concrete product (its own
//! `reqwest::Client`, its own state); nothing here shares state between a
//! reflectivity handle and a precip-rate handle, so one product's failure
//! (network error, decode error, discovery miss) can never affect the
//! other.

use std::cell::RefCell;
use std::rc::Rc;

use forecast_core::grid::GridGeometry;
use mrms::client::MrmsClient;
use mrms::decode::decode_field;
use mrms::grid::MrmsGrid;
use mrms::keys::{MrmsProduct, SnapshotReference};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

/// Runs once when the wasm module is instantiated: installs a panic hook so
/// a Rust panic surfaces as a readable `console.error` message instead of an
/// opaque wasm trap -- same as `forecast-web`/`radar-web`/`weather-alerts`'s
/// `on_wasm_module_init`.
#[wasm_bindgen(start)]
fn on_wasm_module_init() {
    console_error_panic_hook::set_once();
}

/// [`MrmsHandle`]'s actual mutable state, held behind `Rc<RefCell<..>>` --
/// see this module's doc comment for why.
struct MrmsState {
    client: MrmsClient,
    product: MrmsProduct,
    last_snapshot: Option<SnapshotReference>,
    /// `Rc`-wrapped, not a bare `MrmsGrid` -- see this module's doc comment
    /// (an MRMS CONUS grid is ~24.5 million cells; this avoids a deep clone
    /// of the whole value array on every [`MrmsHandle::render_current_grid`]
    /// call).
    last_grid: Option<Rc<MrmsGrid>>,
}

/// A selected MRMS product (`"reflectivity"` or `"precip_rate"`), exposed to
/// JS/wasm. See this module's doc comment for the full API shape.
#[wasm_bindgen]
pub struct MrmsHandle {
    state: Rc<RefCell<MrmsState>>,
}

#[wasm_bindgen]
impl MrmsHandle {
    /// Construct a handle for `product_id` (`"reflectivity"` or
    /// `"precip_rate"`), against the current public `noaa-mrms-pds` bucket
    /// (`MrmsClient::default_bucket`). Returns `Err` for an unrecognized id
    /// or if building the underlying HTTP client fails.
    #[wasm_bindgen(constructor)]
    pub fn new(product_id: &str) -> Result<MrmsHandle, JsValue> {
        let product = parse_product(product_id).ok_or_else(|| {
            JsValue::from_str(&format!(
                "unknown MRMS product id {product_id:?}; expected \"reflectivity\" or \
                     \"precip_rate\""
            ))
        })?;
        let client = MrmsClient::default_bucket().map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(Self {
            state: Rc::new(RefCell::new(MrmsState {
                client,
                product,
                last_snapshot: None,
                last_grid: None,
            })),
        })
    }

    /// This handle's product id, echoed back verbatim (`"reflectivity"` or
    /// `"precip_rate"`) -- lets a UI label a handle without keeping its own
    /// separate copy of which id it was constructed with.
    #[wasm_bindgen(js_name = productId)]
    pub fn product_id(&self) -> String {
        product_id_str(self.state.borrow().product).to_string()
    }

    /// Discover this product's most recently published snapshot, looking
    /// back at most `lookback_days` UTC calendar days. Resolves to a JSON
    /// `{"product","snapshotTime","objectKey"}` string (see
    /// [`snapshot_metadata_json`]); rejects with a descriptive error string
    /// if no published snapshot is found (or a network/parse error
    /// occurred). This is discovery only -- it does not fetch or decode the
    /// object; call [`Self::fetch_snapshot`] next.
    #[wasm_bindgen(js_name = discoverLatestSnapshot)]
    pub fn discover_latest_snapshot(&self, lookback_days: u32) -> js_sys::Promise {
        let state = Rc::clone(&self.state);
        // Cloned out (not borrowed across the `.await` below) -- see this
        // module's doc comment. `MrmsClient` is cheap to clone (an
        // internally reference-counted `reqwest::Client` plus a bucket URL
        // string); `MrmsProduct` is `Copy`.
        let (client, product) = {
            let state = state.borrow();
            (state.client.clone(), state.product)
        };
        future_to_promise(async move {
            let snapshot = client
                .discover_latest_snapshot(product, lookback_days)
                .await
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let json = snapshot_metadata_json(&snapshot);
            state.borrow_mut().last_snapshot = Some(snapshot);
            Ok(JsValue::from_str(&json))
        })
    }

    /// Fetch, gunzip, and decode the snapshot discovered by the most recent
    /// [`Self::discover_latest_snapshot`] call.
    ///
    /// Resolves to this snapshot's decoded metadata as JSON (product, native
    /// unit, observed valid time, and grid dimensions/bounds) --
    /// deliberately *not* including the decoded value array itself (24.5
    /// million cells; JS never needs it directly: call
    /// [`Self::render_current_grid`] to get a rendered image instead).
    /// Rejects if no snapshot has been discovered yet, or the fetch/decode
    /// itself fails.
    #[wasm_bindgen(js_name = fetchSnapshot)]
    pub fn fetch_snapshot(&self) -> js_sys::Promise {
        let state = Rc::clone(&self.state);
        future_to_promise(async move {
            let (client, product, snapshot) = {
                let state = state.borrow();
                let snapshot = state.last_snapshot.ok_or_else(|| {
                    JsValue::from_str(
                        "no snapshot discovered yet; call discoverLatestSnapshot first",
                    )
                })?;
                (state.client.clone(), state.product, snapshot)
            };
            let key = snapshot.object_key();
            let bytes = client
                .fetch_and_decompress(&key)
                .await
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let grid = decode_field(&key, &bytes, product)
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            let json = grid_metadata_json(&grid)?;
            state.borrow_mut().last_grid = Some(Rc::new(grid));
            Ok(JsValue::from_str(&json))
        })
    }

    /// Render the snapshot most recently decoded by [`Self::fetch_snapshot`],
    /// via `mrms::palette::render_mrms_grid` (which itself calls
    /// `forecast_core::gpu::render_forecast_grid` -- the same shared GPU
    /// render/palette path every native harness in this workspace uses, not
    /// reimplemented here). `center_lon`/`center_lat`/`half_extent_lon`/
    /// `half_extent_lat` (decimal degrees) define the orthographic camera
    /// (see `radar_render::camera::clip_to_world`); `render_width`/
    /// `render_height` are the output image's pixel dimensions.
    ///
    /// Resolves to a `Uint8Array` of `render_width * render_height * 4`
    /// RGBA8 bytes, row-major, ready for a JS `ImageData`/canvas bitmap
    /// (the caller already knows `render_width`/`render_height` -- it
    /// supplied them -- so they are not echoed back separately). Rejects if
    /// no snapshot has been fetched yet, or if no GPU adapter is available
    /// in this browser.
    #[wasm_bindgen(js_name = renderCurrentGrid)]
    #[allow(clippy::too_many_arguments)]
    pub fn render_current_grid(
        &self,
        center_lon: f32,
        center_lat: f32,
        half_extent_lon: f32,
        half_extent_lat: f32,
        render_width: u32,
        render_height: u32,
    ) -> js_sys::Promise {
        let state = Rc::clone(&self.state);
        future_to_promise(async move {
            // Clones the `Rc`, not the grid itself -- see this module's doc
            // comment on `MrmsState::last_grid`.
            let grid = {
                let state = state.borrow();
                state.last_grid.clone().ok_or_else(|| {
                    JsValue::from_str("no snapshot fetched yet; call fetchSnapshot first")
                })?
            };

            // `center_lon` arrives in the conventional -180..180 range (the
            // rest of the JS/map ecosystem's convention, and what a caller
            // naturally supplies) but MRMS's own grid geometry -- confirmed
            // in ADR-0014 (La1/Lo1=54.995N/230.005E, i.e. -129.995W) -- is in
            // GRIB's native 0..360 convention, the same convention
            // `forecast-web::wasm_api::render_current_grid` normalizes into
            // at this exact boundary for GEFS/HRRR. Normalize here, not out
            // to JS callers.
            let center_lon_0_360 = if center_lon < 0.0 {
                center_lon + 360.0
            } else {
                center_lon
            };
            let clip_to_world = radar_render::camera::clip_to_world(
                (center_lon_0_360, center_lat),
                (half_extent_lon, half_extent_lat),
            );

            let pixels =
                mrms::palette::render_mrms_grid(&grid, clip_to_world, render_width, render_height)
                    .await;

            match pixels {
                Some(pixels) => Ok(js_sys::Uint8Array::from(pixels.as_slice()).into()),
                None => Err(JsValue::from_str(
                    "no GPU adapter available in this browser",
                )),
            }
        })
    }
}

/// Map a JS-facing product id string to its [`MrmsProduct`]. `None` for
/// anything else -- never panics on caller-supplied input.
fn parse_product(id: &str) -> Option<MrmsProduct> {
    match id {
        "reflectivity" => Some(MrmsProduct::ReflectivityQcComposite),
        "precip_rate" => Some(MrmsProduct::PrecipRate),
        _ => None,
    }
}

/// The JS-facing product id string for `product` -- the exact inverse of
/// [`parse_product`].
fn product_id_str(product: MrmsProduct) -> &'static str {
    match product {
        MrmsProduct::ReflectivityQcComposite => "reflectivity",
        MrmsProduct::PrecipRate => "precip_rate",
    }
}

/// `snapshot`'s own `year`/`month`/.../`second` fields as an ISO-8601-ish
/// UTC string, e.g. `"2026-09-13T05:04:38Z"` -- matches
/// `forecast_core::time::UtcTimestamp::to_iso8601`'s exact format (this
/// crate has no dependency of its own on that type; a [`SnapshotReference`]
/// is not a `UtcTimestamp`, see `mrms::keys`'s module doc for why).
fn snapshot_time_iso8601(snapshot: &SnapshotReference) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        snapshot.year,
        snapshot.month,
        snapshot.day,
        snapshot.hour,
        snapshot.minute,
        snapshot.second
    )
}

#[derive(serde::Serialize)]
struct SnapshotMetadataJson<'a> {
    product: &'static str,
    #[serde(rename = "snapshotTime")]
    snapshot_time: String,
    #[serde(rename = "objectKey")]
    object_key: &'a str,
}

/// A discovered (not yet fetched) snapshot's metadata as JSON. `objectKey`
/// is included as a diagnostic/debugging aid for a Phase 3 caller (e.g. to
/// show which exact NOAA object is about to be fetched) -- it carries no
/// meaning this crate's own API relies on internally, [`MrmsHandle`] always
/// re-derives it from its own stored `last_snapshot`.
fn snapshot_metadata_json(snapshot: &SnapshotReference) -> String {
    let object_key = snapshot.object_key();
    let json = SnapshotMetadataJson {
        product: product_id_str(snapshot.product),
        snapshot_time: snapshot_time_iso8601(snapshot),
        object_key: &object_key,
    };
    serde_json::to_string(&json).expect("SnapshotMetadataJson always encodes")
}

/// This grid's geometry as JSON: dimensions, origin/step, and the bounding
/// box they imply -- all in MRMS's native `[0, 360)` longitude convention
/// (see [`MrmsHandle::render_current_grid`]'s doc comment for why that
/// normalization happens only at the render call, not here). Deliberately
/// minimal, mirroring `forecast-web::wasm_api::GeometryJson`'s own
/// discipline: a future stage that needs more (e.g. a CPU-side point lookup
/// exposed to JS) can add fields without a breaking change, since this is
/// additive-only JSON.
///
/// MRMS's own decode path (`mrms::decode::decode_field`) only ever produces
/// [`GridGeometry::RegularLatLon`] (it rejects any other Grid Definition
/// Template before an [`MrmsGrid`] can exist at all -- see
/// `crates/mrms/src/decode.rs`), so [`GridGeometry::LambertConformal`] is
/// unreachable in practice for this crate's own inputs. This still handles
/// it explicitly with a descriptive `Err` rather than a panic or a silent
/// `unwrap`, per GLOBAL_CONTRACT's "no uncontrolled panics on malformed
/// input" -- the invariant lives one layer away (in `mrms`, not here), and
/// this boundary crate never assumes an invariant it cannot itself enforce.
#[derive(serde::Serialize)]
struct GridGeometryJson {
    width: u32,
    height: u32,
    #[serde(rename = "originLatDeg")]
    origin_lat_deg: f64,
    #[serde(rename = "originLonDeg")]
    origin_lon_deg: f64,
    #[serde(rename = "latStepDeg")]
    lat_step_deg: f64,
    #[serde(rename = "lonStepDeg")]
    lon_step_deg: f64,
    #[serde(rename = "latMinDeg")]
    lat_min_deg: f64,
    #[serde(rename = "latMaxDeg")]
    lat_max_deg: f64,
    #[serde(rename = "lonMinDeg")]
    lon_min_deg: f64,
    #[serde(rename = "lonMaxDeg")]
    lon_max_deg: f64,
}

fn geometry_json(geometry: &GridGeometry) -> Result<GridGeometryJson, JsValue> {
    let GridGeometry::RegularLatLon(g) = geometry else {
        // See this struct's own doc comment: unreachable for any real
        // `MrmsGrid`, but never assumed.
        return Err(JsValue::from_str(
            "internal error: MRMS grid geometry was not RegularLatLon (this crate's decode path \
             only ever produces that variant)",
        ));
    };
    let lat_end = g.lat_for_row(g.height.saturating_sub(1));
    let lon_end = g.lon_for_col(g.width.saturating_sub(1));
    Ok(GridGeometryJson {
        width: g.width,
        height: g.height,
        origin_lat_deg: g.origin_lat_deg,
        origin_lon_deg: g.origin_lon_deg,
        lat_step_deg: g.lat_step_deg,
        lon_step_deg: g.lon_step_deg,
        lat_min_deg: g.origin_lat_deg.min(lat_end),
        lat_max_deg: g.origin_lat_deg.max(lat_end),
        lon_min_deg: g.origin_lon_deg.min(lon_end),
        lon_max_deg: g.origin_lon_deg.max(lon_end),
    })
}

#[derive(serde::Serialize)]
struct MrmsGridMetadataJson {
    product: &'static str,
    unit: &'static str,
    #[serde(rename = "validTime")]
    valid_time: String,
    geometry: GridGeometryJson,
}

/// This decoded snapshot's metadata as JSON -- deliberately excludes
/// `grid.values` (see [`MrmsHandle::fetch_snapshot`]'s own doc comment for
/// why). Never labeled with any run/lead/ensemble field: `validTime` is
/// MRMS's only timestamp (the observed instant), per this crate's own "an
/// observation, never a forecast" discipline.
///
/// `Err` only if `grid.geometry` is not [`GridGeometry::RegularLatLon`] --
/// see [`geometry_json`]'s own doc comment for why that is unreachable for
/// any real [`MrmsGrid`] but never assumed here regardless.
fn grid_metadata_json(grid: &MrmsGrid) -> Result<String, JsValue> {
    let geometry = geometry_json(&grid.geometry)?;
    let json = MrmsGridMetadataJson {
        product: product_id_str(grid.product),
        unit: grid.unit,
        valid_time: grid.valid_time.to_iso8601(),
        geometry,
    };
    Ok(serde_json::to_string(&json).expect("MrmsGridMetadataJson always encodes"))
}
