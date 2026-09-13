//! [`HrrrProvider`]: this crate's `forecast_core::provider::ForecastProvider`
//! implementation.

use crate::client::HrrrClient;
use crate::error::HrrrError;
use crate::keys::{ForecastHour, Product, RunReference};
use forecast_core::grid::ForecastGrid;
use forecast_core::model::{ModelMetadata, ModelRun};
use forecast_core::request::FieldRequest;
use forecast_core::time::UtcTimestamp;

/// NOAA HRRR, via the anonymous `noaa-hrrr-bdp-pds` S3 bucket.
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

        let (idx_variable, idx_level) =
            crate::decode::idx_names(request.variable).ok_or_else(|| {
                HrrrError::FieldNotFoundInIdx {
                    url: idx_key.clone(),
                    variable: request.variable.canonical_name().to_string(),
                    level: String::new(),
                }
            })?;

        let idx_text = self.client.fetch_idx_text(&idx_key).await?;
        let entries = crate::idx::parse_idx(&idx_key, &idx_text)?;
        let position = entries
            .iter()
            .position(|e| e.variable == idx_variable && e.level == idx_level)
            .ok_or_else(|| HrrrError::FieldNotFoundInIdx {
                url: idx_key.clone(),
                variable: idx_variable.to_string(),
                level: idx_level.to_string(),
            })?;

        let content_length = if position + 1 == entries.len() {
            Some(self.client.content_length(&key).await?)
        } else {
            None
        };
        let (start, end) = crate::idx::byte_range(&key, &entries, position, content_length)?;

        let bytes = self.client.fetch_byte_range(&key, start, end).await?;
        crate::decode::decode_field(&key, &bytes, request.variable)
    }
}
