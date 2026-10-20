//! Phase 5.3: NOAA RTMA (Real-Time Mesoscale Analysis), the hourly 2.5 km
//! CONUS surface analysis. It lives in this crate because it is the same
//! NCEP GRIB2 family as HRRR: a Lambert Conformal grid (template 3.30, but
//! earth shape 1, radius 6,371,200 m), an `.idx` sidecar per file, and the
//! same S3 client and decoder. An analysis has one valid time and no
//! forecast hours, so only lead 0 exists.

use crate::client::HrrrClient;
use crate::decode::MessageSpec;
use crate::error::HrrrError;
use crate::keys::RunReference;
use forecast_core::grid::ForecastGrid;
use forecast_core::model::{ModelMetadata, ModelRun};
use forecast_core::request::FieldRequest;
use forecast_core::time::UtcTimestamp;
use forecast_core::variable::ForecastVariable;

pub const RTMA_BUCKET_URL: &str = "https://noaa-rtma-pds.s3.amazonaws.com";

/// `rtma2p5.20261020/rtma2p5.t19z.2dvaranl_ndfd.grb2_wexp` (the analysis;
/// `2dvarges` is the first guess and `2dvarerr` the error estimate).
pub fn object_key(run: RunReference) -> String {
    format!(
        "rtma2p5.{}/rtma2p5.t{:02}z.2dvaranl_ndfd.grb2_wexp",
        run.date_component(),
        run.run_hour
    )
}

/// The analysis hour of `key`, if it is an analysis `.idx` sidecar (written
/// a few seconds after its GRIB2 file, so its presence means both are ready).
fn idx_hour(key: &str) -> Option<u8> {
    let name = key.rsplit('/').next()?;
    let hour = name
        .strip_prefix("rtma2p5.t")?
        .strip_suffix("z.2dvaranl_ndfd.grb2_wexp.idx")?;
    hour.parse().ok().filter(|h| *h < 24)
}

/// Every message is published directly; nothing is derived.
/// (idx VARNAME, idx LEVEL, native unit, category, number), checked against
/// `rtma2p5.t19z.2dvaranl_ndfd.grb2_wexp` on 2026-10-20.
pub fn idx_names(variable: ForecastVariable) -> Option<MessageSpec> {
    match variable {
        ForecastVariable::Temperature2m => Some(("TMP", "2 m above ground", "K", 0, 0)),
        ForecastVariable::Dewpoint2m => Some(("DPT", "2 m above ground", "K", 0, 6)),
        ForecastVariable::WindU10m => Some(("UGRD", "10 m above ground", "m/s", 2, 2)),
        ForecastVariable::WindV10m => Some(("VGRD", "10 m above ground", "m/s", 2, 3)),
        ForecastVariable::WindSpeed10m => Some(("WIND", "10 m above ground", "m/s", 2, 1)),
        // Unlike HRRR's `GUST:surface`.
        ForecastVariable::WindGust => Some(("GUST", "10 m above ground", "m/s", 2, 22)),
        ForecastVariable::CloudCover => Some((
            "TCDC",
            "entire atmosphere (considered as a single layer)",
            "%",
            6,
            1,
        )),
        _ => None,
    }
}

#[derive(Clone)]
pub struct RtmaProvider {
    client: HrrrClient,
}

impl RtmaProvider {
    pub fn default_bucket() -> Result<Self, HrrrError> {
        Ok(Self {
            client: HrrrClient::new(RTMA_BUCKET_URL)?,
        })
    }
}

impl forecast_core::provider::ForecastProvider for RtmaProvider {
    type Run = RunReference;
    type Error = HrrrError;

    fn metadata(&self) -> ModelMetadata {
        ModelMetadata {
            provider_id: "rtma",
            display_name: "NOAA RTMA",
            is_ensemble: false,
            resolution_description: "2.5 km, CONUS surface analysis",
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

    /// The newest hour with a published analysis, today first, then each
    /// earlier day up to `lookback_days` (one listing per day).
    async fn discover_latest_run(&self, lookback_days: u32) -> Result<Self::Run, Self::Error> {
        let (year, month, day) = forecast_core::time::today_utc_date();
        for back in 0..=lookback_days {
            let (y, m, d) = forecast_core::time::civil_date_minus_days(year, month, day, back);
            let date = RunReference::new(y, m, d, 0);
            let keys = self
                .client
                .list_objects(&format!("rtma2p5.{}/", date.date_component()))
                .await?;
            if let Some(hour) = keys.iter().filter_map(|k| idx_hour(k)).max() {
                return Ok(RunReference::new(y, m, d, hour));
            }
        }
        Err(HrrrError::NoPublishedRunFound {
            product: "rtma2p5 2dvaranl".to_string(),
            lookback_hours: lookback_days * 24,
        })
    }

    async fn fetch_field(
        &self,
        run: &Self::Run,
        request: &FieldRequest,
    ) -> Result<ForecastGrid, Self::Error> {
        if request.ensemble.is_some() {
            return Err(HrrrError::EnsembleNotSupported);
        }
        let key = object_key(*run);
        let idx_key = format!("{key}.idx");
        let spec = idx_names(request.variable)
            .filter(|_| request.forecast_lead_hours == 0)
            .ok_or_else(|| HrrrError::FieldNotFoundInIdx {
                url: idx_key.clone(),
                variable: request.variable.canonical_name().to_string(),
                level: format!("analysis (lead {} h)", request.forecast_lead_hours),
            })?;
        crate::provider::fetch_messages(&self.client, &key, &idx_key, request.variable, vec![spec])
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_key_matches_the_live_bucket() {
        assert_eq!(
            object_key(RunReference::new(2026, 10, 20, 9)),
            "rtma2p5.20261020/rtma2p5.t09z.2dvaranl_ndfd.grb2_wexp"
        );
    }

    #[test]
    fn only_analysis_idx_keys_name_an_hour() {
        assert_eq!(
            idx_hour("rtma2p5.20261020/rtma2p5.t19z.2dvaranl_ndfd.grb2_wexp.idx"),
            Some(19)
        );
        assert_eq!(
            idx_hour("rtma2p5.20261020/rtma2p5.t19z.2dvaranl_ndfd.grb2_wexp"),
            None
        );
        assert_eq!(
            idx_hour("rtma2p5.20261020/rtma2p5.t19z.2dvarges_ndfd.grb2_wexp.idx"),
            None
        );
        assert_eq!(
            idx_hour("rtma2p5.20261020/rtma2p5.t99z.2dvaranl_ndfd.grb2_wexp.idx"),
            None
        );
    }

    #[test]
    fn decodes_real_rtma_wind_speed() {
        // Live capture: `WIND:10 m above ground` of rtma2p5.t19z.2dvaranl_ndfd (2026-10-20).
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/provider-hrrr/rtma_20261020_t19z_wind10m.grib2");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("missing fixture {}: {e}", path.display()));
        let spec = idx_names(ForecastVariable::WindSpeed10m).unwrap();
        let grid = crate::decode::decode_message("u", &bytes, ForecastVariable::WindSpeed10m, spec)
            .unwrap();
        assert_eq!(grid.values.len(), 2345 * 1597);
        assert_eq!(grid.forecast_lead_hours, 0);
        let finite: Vec<f32> = grid
            .values
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .collect();
        let max = finite.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            finite.iter().all(|&v| v >= 0.0),
            "speeds are never negative"
        );
        assert!((5.0..60.0).contains(&max), "max 10 m wind {max} m/s");
    }
}
