//! [`GefsProvider`]: this crate's `forecast_core::provider::ForecastProvider`
//! implementation.

use crate::client::GefsClient;
use crate::error::GefsError;
use crate::keys::{ForecastHour, MemberKey, ProductGroup, RunReference};
use forecast_core::ensemble::EnsembleStatistic;
use forecast_core::grid::ForecastGrid;
use forecast_core::model::{ModelMetadata, ModelRun};
use forecast_core::request::FieldRequest;
use forecast_core::time::UtcTimestamp;

/// NOAA GEFS, via the anonymous `noaa-gefs-pds` S3 bucket.
///
/// `Clone` (cheap: `GefsClient` itself derives it, wrapping a `reqwest::Client`
/// which is internally reference-counted) -- needed so a caller holding a
/// `GefsProvider` across an async boundary it does not control (e.g.
/// `forecast-web`'s `wasm_bindgen_futures::future_to_promise`, which needs
/// a `'static` future and so cannot capture a borrow of a caller-owned
/// value) can clone the provider into that future instead.
#[derive(Clone)]
pub struct GefsProvider {
    client: GefsClient,
}

impl GefsProvider {
    pub fn new(client: GefsClient) -> Self {
        Self { client }
    }

    pub fn default_bucket() -> Result<Self, GefsError> {
        Ok(Self::new(GefsClient::default_bucket()?))
    }
}

/// Map a requested [`EnsembleStatistic`] onto GEFS's own member-file
/// identity. GEFS does not publish named percentiles (S08 stage file: "do
/// not invent an interpolation scheme to fake them"), so
/// `EnsembleStatistic::Percentile(_)` is rejected, not approximated.
fn member_key_for(statistic: EnsembleStatistic) -> Result<MemberKey, GefsError> {
    match statistic {
        EnsembleStatistic::Control => Ok(MemberKey::Control),
        EnsembleStatistic::Member(n) => Ok(MemberKey::Perturbed(n)),
        EnsembleStatistic::Mean => Ok(MemberKey::Mean),
        EnsembleStatistic::Percentile(p) => Err(GefsError::UnsupportedEnsembleStatistic {
            requested: format!("Percentile({p})"),
        }),
    }
}

impl forecast_core::provider::ForecastProvider for GefsProvider {
    type Run = RunReference;
    type Error = GefsError;

    fn metadata(&self) -> ModelMetadata {
        ModelMetadata {
            provider_id: "gefs",
            display_name: "NOAA GEFS",
            is_ensemble: true,
            resolution_description: "0.25 degree, global",
        }
    }

    fn run_metadata(&self, run: &Self::Run) -> ModelRun {
        ModelRun::new(
            UtcTimestamp::new(
                i64::from(run.year),
                u32::from(run.month),
                u32::from(run.day),
                u32::from(run.run_hour.hour()),
                0,
                0,
            ),
            run.to_string(),
        )
    }

    async fn discover_latest_run(&self, lookback_days: u32) -> Result<Self::Run, Self::Error> {
        self.client
            .find_recent_run(ProductGroup::PGRB2S_P25, ForecastHour(0), lookback_days)
            .await
    }

    async fn fetch_field(
        &self,
        run: &Self::Run,
        request: &FieldRequest,
    ) -> Result<ForecastGrid, Self::Error> {
        let statistic = request
            .ensemble
            .ok_or(GefsError::EnsembleStatisticRequired)?;
        let member = member_key_for(statistic)?;

        let forecast_hour = ForecastHour(request.forecast_lead_hours as u16);
        let key = crate::keys::object_key(*run, member, ProductGroup::PGRB2S_P25, forecast_hour);
        let idx_key = crate::keys::idx_key(*run, member, ProductGroup::PGRB2S_P25, forecast_hour);

        let (idx_variable, idx_level, _unit, _category, _number) =
            crate::decode::idx_names(request.variable).ok_or_else(|| {
                GefsError::FieldNotFoundInIdx {
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
            .ok_or_else(|| GefsError::FieldNotFoundInIdx {
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
