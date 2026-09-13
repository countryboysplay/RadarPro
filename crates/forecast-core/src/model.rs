//! [`ModelMetadata`] (a provider's own static identity/description) and
//! [`ModelRun`] (one specific published run of that model), the canonical
//! concepts the S08 stage file names alongside `ForecastProvider`.
//!
//! Deliberately thin: everything genuinely provider-specific about "how do
//! you name/discover a run" (GEFS's `RunReference`/`RunHour`, HRRR's own
//! date+hour run key) stays inside each provider crate as
//! `ForecastProvider::Run` (see `src/provider.rs`) rather than being
//! generalized into this struct -- [`ModelRun`] only carries what every
//! provider can express identically: the run's UTC initialization time and
//! a human-readable label.

use crate::time::UtcTimestamp;

/// A provider's own static identity/description -- never varies per run or
/// per request, so a UI can show "which model is this" without asking the
/// provider to fetch anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelMetadata {
    /// A stable, lowercase, machine-usable identifier, e.g. `"gefs"`,
    /// `"hrrr"`.
    pub provider_id: &'static str,
    /// A human-readable display name, e.g. `"NOAA GEFS"`, `"NOAA HRRR"`.
    pub display_name: &'static str,
    /// Whether this provider publishes an ensemble at all (`true` for
    /// GEFS, `false` for HRRR) -- lets a UI decide whether to show an
    /// ensemble-statistic/member picker at all, without inspecting a
    /// specific [`crate::grid::ForecastGrid`]'s `ensemble` field first.
    pub is_ensemble: bool,
    /// A short, human-readable resolution/domain description, e.g.
    /// `"0.25 degree, global"` or `"~3 km, CONUS"` -- display metadata
    /// only, never parsed.
    pub resolution_description: &'static str,
}

/// One specific, already-published run of a model: its UTC initialization
/// ("reference") time, plus a human-readable label for display
/// (e.g. `"2026-09-12 12Z"`). Provider-specific run identity (how to turn
/// this back into that provider's own object keys) is carried separately,
/// as each [`crate::provider::ForecastProvider`] implementation's own
/// associated `Run` type -- see that trait's module docs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelRun {
    pub init_time: UtcTimestamp,
    pub label: String,
}

impl ModelRun {
    pub fn new(init_time: UtcTimestamp, label: impl Into<String>) -> Self {
        Self {
            init_time,
            label: label.into(),
        }
    }
}

impl std::fmt::Display for ModelRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_run_displays_its_label_verbatim() {
        let run = ModelRun::new(UtcTimestamp::new(2026, 9, 12, 12, 0, 0), "2026-09-12 12Z");
        assert_eq!(run.to_string(), "2026-09-12 12Z");
    }
}
