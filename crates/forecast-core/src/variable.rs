//! Canonical, provider-agnostic forecast variable identity.
//!
//! `FORECASTING.md`: "Provider-specific variable names stop at the adapter.
//! Canonical fields use stable RadarPro identifiers while preserving native
//! names as metadata." [`ForecastVariable`] is that stable identifier set
//! (generalizing S07's `provider-gefs::field::CanonicalField`, which had
//! exactly one variant, `Temperature2m`); [`crate::grid::NativeVariableMetadata`]
//! is where a decoded [`crate::grid::ForecastGrid`] keeps the *actual*
//! provider-native variable/level name and unit alongside this canonical
//! identity, so nothing is discarded at the provider boundary.
//!
//! Per the S08 stage file, this includes every named canonical variable
//! ("temperature_2m, dewpoint_2m, wind_u_10m, wind_v_10m, wind_gust, MSLP,
//! precipitation_1h, cloud_cover, geopotential_height, relative_humidity")
//! even though, as of this stage, only `Temperature2m` is actually decoded
//! by any provider (`provider-gefs` and `provider-hrrr` both still decode
//! only 2m temperature, matching S07's own "avoid premature abstractions"
//! rule) -- a future stage adding a second decoded variable to either
//! provider needs no change to this enum.

/// A forecast variable RadarPro can name canonically, independent of which
/// provider (or GRIB2 message, or future non-GRIB2 source) actually
/// supplies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForecastVariable {
    /// 2-meter air temperature.
    Temperature2m,
    /// 2-meter dew point temperature.
    Dewpoint2m,
    /// 10-meter eastward (u) wind component.
    WindU10m,
    /// 10-meter northward (v) wind component.
    WindV10m,
    /// Surface wind gust.
    WindGust,
    /// Mean sea level pressure.
    Mslp,
    /// 1-hour accumulated precipitation.
    Precipitation1h,
    /// Total cloud cover.
    CloudCover,
    /// Geopotential height (at whatever level the request/grid names --
    /// this variant identifies the *quantity*, not a specific level).
    GeopotentialHeight,
    /// Relative humidity (at whatever level the request/grid names).
    RelativeHumidity,
}

impl ForecastVariable {
    /// Every canonical variable, in declaration order -- useful for a UI
    /// variable picker or an exhaustiveness check in a test.
    pub const ALL: [ForecastVariable; 10] = [
        ForecastVariable::Temperature2m,
        ForecastVariable::Dewpoint2m,
        ForecastVariable::WindU10m,
        ForecastVariable::WindV10m,
        ForecastVariable::WindGust,
        ForecastVariable::Mslp,
        ForecastVariable::Precipitation1h,
        ForecastVariable::CloudCover,
        ForecastVariable::GeopotentialHeight,
        ForecastVariable::RelativeHumidity,
    ];

    /// A stable, RadarPro-native identifier (`FORECASTING.md`: "canonical
    /// fields use stable RadarPro identifiers while preserving native names
    /// as metadata").
    pub const fn canonical_name(&self) -> &'static str {
        match self {
            Self::Temperature2m => "temperature_2m",
            Self::Dewpoint2m => "dewpoint_2m",
            Self::WindU10m => "wind_u_10m",
            Self::WindV10m => "wind_v_10m",
            Self::WindGust => "wind_gust",
            Self::Mslp => "mslp",
            Self::Precipitation1h => "precipitation_1h",
            Self::CloudCover => "cloud_cover",
            Self::GeopotentialHeight => "geopotential_height",
            Self::RelativeHumidity => "relative_humidity",
        }
    }

    /// A typical/reference WMO GRIB2 Table 4.2 discipline-0 ("Meteorological
    /// products") `(parameter category, parameter number)` for this
    /// quantity -- a useful starting point for a new GRIB2-based provider,
    /// **not a guarantee every provider's real message uses this exact
    /// pair**. Confirmed identical across GEFS and HRRR for most of this
    /// enum's variables (e.g. both providers' 2m temperature really is
    /// `(0, 0)`), but empirically **false** for [`Self::Mslp`]: GEFS's real
    /// message (`MSLET`) decodes as `(3, 192)` while HRRR's
    /// (`MSLMA`) decodes as `(3, 198)` -- two genuinely different named
    /// MSLP-family products, not a bug in either provider (see
    /// `provider-gefs`/`provider-hrrr`'s own `decode::idx_names` doc
    /// comments). Because of this, no provider's `decode_field` actually
    /// cross-checks a decoded message against *this* method any more --
    /// each provider's own `idx_names` table carries the `(category,
    /// number)` pair *it* empirically verified for *its own* real message,
    /// per this project's "provider-specific names and formats stop at the
    /// adapter" rule. This method remains here only as documentation/a
    /// reference default, not part of [`crate::provider::ForecastProvider`]'s
    /// contract.
    pub const fn grib2_parameter(&self) -> (u8, u8) {
        match self {
            Self::Temperature2m => (0, 0), // TMP
            Self::Dewpoint2m => (0, 6),    // DPT
            Self::WindU10m => (2, 2),      // UGRD
            Self::WindV10m => (2, 3),      // VGRD
            Self::WindGust => (2, 22),     // GUST
            // MSLET (NCEP's Eta-model-reduction mean sea level pressure --
            // what NCEP model output conventionally calls "MSLP", and the
            // message `provider-gefs` decodes for this variable), NOT the
            // standard WMO `PRMSL` (which would be `(3, 1)`. Empirically
            // corrected during GEFS 8-variable decode work (2026-09-13):
            // real GEFS `MSLET` messages decode with parameter category 3
            // / number 192 -- a discipline-0/category-3 *local-table* code
            // (NCEP GRIB2 Table 4.2, local extension range 192-254), not
            // the standard PRMSL pairing this field previously assumed.
            // Verified against a real, live-fetched
            // `gefs.20260913/12/.../gec00...` `MSLET` message via this
            // crate's own decode path -- both `MSLET` and `PRMSL` are
            // published side by side at the identical `.idx` level
            // ("mean sea level"), so this is not a case of one message
            // simply being absent; the two are genuinely different
            // parameter codes for closely related but distinct
            // reduction methods.
            Self::Mslp => (3, 192),             // MSLET
            Self::Precipitation1h => (1, 8),    // APCP
            Self::CloudCover => (6, 1),         // TCDC
            Self::GeopotentialHeight => (3, 5), // HGT
            Self::RelativeHumidity => (1, 1),   // RH
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_every_variant_exactly_once() {
        let mut names: Vec<&'static str> = ForecastVariable::ALL
            .iter()
            .map(ForecastVariable::canonical_name)
            .collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ForecastVariable::ALL.len());
    }

    #[test]
    fn temperature_2m_canonical_name_and_parameter_match_gefs_and_hrrr() {
        // Empirically confirmed identical for both providers' real,
        // live-fetched 2m temperature messages (S07/S08).
        assert_eq!(
            ForecastVariable::Temperature2m.canonical_name(),
            "temperature_2m"
        );
        assert_eq!(ForecastVariable::Temperature2m.grib2_parameter(), (0, 0));
    }
}
