//! [`PointForecast`]: a single decoded value at a specific point, derived
//! from a [`crate::grid::ForecastGrid`] via [`crate::grid::ForecastGrid::point_forecast`]
//! (e.g. for a cursor probe/tooltip, without a GPU round-trip).

use crate::ensemble::EnsembleStatistic;
use crate::time::UtcTimestamp;
use crate::variable::ForecastVariable;

/// One forecast value at one point, at one valid time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointForecast {
    pub variable: ForecastVariable,
    pub lon_deg: f64,
    pub lat_deg: f64,
    pub valid_time: UtcTimestamp,
    /// Native-unit (`unit`) decoded value at the nearest grid cell to
    /// `(lon_deg, lat_deg)` -- never interpolated between cells, matching
    /// this project's existing nearest-only convention (GLOBAL_CONTRACT:
    /// do not fabricate structure between real samples).
    pub value: f32,
    pub unit: &'static str,
    pub ensemble: Option<EnsembleStatistic>,
}
