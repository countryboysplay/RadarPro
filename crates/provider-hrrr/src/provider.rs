//! [`HrrrProvider`]: this crate's `forecast_core::provider::ForecastProvider`
//! implementation.

use crate::client::HrrrClient;
use crate::error::HrrrError;
use crate::keys::{ForecastHour, Product, RunReference};
use forecast_core::grid::{ForecastGrid, NativeVariableMetadata};
use forecast_core::model::{ModelMetadata, ModelRun};
use forecast_core::request::FieldRequest;
use forecast_core::time::UtcTimestamp;
use forecast_core::variable::ForecastVariable;

/// NOAA HRRR, via the anonymous `noaa-hrrr-bdp-pds` S3 bucket.
///
/// `Clone` (cheap: `HrrrClient` itself derives it, wrapping a `reqwest::Client`
/// which is internally reference-counted) -- same rationale as
/// `provider_gefs::GefsProvider`'s own `Clone` derive: lets a caller (e.g.
/// `forecast-web`) clone the provider into a `'static` async boundary it
/// does not control, rather than needing to borrow it.
#[derive(Clone)]
pub struct HrrrProvider {
    client: HrrrClient,
}

impl HrrrProvider {
    pub fn new(client: HrrrClient) -> Self {
        Self { client }
    }

    pub fn default_bucket() -> Result<Self, HrrrError> {
        Ok(Self::new(HrrrClient::default_bucket()?))
    }

    /// Forecast hours of `run` published so far (a run fills in over about an hour).
    pub async fn published_forecast_hours(&self, run: RunReference) -> Result<Vec<u16>, HrrrError> {
        let prefix = crate::keys::run_product_prefix(run, Product::CONUS_SURFACE);
        let keys = self.client.list_objects(&prefix).await?;
        Ok(crate::keys::published_forecast_hours(
            run,
            Product::CONUS_SURFACE,
            &keys,
        ))
    }
}

impl forecast_core::provider::ForecastProvider for HrrrProvider {
    type Run = RunReference;
    type Error = HrrrError;

    fn metadata(&self) -> ModelMetadata {
        ModelMetadata {
            provider_id: "hrrr",
            display_name: "NOAA HRRR",
            is_ensemble: false,
            resolution_description: "~3 km, CONUS",
        }
    }

    fn run_metadata(&self, run: &Self::Run) -> ModelRun {
        ModelRun::new(
            UtcTimestamp::new(
                i64::from(run.year),
                u32::from(run.month),
                u32::from(run.day),
                u32::from(run.run_hour),
                0,
                0,
            ),
            run.to_string(),
        )
    }

    async fn discover_latest_run(&self, lookback_days: u32) -> Result<Self::Run, Self::Error> {
        // HRRR runs hourly, so "lookback_days" is converted to hours here
        // (a UI-facing caller thinks in days for parity with GEFS's own
        // four-daily-runs cadence; this provider's own discovery walks
        // hour by hour).
        self.client
            .find_recent_run(Product::CONUS_SURFACE, ForecastHour(0), lookback_days * 24)
            .await
    }

    async fn fetch_field(
        &self,
        run: &Self::Run,
        request: &FieldRequest,
    ) -> Result<ForecastGrid, Self::Error> {
        if request.ensemble.is_some() {
            return Err(HrrrError::EnsembleNotSupported);
        }

        let forecast_hour = ForecastHour(request.forecast_lead_hours as u16);
        let key = crate::keys::object_key(*run, Product::CONUS_SURFACE, forecast_hour);
        let idx_key = crate::keys::idx_key(*run, Product::CONUS_SURFACE, forecast_hour);

        let specs = crate::decode::input_messages(request.variable).ok_or_else(|| {
            HrrrError::FieldNotFoundInIdx {
                url: idx_key.clone(),
                variable: request.variable.canonical_name().to_string(),
                level: String::new(),
            }
        })?;

        fetch_messages(&self.client, &key, &idx_key, request.variable, specs).await
    }
}

/// Fetch `specs` (one range GET each) from the GRIB2 object `key` and decode
/// them as `variable`; several messages are combined by
/// [`crate::decode::derive`]. Shared by HRRR and RTMA.
pub(crate) async fn fetch_messages(
    client: &HrrrClient,
    key: &str,
    idx_key: &str,
    variable: ForecastVariable,
    specs: Vec<crate::decode::MessageSpec>,
) -> Result<ForecastGrid, HrrrError> {
    let idx_text = client.fetch_idx_text(idx_key).await?;
    let entries = crate::idx::parse_idx(idx_key, &idx_text)?;
    // A derived field (shear, STP) needs several messages; each is one range GET.
    let mut grids = Vec::with_capacity(specs.len());
    for spec in specs {
        let (idx_variable, idx_level, ..) = spec;
        let position = entries
            .iter()
            .position(|e| e.variable == idx_variable && e.level == idx_level)
            .ok_or_else(|| HrrrError::FieldNotFoundInIdx {
                url: idx_key.to_string(),
                variable: idx_variable.to_string(),
                level: idx_level.to_string(),
            })?;
        let content_length = if position + 1 == entries.len() {
            Some(client.content_length(key).await?)
        } else {
            None
        };
        let (start, end) = crate::idx::byte_range(key, &entries, position, content_length)?;
        let bytes = client.fetch_byte_range(key, start, end).await?;
        grids.push(crate::decode::decode_message(key, &bytes, variable, spec)?);
    }
    if grids.len() == 1 {
        return Ok(grids.remove(0));
    }
    let names = |f: fn(&ForecastGrid) -> &str| grids.iter().map(f).collect::<Vec<_>>().join(",");
    let native = NativeVariableMetadata {
        provider_variable_name: names(|g| &g.native.provider_variable_name),
        provider_level_name: names(|g| &g.native.provider_level_name),
        native_unit: crate::decode::derived_unit(variable),
    };
    let inputs: Vec<Vec<f32>> = grids
        .iter_mut()
        .map(|g| std::mem::take(&mut g.values))
        .collect();
    let mut grid = grids.remove(0);
    grid.values = crate::decode::derive(variable, &inputs);
    grid.unit = native.native_unit;
    grid.native = native;
    Ok(grid)
}
