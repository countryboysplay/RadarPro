//! This crate's canonical, GEFS-scoped gridded-field type.
//!
//! Deliberately **not** added to `radar-types` (that crate is the polar
//! radar canonical model -- `ARCHITECTURE.md`'s `PolarRadarLayer` vs
//! `GridLayer` distinction -- and this stage's own "Critical rule" is not
//! to formalize a generic forecast framework yet) and deliberately **not**
//! a generic multi-provider `forecast-core` abstraction (out of scope per
//! the same rule; `HRRR`/`MRMS` would be a second concrete case to
//! generalize from, and neither exists yet). This is the smallest type
//! that can carry one real, physically- and temporally-labeled GEFS field
//! from decode through to rendering, with every piece of metadata
//! GLOBAL_CONTRACT and the S07 stage file require preserved and
//! inspectable: canonical field identity, unit, run/init time, forecast
//! lead, valid time, ensemble statistic/member identity, resolution, and
//! the decoded grid values with their lat/lon mapping.

use crate::ensemble::EnsembleIdentity;
use crate::time::UtcTimestamp;

/// A GEFS field this crate knows how to identify and decode. Only one
/// variant for this S07 proof-of-concept, per the stage file: "Start with
/// `temperature_2m`. Then wind, precipitation, MSLP" (future stages'
/// work, not this one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalField {
    /// 2-meter air temperature (`TMP:2 m above ground` in GEFS's `.idx`
    /// naming; GRIB2 discipline 0 / parameter category 0 / parameter
    /// number 0; native unit Kelvin).
    Temperature2m,
}

impl CanonicalField {
    /// A stable, RadarPro-native identifier (`FORECASTING.md`: "canonical
    /// fields use stable RadarPro identifiers while preserving native
    /// names as metadata").
    pub const fn canonical_name(&self) -> &'static str {
        match self {
            CanonicalField::Temperature2m => "temperature_2m",
        }
    }

    /// GRIB2 (parameter category, parameter number) -- WMO Table 4.2,
    /// discipline 0 ("Meteorological products").
    pub const fn grib2_parameter(&self) -> (u8, u8) {
        match self {
            CanonicalField::Temperature2m => (0, 0),
        }
    }

    /// The exact `.idx` `VARNAME` this field is listed under.
    pub const fn idx_variable(&self) -> &'static str {
        match self {
            CanonicalField::Temperature2m => "TMP",
        }
    }

    /// The exact `.idx` `LEVEL` this field is listed under.
    pub const fn idx_level(&self) -> &'static str {
        match self {
            CanonicalField::Temperature2m => "2 m above ground",
        }
    }

    /// Native (as decoded, unconverted) physical unit.
    pub const fn native_unit(&self) -> &'static str {
        match self {
            CanonicalField::Temperature2m => "K",
        }
    }
}

/// A regular latitude/longitude grid's geometry: origin (row 0, column 0)
/// plus per-step increments. Row-major: row varies with `j` (poleward
/// axis), column varies with `i` (west-east axis) -- matching
/// [`crate::decode::decode_field`]'s use of `grib::SubMessage::ij()` to
/// place each decoded value, so this geometry is valid regardless of the
/// source message's own GRIB2 scanning-mode byte (that interpretation is
/// `grib`'s job, not reimplemented here -- see `decode.rs`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridGeometry {
    /// Number of columns (the "i"/longitude axis; GRIB2 template 3.0's
    /// `Ni`).
    pub width: u32,
    /// Number of rows (the "j"/latitude axis; GRIB2 template 3.0's `Nj`).
    pub height: u32,
    /// Latitude of row 0, decimal degrees (GRIB2 `La1`).
    pub origin_lat_deg: f64,
    /// Longitude of column 0, decimal degrees, in `[0, 360)` (GRIB2 `Lo1`
    /// -- GEFS never needs negative/`[-180,180)` longitudes since its
    /// native grid already spans `[0, 360)`).
    pub origin_lon_deg: f64,
    /// Signed degrees-per-row step (GRIB2 `Dj`, sign derived from whether
    /// `La2 < La1`, i.e. whether latitude decreases as `j` increases --
    /// true for every real GEFS message observed: `La1=90` (north pole),
    /// `La2=-90` (south pole)).
    pub lat_step_deg: f64,
    /// Degrees-per-column step, always positive (GRIB2 `Di`; every real
    /// GEFS message observed scans west to east).
    pub lon_step_deg: f64,
}

impl GridGeometry {
    pub fn lat_for_row(&self, row: u32) -> f64 {
        self.origin_lat_deg + self.lat_step_deg * f64::from(row)
    }

    /// Longitude for `col`, normalized to `[0, 360)`.
    pub fn lon_for_col(&self, col: u32) -> f64 {
        (self.origin_lon_deg + self.lon_step_deg * f64::from(col)).rem_euclid(360.0)
    }

    pub fn point_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Nearest grid cell to a world `(lon_deg, lat_deg)` position, or
    /// `None` if it falls outside the grid. This is the exact CPU-side
    /// mirror of `shaders/gefs_grid.wgsl`'s per-pixel lookup formula
    /// (`round((coord - origin) / step)`, bounds-checked) -- kept in sync
    /// deliberately so this one method is the single source of truth for
    /// "which cell does this world position belong to", checked by both
    /// [`crate::gpu`]'s tests (comparing this CPU formula's answer against
    /// what the GPU shader actually rendered) and any future CPU-only
    /// caller (e.g. a cursor probe/tooltip) that needs the same answer
    /// without a GPU round-trip.
    pub fn nearest_cell(&self, lon_deg: f64, lat_deg: f64) -> Option<(u32, u32)> {
        let col_f = (lon_deg - self.origin_lon_deg) / self.lon_step_deg;
        let row_f = (lat_deg - self.origin_lat_deg) / self.lat_step_deg;
        let col = col_f.round();
        let row = row_f.round();
        if col < 0.0 || row < 0.0 {
            return None;
        }
        let col = col as u32;
        let row = row as u32;
        if col >= self.width || row >= self.height {
            return None;
        }
        Some((row, col))
    }
}

/// One fully decoded, normalized GEFS field: metadata plus a row-major
/// grid of values.
#[derive(Debug, Clone, PartialEq)]
pub struct GriddedField {
    pub field: CanonicalField,
    /// Native physical unit of every value in `values` (never mutated by
    /// display-unit conversion helpers below -- GLOBAL_CONTRACT: "display-
    /// unit changes never mutate source data").
    pub unit: &'static str,
    /// The model run's initialization ("reference") time, UTC.
    pub run_time: UtcTimestamp,
    /// Forecast lead time, in whole hours, from `run_time`.
    pub forecast_lead_hours: u32,
    /// `run_time + forecast_lead_hours` -- stored (not just derivable) so
    /// every consumer of a [`GriddedField`] can read it directly, per
    /// GLOBAL_CONTRACT's "forecasts keep initialization time, lead time,
    /// and valid time separately."
    pub valid_time: UtcTimestamp,
    /// Which ensemble statistic or member this field represents.
    pub ensemble: EnsembleIdentity,
    pub geometry: GridGeometry,
    /// Row-major values, `values[row * geometry.width + col]`, native unit
    /// (`unit`), length always `geometry.point_count()`.
    pub values: Vec<f32>,
}

impl GriddedField {
    pub fn value_at(&self, row: u32, col: u32) -> f32 {
        self.values[row as usize * self.geometry.width as usize + col as usize]
    }

    /// Convert a native-unit (Kelvin, for [`CanonicalField::Temperature2m`])
    /// value to Celsius. A pure function over a single value -- never
    /// mutates `self.values` -- so display-unit changes never touch source
    /// data (GLOBAL_CONTRACT).
    pub fn kelvin_to_celsius(kelvin: f32) -> f32 {
        kelvin - 273.15
    }

    pub fn kelvin_to_fahrenheit(kelvin: f32) -> f32 {
        Self::kelvin_to_celsius(kelvin) * 9.0 / 5.0 + 32.0
    }

    /// A rectangular lat/lon subset of this field -- the CPU-side
    /// "viewport-aware/regional subset selection" the S07 stage file calls
    /// for. GRIB2's per-message granularity means the whole message must
    /// still be *decoded* first (unlike Zarr's per-chunk addressing, which
    /// the stage file explicitly warns not to assume carries over --
    /// GRIB2 offers no sub-message field/region seek point); this crops
    /// the already-decoded grid down to a region of interest before it
    /// ever reaches the GPU, which is what actually matters for render
    /// cost and VRAM.
    ///
    /// Bounds are inclusive; `lon_min`/`lon_max` are in `[0, 360)`
    /// (matching [`GridGeometry::lon_for_col`]) and `lon_min <= lon_max`
    /// (no antimeridian wraparound support in this PoC). Returns `None` if
    /// the requested box has no overlap with this grid, rather than an
    /// empty-but-valid subset that could be mistaken for "the field is all
    /// missing here."
    pub fn subset(
        &self,
        lat_min: f64,
        lat_max: f64,
        lon_min: f64,
        lon_max: f64,
    ) -> Option<GriddedField> {
        let mut rows: Vec<u32> = (0..self.geometry.height)
            .filter(|&r| {
                let lat = self.geometry.lat_for_row(r);
                lat >= lat_min && lat <= lat_max
            })
            .collect();
        let mut cols: Vec<u32> = (0..self.geometry.width)
            .filter(|&c| {
                let lon = self.geometry.lon_for_col(c);
                lon >= lon_min && lon <= lon_max
            })
            .collect();
        rows.sort_unstable();
        cols.sort_unstable();
        if rows.is_empty() || cols.is_empty() {
            return None;
        }

        let row_start = *rows.first().unwrap();
        let row_end = *rows.last().unwrap();
        let col_start = *cols.first().unwrap();
        let col_end = *cols.last().unwrap();
        let width = col_end - col_start + 1;
        let height = row_end - row_start + 1;

        let mut values = Vec::with_capacity(width as usize * height as usize);
        for row in row_start..=row_end {
            for col in col_start..=col_end {
                values.push(self.value_at(row, col));
            }
        }

        Some(GriddedField {
            field: self.field,
            unit: self.unit,
            run_time: self.run_time,
            forecast_lead_hours: self.forecast_lead_hours,
            valid_time: self.valid_time,
            ensemble: self.ensemble,
            geometry: GridGeometry {
                width,
                height,
                origin_lat_deg: self.geometry.lat_for_row(row_start),
                origin_lon_deg: self.geometry.lon_for_col(col_start),
                lat_step_deg: self.geometry.lat_step_deg,
                lon_step_deg: self.geometry.lon_step_deg,
            },
            values,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_field() -> GriddedField {
        // A tiny synthetic 4x3 grid (width=4 columns of longitude,
        // height=3 rows of latitude), matching real GEFS's north-first,
        // west-to-east convention: row 0 = 10N, row step -5 deg; col 0 =
        // 100E, col step +5 deg.
        let geometry = GridGeometry {
            width: 4,
            height: 3,
            origin_lat_deg: 10.0,
            origin_lon_deg: 100.0,
            lat_step_deg: -5.0,
            lon_step_deg: 5.0,
        };
        #[rustfmt::skip]
        let values = vec![
            0.0, 1.0, 2.0, 3.0,
            10.0, 11.0, 12.0, 13.0,
            20.0, 21.0, 22.0, 23.0,
        ];
        GriddedField {
            field: CanonicalField::Temperature2m,
            unit: "K",
            run_time: UtcTimestamp::new(2026, 9, 12, 12, 0, 0),
            forecast_lead_hours: 0,
            valid_time: UtcTimestamp::new(2026, 9, 12, 12, 0, 0),
            ensemble: EnsembleIdentity::Control,
            geometry,
            values,
        }
    }

    #[test]
    fn value_at_reads_row_major() {
        let field = sample_field();
        assert_eq!(field.value_at(0, 0), 0.0);
        assert_eq!(field.value_at(0, 3), 3.0);
        assert_eq!(field.value_at(2, 0), 20.0);
        assert_eq!(field.value_at(1, 2), 12.0);
    }

    #[test]
    fn lat_lon_for_row_col_matches_geometry() {
        let field = sample_field();
        assert_eq!(field.geometry.lat_for_row(0), 10.0);
        assert_eq!(field.geometry.lat_for_row(2), 0.0);
        assert_eq!(field.geometry.lon_for_col(0), 100.0);
        assert_eq!(field.geometry.lon_for_col(3), 115.0);
    }

    #[test]
    fn kelvin_conversions_are_correct_and_never_mutate_source() {
        let field = sample_field();
        let original = field.values.clone();
        assert!((GriddedField::kelvin_to_celsius(273.15) - 0.0).abs() < 1e-6);
        assert!((GriddedField::kelvin_to_fahrenheit(273.15) - 32.0).abs() < 1e-6);
        assert!((GriddedField::kelvin_to_celsius(373.15) - 100.0).abs() < 1e-6);
        assert_eq!(
            field.values, original,
            "conversion must not mutate source data"
        );
    }

    #[test]
    fn subset_crops_to_the_requested_box() {
        let field = sample_field();
        // lat in [0, 10] keeps rows 0,1,2 (10, 5, 0); lon in [100, 110]
        // keeps cols 0,1,2 (100,105,110).
        let subset = field.subset(0.0, 10.0, 100.0, 110.0).unwrap();
        assert_eq!(subset.geometry.width, 3);
        assert_eq!(subset.geometry.height, 3);
        assert_eq!(subset.value_at(0, 0), 0.0);
        assert_eq!(subset.value_at(2, 2), 22.0);
        // Metadata is preserved through subsetting.
        assert_eq!(subset.field, field.field);
        assert_eq!(subset.run_time, field.run_time);
        assert_eq!(subset.valid_time, field.valid_time);
        assert_eq!(subset.ensemble, field.ensemble);
    }

    #[test]
    fn subset_returns_none_for_a_box_with_no_overlap() {
        let field = sample_field();
        assert!(field.subset(50.0, 60.0, 100.0, 110.0).is_none());
        assert!(field.subset(0.0, 10.0, 200.0, 210.0).is_none());
    }

    #[test]
    fn nearest_cell_finds_exact_cell_centers() {
        let field = sample_field();
        assert_eq!(field.geometry.nearest_cell(100.0, 10.0), Some((0, 0)));
        assert_eq!(field.geometry.nearest_cell(115.0, 10.0), Some((0, 3)));
        assert_eq!(field.geometry.nearest_cell(100.0, 0.0), Some((2, 0)));
    }

    #[test]
    fn nearest_cell_rounds_to_the_closest_cell() {
        let field = sample_field();
        // Halfway-ish between col 0 (lon 100) and col 1 (lon 105), closer
        // to col 0.
        assert_eq!(field.geometry.nearest_cell(102.0, 10.0), Some((0, 0)));
        assert_eq!(field.geometry.nearest_cell(104.0, 10.0), Some((0, 1)));
    }

    #[test]
    fn nearest_cell_returns_none_outside_the_grid() {
        let field = sample_field();
        assert_eq!(field.geometry.nearest_cell(50.0, 10.0), None);
        assert_eq!(field.geometry.nearest_cell(100.0, 50.0), None);
        assert_eq!(field.geometry.nearest_cell(100.0, -50.0), None);
    }

    #[test]
    fn nearest_cell_matches_real_gefs_geometry_north_pole() {
        // The real, empirically-verified GEFS grid: (row 0, col 0) is
        // exactly the North Pole.
        let geometry = GridGeometry {
            width: 1440,
            height: 721,
            origin_lat_deg: 90.0,
            origin_lon_deg: 0.0,
            lat_step_deg: -0.25,
            lon_step_deg: 0.25,
        };
        assert_eq!(geometry.nearest_cell(0.0, 90.0), Some((0, 0)));
        // The last row is the South Pole.
        assert_eq!(geometry.nearest_cell(0.0, -90.0), Some((720, 0)));
        // A real, familiar reference point: (0N, 0E), the grid's exact
        // center latitude.
        assert_eq!(geometry.nearest_cell(0.0, 0.0), Some((360, 0)));
    }
}
