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

    /// WMO GRIB2 Table 4.2 discipline-0 ("Meteorological products")
    /// `(parameter category, parameter number)` for this quantity -- the
    /// same pair regardless of which GRIB2-based provider's message carries
    /// it (empirically confirmed identical for GEFS and HRRR's 2m
    /// temperature: both `(0, 0)`). A convenience for GRIB2-based providers
    /// cross-checking a decoded message's declared parameter against the
    /// field they asked for; not part of [`crate::provider::ForecastProvider`]'s
    /// contract, since a future non-GRIB2 provider would not use this at
    /// all.
    pub const fn grib2_parameter(&self) -> (u8, u8) {
        match self {
            Self::Temperature2m => (0, 0),      // TMP
            Self::Dewpoint2m => (0, 6),         // DPT
            Self::WindU10m => (2, 2),           // UGRD
            Self::WindV10m => (2, 3),           // VGRD
            Self::WindGust => (2, 22),          // GUST
            Self::Mslp => (3, 1),               // PRMSL / MSLET
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
