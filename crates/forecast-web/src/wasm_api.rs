//! Browser/wasm-bindgen glue for `forecast-web` -- see this crate's
//! `src/lib.rs` module docs for why this glue lives in its own adapter
//! crate (unlike `weather-alerts`) and why it dispatches over an enum
//! rather than a trait object.
//!
//! # API shape
//!
//! [`ProviderHandle`] wraps whichever concrete provider (`provider-gefs`'s
//! [`GefsProvider`] or `provider-hrrr`'s [`HrrrProvider`]) was selected at
//! construction. Every fallible method returns `Result<String, JsValue>`/
//! rejects a `Promise` with a `JsValue` error string
//! (`JsValue::from_str(&e.to_string())`), and every method returning
//! structured data returns a JSON string for the caller to `JSON.parse` --
//! same conventions `weather-alerts::wasm_api` already established. The
//! one exception is [`ProviderHandle::render_current_grid`], which returns
//! raw RGBA8 pixel bytes as a `Uint8Array` (wasm-bindgen has no reason to
//! JSON-encode a pixel buffer).
//!
//! `discoverLatestRun`/`fetchField`/`renderCurrentGrid` are all genuinely
//! async (network fetch, GPU readback) and bridged to a JS `Promise` via
//! [`wasm_bindgen_futures::future_to_promise`]. Because that bridge needs a
//! `'static` future, it cannot borrow `&self` across the `.await` the way a
//! plain async method would -- [`ProviderHandle`] instead holds its
//! mutable state (`last_run`, `last_grid`) behind `Rc<RefCell<..>>`,
//! cloned into the future, with every borrow scoped to end *before* the
//! next `.await` (never held across one) so a caller invoking two methods
//! on the same handle back-to-back, without awaiting the first `Promise`,
//! can never trip a `RefCell` double-borrow panic.
//!
//! # Provider independence
//!
//! Each [`ProviderHandle`] owns exactly one concrete provider (its own
//! `reqwest::Client`, its own state); nothing here shares state between a
//! GEFS handle and an HRRR handle, so one provider's failure (network
//! error, decode error, discovery miss) can never affect the other -- the
//! Global Contract's "GEFS/HRRR must each fail independently" requirement
//! falls out of this crate's own construction, not an extra check.

use std::cell::RefCell;
use std::rc::Rc;

use forecast_core::ensemble::EnsembleStatistic;
use forecast_core::grid::{ForecastGrid, GridGeometry};
use forecast_core::model::ModelMetadata;
use forecast_core::provider::ForecastProvider;
use forecast_core::request::FieldRequest;
use forecast_core::variable::ForecastVariable;
use provider_gefs::GefsProvider;
use provider_hrrr::HrrrProvider;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

/// Runs once when the wasm module is instantiated: installs a panic hook
/// so a Rust panic surfaces as a readable `console.error` message instead
/// of an opaque wasm trap -- same as `radar-web`/`weather-alerts`'s
/// `on_wasm_module_init`.
#[wasm_bindgen(start)]
fn on_wasm_module_init() {
    console_error_panic_hook::set_once();
}

/// The concrete provider a [`ProviderHandle`] was constructed with. Not a
/// `Box<dyn ForecastProvider>` -- see this module's parent doc comment.
///
/// `Clone` is cheap for both variants: each provider wraps only a
/// `reqwest::Client` (internally reference-counted) plus a bucket URL
/// string.
#[derive(Clone)]
enum ProviderImpl {
    Gefs(GefsProvider),
    Hrrr(HrrrProvider),
}

/// The run a [`ProviderHandle`] last discovered, tagged by which concrete
/// provider produced it -- `provider_gefs::keys::RunReference` and
/// `provider_hrrr::keys::RunReference` are different types (GEFS's own
/// four-daily-run-hour identity vs. HRRR's plain hourly one), so this
/// crate cannot store just one shared run type; both are `Copy`, so this
/// enum is too.
#[derive(Clone, Copy)]
enum RunImpl {
    Gefs(provider_gefs::keys::RunReference),
    Hrrr(provider_hrrr::keys::RunReference),
}

/// [`ProviderHandle`]'s actual mutable state, held behind `Rc<RefCell<..>>`
/// -- see this module's doc comment for why.
struct ProviderState {
    provider: ProviderImpl,
    last_run: Option<RunImpl>,
    last_grid: Option<ForecastGrid>,
}

/// A selected forecast provider (`"gefs"` or `"hrrr"`), exposed to
/// JS/wasm. See this module's doc comment for the full API shape.
#[wasm_bindgen]
pub struct ProviderHandle {
    state: Rc<RefCell<ProviderState>>,
}

#[wasm_bindgen]
impl ProviderHandle {
    /// Construct a handle for `provider_id` (`"gefs"` or `"hrrr"`),
    /// against each provider's current public NOAA S3 bucket
    /// (`GefsProvider::default_bucket`/`HrrrProvider::default_bucket`).
    /// Returns `Err` for an unrecognized id or if building the underlying
    /// HTTP client fails.
    #[wasm_bindgen(constructor)]
    pub fn new(provider_id: &str) -> Result<ProviderHandle, JsValue> {
        let provider = match provider_id {
            "gefs" => ProviderImpl::Gefs(
                GefsProvider::default_bucket().map_err(|e| JsValue::from_str(&e.to_string()))?,
            ),
            "hrrr" => ProviderImpl::Hrrr(
                HrrrProvider::default_bucket().map_err(|e| JsValue::from_str(&e.to_string()))?,
            ),
            other => {
                return Err(JsValue::from_str(&format!(
                    "unknown provider id {other:?}; expected \"gefs\" or \"hrrr\""
                )))
            }
        };
        Ok(Self {
            state: Rc::new(RefCell::new(ProviderState {
                provider,
                last_run: None,
                last_grid: None,
            })),
        })
    }

    /// This provider's static identity/description
    /// (`{"providerId","displayName","isEnsemble","resolutionDescription"}`)
    /// -- lets a UI build its provider switcher (and decide whether to show
    /// an ensemble-statistic picker at all) with no network call.
    #[wasm_bindgen(js_name = metadata)]
    pub fn metadata(&self) -> String {
        let metadata = match &self.state.borrow().provider {
            ProviderImpl::Gefs(p) => p.metadata(),
            ProviderImpl::Hrrr(p) => p.metadata(),
        };
        model_metadata_json(metadata)
    }

    /// Discover this provider's most recently published run, looking back
    /// at most `lookback_days` UTC calendar days. Resolves to a JSON
    /// `{"providerId","initTime","label"}` string (see [`run_metadata_json`]);
    /// rejects with a descriptive error string if no published run is
    /// found (or a network/parse error occurred).
    #[wasm_bindgen(js_name = discoverLatestRun)]
    pub fn discover_latest_run(&self, lookback_days: u32) -> js_sys::Promise {
        let state = Rc::clone(&self.state);
        // Cloned out (not borrowed across the `.await` below) -- see this
        // module's doc comment.
        let provider = state.borrow().provider.clone();
        future_to_promise(async move {
            let json = match provider {
                ProviderImpl::Gefs(p) => {
                    let run = p
                        .discover_latest_run(lookback_days)
                        .await
                        .map_err(|e| JsValue::from_str(&e.to_string()))?;
                    let model_run = p.run_metadata(&run);
                    let json = run_metadata_json("gefs", &model_run);
                    state.borrow_mut().last_run = Some(RunImpl::Gefs(run));
                    json
                }
                ProviderImpl::Hrrr(p) => {
                    let run = p
                        .discover_latest_run(lookback_days)
                        .await
                        .map_err(|e| JsValue::from_str(&e.to_string()))?;
                    let model_run = p.run_metadata(&run);
                    let json = run_metadata_json("hrrr", &model_run);
                    state.borrow_mut().last_run = Some(RunImpl::Hrrr(run));
                    json
                }
            };
            Ok(JsValue::from_str(&json))
        })
    }

    /// Fetch and decode one field for the run discovered by the most
    /// recent [`Self::discover_latest_run`] call.
    ///
    /// `variable` is a canonical variable name (e.g. `"temperature_2m"`,
    /// see `forecast_core::variable::ForecastVariable::canonical_name`).
    /// `ensemble` (only meaningful for an ensemble provider like GEFS) is
    /// one of `"control"`, `"mean"`, `"member:<n>"`, or `"percentile:<n>"`
    /// -- `null`/absent for a deterministic provider (HRRR), or to request
    /// no specific ensemble statistic.
    ///
    /// Resolves to this field's metadata as JSON (variable/native identity,
    /// provider id, unit, run/lead/valid time, ensemble identity, and grid
    /// geometry) -- deliberately *not* including the decoded value array
    /// itself (large, and JS never needs it directly: call
    /// [`Self::render_current_grid`] to get a rendered image instead).
    /// Rejects if no run has been discovered yet, `variable`/`ensemble` is
    /// unrecognized, or the fetch/decode itself fails.
    #[wasm_bindgen(js_name = fetchField)]
    pub fn fetch_field(
        &self,
        variable: &str,
        forecast_lead_hours: u32,
        ensemble: Option<String>,
    ) -> js_sys::Promise {
        let state = Rc::clone(&self.state);

        let variable = match parse_variable(variable) {
            Some(v) => v,
            None => {
                let message = format!("unknown forecast variable {variable:?}");
                return future_to_promise(async move { Err(JsValue::from_str(&message)) });
            }
        };
        let ensemble = match ensemble.as_deref().map(parse_ensemble).transpose() {
            Ok(e) => e,
            Err(message) => {
                return future_to_promise(async move { Err(JsValue::from_str(&message)) })
            }
        };

        future_to_promise(async move {
            let (provider, run) = {
                let state = state.borrow();
                let run = state.last_run.ok_or_else(|| {
                    JsValue::from_str("no run discovered yet; call discoverLatestRun first")
                })?;
                (state.provider.clone(), run)
            };
            let request = FieldRequest {
                variable,
                forecast_lead_hours,
                ensemble,
            };
            let grid = match (provider, run) {
                (ProviderImpl::Gefs(p), RunImpl::Gefs(run)) => p
                    .fetch_field(&run, &request)
                    .await
                    .map_err(|e| JsValue::from_str(&e.to_string()))?,
                (ProviderImpl::Hrrr(p), RunImpl::Hrrr(run)) => p
                    .fetch_field(&run, &request)
                    .await
                    .map_err(|e| JsValue::from_str(&e.to_string()))?,
                // Unreachable in practice: `last_run` is only ever set by
                // this same handle's own `discover_latest_run`, which
                // always tags it with this handle's own provider kind.
                // Still handled explicitly (never a panic) per the Global
                // Contract's "no uncontrolled panics" rule.
                _ => {
                    return Err(JsValue::from_str(
                        "internal error: discovered run does not match this handle's provider",
                    ))
                }
            };
            let json = grid_metadata_json(&grid);
            state.borrow_mut().last_grid = Some(grid);
            Ok(JsValue::from_str(&json))
        })
    }

    /// Render the field most recently fetched by [`Self::fetch_field`],
    /// via `forecast_core::gpu::render_forecast_grid` (the same GPU
    /// render/palette path every native harness uses -- not reimplemented
    /// here). `center_lon`/`center_lat`/`half_extent_lon`/`half_extent_lat`
    /// (decimal degrees) define the orthographic camera (see
    /// `radar_render::camera::clip_to_world`); `render_width`/
    /// `render_height` are the output image's pixel dimensions.
    ///
    /// Resolves to a `Uint8Array` of `render_width * render_height * 4`
    /// RGBA8 bytes, row-major, ready for a JS `ImageData`/canvas bitmap
    /// (the caller already knows `render_width`/`render_height` -- it
    /// supplied them -- so they are not echoed back separately). Rejects
    /// if no field has been fetched yet, or if no GPU adapter is available
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
            let grid = {
                let state = state.borrow();
                state.last_grid.clone().ok_or_else(|| {
                    JsValue::from_str("no field fetched yet; call fetchField first")
                })?
            };

            let display_values = forecast_core::render::to_display_celsius(&grid);
            let palette_lut = forecast_core::render::build_default_palette_lut();
            // `center_lon` arrives in the conventional -180..180 range (the
            // rest of the JS/map ecosystem's convention, and what a caller
            // naturally supplies) but GEFS/HRRR's own grid geometry -- and
            // every native harness's known-good, PNG-verified camera -- is
            // in GRIB's native 0..360 convention (see e.g.
            // provider-hrrr/src/bin/harness.rs's `CONUS_LON_MIN: f64 = 230.0
            // // -130 deg E`). Normalize here, at the JS/Rust boundary,
            // rather than push the 0..360 convention out to JS callers.
            let center_lon_0_360 = if center_lon < 0.0 {
                center_lon + 360.0
            } else {
                center_lon
            };
            let clip_to_world = radar_render::camera::clip_to_world(
                (center_lon_0_360, center_lat),
                (half_extent_lon, half_extent_lat),
            );

            let pixels = forecast_core::gpu::render_forecast_grid(
                &grid,
                &display_values,
                &palette_lut,
                forecast_core::render::DEFAULT_MIN_CELSIUS,
                forecast_core::render::DEFAULT_MAX_CELSIUS,
                clip_to_world,
                render_width,
                render_height,
            )
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

/// Match a canonical variable name (e.g. `"temperature_2m"`) back to its
/// [`ForecastVariable`]. `None` for anything else -- never panics on
/// caller-supplied input.
fn parse_variable(name: &str) -> Option<ForecastVariable> {
    ForecastVariable::ALL
        .into_iter()
        .find(|v| v.canonical_name() == name)
}

/// Parse an ensemble spec string (`"control"`, `"mean"`, `"member:<n>"`,
/// `"percentile:<n>"`) into an [`EnsembleStatistic`]. Returns a descriptive
/// `Err` message for anything else -- never panics on caller-supplied
/// input.
fn parse_ensemble(spec: &str) -> Result<EnsembleStatistic, String> {
    if spec == "control" {
        return Ok(EnsembleStatistic::Control);
    }
    if spec == "mean" {
        return Ok(EnsembleStatistic::Mean);
    }
    if let Some(n) = spec.strip_prefix("member:") {
        let n: u8 = n
            .parse()
            .map_err(|_| format!("invalid ensemble member number in {spec:?}"))?;
        return Ok(EnsembleStatistic::Member(n));
    }
    if let Some(p) = spec.strip_prefix("percentile:") {
        let p: u8 = p
            .parse()
            .map_err(|_| format!("invalid ensemble percentile in {spec:?}"))?;
        return Ok(EnsembleStatistic::Percentile(p));
    }
    Err(format!(
        "unrecognized ensemble spec {spec:?}; expected \"control\", \"mean\", \"member:<n>\", \
         or \"percentile:<n>\""
    ))
}

#[derive(serde::Serialize)]
struct ModelMetadataJson {
    #[serde(rename = "providerId")]
    provider_id: &'static str,
    #[serde(rename = "displayName")]
    display_name: &'static str,
    #[serde(rename = "isEnsemble")]
    is_ensemble: bool,
    #[serde(rename = "resolutionDescription")]
    resolution_description: &'static str,
}

fn model_metadata_json(metadata: ModelMetadata) -> String {
    let json = ModelMetadataJson {
        provider_id: metadata.provider_id,
        display_name: metadata.display_name,
        is_ensemble: metadata.is_ensemble,
        resolution_description: metadata.resolution_description,
    };
    serde_json::to_string(&json).expect("ModelMetadataJson always encodes")
}

#[derive(serde::Serialize)]
struct RunMetadataJson<'a> {
    #[serde(rename = "providerId")]
    provider_id: &'a str,
    #[serde(rename = "initTime")]
    init_time: String,
    label: &'a str,
}

fn run_metadata_json(provider_id: &str, run: &forecast_core::model::ModelRun) -> String {
    let json = RunMetadataJson {
        provider_id,
        init_time: run.init_time.to_iso8601(),
        label: &run.label,
    };
    serde_json::to_string(&json).expect("RunMetadataJson always encodes")
}

#[derive(serde::Serialize)]
struct NativeVariableMetadataJson<'a> {
    #[serde(rename = "providerVariableName")]
    provider_variable_name: &'a str,
    #[serde(rename = "providerLevelName")]
    provider_level_name: &'a str,
    #[serde(rename = "nativeUnit")]
    native_unit: &'static str,
}

/// The ensemble identity half of [`ForecastGridMetadataJson`] --
/// `#[serde(tag = "kind")]` so JS sees `{"kind":"control"}`,
/// `{"kind":"member","value":3}`, `{"kind":"mean"}`, or
/// `{"kind":"percentile","value":50}`; `None` on the outer
/// `ForecastGrid::ensemble` serializes as JSON `null` (a deterministic
/// provider, e.g. HRRR -- see `forecast_core::ensemble`'s module doc for
/// why absence, not a variant of its own, represents that).
#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum EnsembleJson {
    Control,
    Member { value: u8 },
    Mean,
    Percentile { value: u8 },
}

impl From<EnsembleStatistic> for EnsembleJson {
    fn from(statistic: EnsembleStatistic) -> Self {
        match statistic {
            EnsembleStatistic::Control => EnsembleJson::Control,
            EnsembleStatistic::Member(n) => EnsembleJson::Member { value: n },
            EnsembleStatistic::Mean => EnsembleJson::Mean,
            EnsembleStatistic::Percentile(p) => EnsembleJson::Percentile { value: p },
        }
    }
}

/// This field's grid geometry, tagged by projection kind -- mirrors
/// [`GridGeometry`]'s own two variants. Deliberately minimal (width/height
/// plus each variant's own placement fields): the render call
/// ([`ProviderHandle::render_current_grid`]) already handles reprojection
/// internally, so JS never needs the full Lambert projection parameters
/// (`forecast_core::projection::LccProjection`) to use this API -- a
/// future stage that needs those for some other purpose (e.g. a CPU-side
/// point-forecast lookup exposed to JS) can add them without a breaking
/// change here, since this is additive-only JSON.
#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum GeometryJson {
    RegularLatLon {
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
    },
    LambertConformal {
        width: u32,
        height: u32,
        #[serde(rename = "originXM")]
        origin_x_m: f64,
        #[serde(rename = "originYM")]
        origin_y_m: f64,
        #[serde(rename = "dxM")]
        dx_m: f64,
        #[serde(rename = "dyM")]
        dy_m: f64,
    },
}

impl From<&GridGeometry> for GeometryJson {
    fn from(geometry: &GridGeometry) -> Self {
        match geometry {
            GridGeometry::RegularLatLon(g) => GeometryJson::RegularLatLon {
                width: g.width,
                height: g.height,
                origin_lat_deg: g.origin_lat_deg,
                origin_lon_deg: g.origin_lon_deg,
                lat_step_deg: g.lat_step_deg,
                lon_step_deg: g.lon_step_deg,
            },
            GridGeometry::LambertConformal(g) => GeometryJson::LambertConformal {
                width: g.width,
                height: g.height,
                origin_x_m: g.origin_x_m,
                origin_y_m: g.origin_y_m,
                dx_m: g.dx_m,
                dy_m: g.dy_m,
            },
        }
    }
}

#[derive(serde::Serialize)]
struct ForecastGridMetadataJson<'a> {
    variable: &'static str,
    native: NativeVariableMetadataJson<'a>,
    #[serde(rename = "providerId")]
    provider_id: &'static str,
    unit: &'static str,
    #[serde(rename = "runTime")]
    run_time: String,
    #[serde(rename = "forecastLeadHours")]
    forecast_lead_hours: u32,
    #[serde(rename = "validTime")]
    valid_time: String,
    ensemble: Option<EnsembleJson>,
    geometry: GeometryJson,
}

/// This decoded field's metadata as JSON -- deliberately excludes
/// `grid.values` (see [`ProviderHandle::fetch_field`]'s own doc comment
/// for why).
fn grid_metadata_json(grid: &ForecastGrid) -> String {
    let json = ForecastGridMetadataJson {
        variable: grid.variable.canonical_name(),
        native: NativeVariableMetadataJson {
            provider_variable_name: &grid.native.provider_variable_name,
            provider_level_name: &grid.native.provider_level_name,
            native_unit: grid.native.native_unit,
        },
        provider_id: grid.provider_id,
        unit: grid.unit,
        run_time: grid.run_time.to_iso8601(),
        forecast_lead_hours: grid.forecast_lead_hours,
        valid_time: grid.valid_time.to_iso8601(),
        ensemble: grid.ensemble.map(EnsembleJson::from),
        geometry: GeometryJson::from(&grid.geometry),
    };
    serde_json::to_string(&json).expect("ForecastGridMetadataJson always encodes")
}
