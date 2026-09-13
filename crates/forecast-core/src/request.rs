//! [`FieldRequest`]: what a caller asks a [`crate::provider::ForecastProvider`]
//! for -- one variable, one forecast lead, and (for a provider that has an
//! ensemble) one statistic/member.

use crate::ensemble::EnsembleStatistic;
use crate::variable::ForecastVariable;

/// A request for one decoded field: a single variable, forecast lead, and
/// (if the target provider has an ensemble at all) ensemble statistic or
/// member.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldRequest {
    pub variable: ForecastVariable,
    /// Forecast lead time in whole hours from the run's initialization
    /// time (`0` for an analysis/nowcast field).
    pub forecast_lead_hours: u32,
    /// `Some(..)` to request a specific ensemble statistic/member from a
    /// provider that has one; `None` for a deterministic provider (a
    /// provider with no ensemble, like HRRR, must reject a request with
    /// `Some(..)` here rather than silently ignoring it -- see
    /// `provider-hrrr`'s own `ForecastProvider` implementation).
    pub ensemble: Option<EnsembleStatistic>,
}

impl FieldRequest {
    pub const fn new(variable: ForecastVariable, forecast_lead_hours: u32) -> Self {
        Self {
            variable,
            forecast_lead_hours,
            ensemble: None,
        }
    }

    pub const fn with_ensemble(mut self, ensemble: EnsembleStatistic) -> Self {
        self.ensemble = Some(ensemble);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults_to_no_ensemble_statistic() {
        let request = FieldRequest::new(crate::variable::ForecastVariable::Temperature2m, 0);
        assert_eq!(request.ensemble, None);
    }

    #[test]
    fn with_ensemble_sets_the_requested_statistic() {
        let request = FieldRequest::new(crate::variable::ForecastVariable::Temperature2m, 0)
            .with_ensemble(EnsembleStatistic::Mean);
        assert_eq!(request.ensemble, Some(EnsembleStatistic::Mean));
    }
}
