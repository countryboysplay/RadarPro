//! Canonical ensemble statistic/member identity -- generalizes S07's
//! `provider-gefs::ensemble::EnsembleIdentity` to be provider-agnostic.
//!
//! The S08 stage file requires this type to "represent GEFS's
//! control/member/mean *and* be extensible to a provider with no ensemble
//! at all, like HRRR, which is deterministic -- do not force HRRR to fake
//! an ensemble identity it doesn't have." The chosen shape is
//! [`EnsembleStatistic`] (an enum with no "not applicable" variant of its
//! own) carried as `Option<EnsembleStatistic>` on [`crate::grid::ForecastGrid`]:
//! `None` means "this provider has no ensemble concept at all" (HRRR,
//! confirmed empirically: every real HRRR message decoded for this stage
//! used GRIB2 Product Definition Template 4.0, "analysis or forecast at a
//! horizontal level/layer", which carries no ensemble metadata whatsoever --
//! not template 4.1/4.2 the way GEFS's members/mean do). This is
//! deliberately different from adding a `Deterministic`/`None`-like variant
//! *inside* the enum, which would force every consumer to match on a case
//! that only ever means "ignore every other field of this enum" -- an
//! `Option` says that more directly and composes with `Option`'s own
//! combinators (`map`, `is_some`, etc.) for free.

/// Which ensemble statistic or member a decoded [`crate::grid::ForecastGrid`]
/// represents, for a provider that has an ensemble at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnsembleStatistic {
    /// The unperturbed control forecast.
    Control,
    /// A perturbed ensemble member, carrying its perturbation number.
    Member(u8),
    /// The ensemble mean.
    Mean,
    /// A named percentile (e.g. `Percentile(50)` for the median). GEFS does
    /// not natively publish percentiles (S08 stage file: "GEFS does not
    /// natively provide percentiles like p10/p25/p50/p75/p90; do not invent
    /// an interpolation scheme to fake them without a documented
    /// justification") and no provider implemented in this stage
    /// constructs this variant -- it exists so this type is ready for a
    /// future provider that *does* publish named percentiles directly
    /// (`FORECASTING.md`'s explicit re-add process for a provider like
    /// that) without another breaking change to this enum.
    Percentile(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_are_distinguishable_by_equality() {
        assert_eq!(EnsembleStatistic::Control, EnsembleStatistic::Control);
        assert_ne!(EnsembleStatistic::Control, EnsembleStatistic::Mean);
        assert_eq!(EnsembleStatistic::Member(1), EnsembleStatistic::Member(1));
        assert_ne!(EnsembleStatistic::Member(1), EnsembleStatistic::Member(2));
        assert_ne!(EnsembleStatistic::Percentile(50), EnsembleStatistic::Mean);
    }

    #[test]
    fn a_deterministic_provider_is_represented_by_none_not_a_variant() {
        // This is the whole point of the `Option<EnsembleStatistic>`
        // design documented above: a deterministic field's ensemble
        // identity is simply absent, not a special enum value.
        let deterministic_field_ensemble: Option<EnsembleStatistic> = None;
        assert!(deterministic_field_ensemble.is_none());
    }
}
