//! [`ForecastGrid`]: this crate's canonical, provider-agnostic decoded
//! gridded-field type -- generalizes S07's `provider-gefs::field::GriddedField`
//! (and its `GridGeometry`) to represent either provider's real grid
//! shape: GEFS's regular latitude/longitude grid ([`RegularLatLonGrid`])
//! *and* HRRR's real Lambert Conformal Conic grid ([`LambertConformalGrid`],
//! WMO GRIB2 Grid Definition Template 3.30 -- confirmed empirically against
//! a real, live-fetched HRRR message; see `docs/adr/0012-hrrr-lambert-conformal-grid.md`
//! and [`crate::projection`]).
//!
//! [`GridGeometry`] is deliberately an enum of exactly these two concrete
//! shapes rather than one single "origin + step" struct: a Lambert
//! conformal grid's rows/columns are *not* lines of constant latitude/
//! longitude (unlike a regular lat/lon grid), so forcing both through one
//! linear-step formula would either be wrong for HRRR or would require
//! HRRR to silently resample onto a regular lat/lon grid -- exactly the
//! kind of "silently fall back to treating it as a regular lat/lon grid...
//! a real geometry bug" the S08 stage brief warns against. Both variants
//! expose the same `nearest_cell`/`lon_lat_for_cell` methods, so every
//! provider-agnostic caller (subsetting, point-forecast lookup, the GPU
//! renderer's CPU-side uniform prep in [`crate::gpu`]) uses one call
//! path regardless of which variant a given [`ForecastGrid`] carries.

use crate::ensemble::EnsembleStatistic;
use crate::projection::LccProjection;
use crate::time::UtcTimestamp;
use crate::variable::ForecastVariable;

/// A regular latitude/longitude grid's geometry: origin (row 0, column 0)
/// plus per-step increments. Row-major: row varies with `j` (poleward
/// axis), column varies with `i` (west-east axis) -- generalized verbatim
/// from S07's `provider-gefs::field::GridGeometry` (GEFS's own real grid
/// uses exactly this shape, GRIB2 Grid Definition Template 3.0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegularLatLonGrid {
    /// Number of columns (the "i"/longitude axis).
    pub width: u32,
    /// Number of rows (the "j"/latitude axis).
    pub height: u32,
    /// Latitude of row 0, decimal degrees.
    pub origin_lat_deg: f64,
    /// Longitude of column 0, decimal degrees, in `[0, 360)`.
    pub origin_lon_deg: f64,
    /// Signed degrees-per-row step (negative when latitude decreases as
    /// row increases, e.g. GEFS's north-pole-first convention).
    pub lat_step_deg: f64,
    /// Degrees-per-column step, always positive for every real grid
    /// observed (west to east).
    pub lon_step_deg: f64,
}

impl RegularLatLonGrid {
    pub fn lat_for_row(&self, row: u32) -> f64 {
        self.origin_lat_deg + self.lat_step_deg * f64::from(row)
    }

    /// Longitude for `col`, normalized to `[0, 360)`.
    pub fn lon_for_col(&self, col: u32) -> f64 {
        (self.origin_lon_deg + self.lon_step_deg * f64::from(col)).rem_euclid(360.0)
    }

    /// Nearest grid cell to a world `(lon_deg, lat_deg)` position, or
    /// `None` if it falls outside the grid.
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

/// A Lambert Conformal Conic grid's geometry: HRRR's real grid shape (WMO
/// GRIB2 Grid Definition Template 3.30). Unlike [`RegularLatLonGrid`], rows
/// and columns are lines of constant *projected* x/y (meters on the
/// Lambert plane), not constant latitude/longitude -- see
/// [`crate::projection`] for the forward/inverse math this wraps.
#[derive(Debug, Clone, PartialEq)]
pub struct LambertConformalGrid {
    pub width: u32,
    pub height: u32,
    pub projection: LccProjection,
    /// Projected x/y (meters) of grid cell `(row=0, col=0)`.
    pub origin_x_m: f64,
    pub origin_y_m: f64,
    /// Signed meters-per-column/-row step.
    pub dx_m: f64,
    pub dy_m: f64,
}

impl LambertConformalGrid {
    pub fn lon_lat_for_cell(&self, row: u32, col: u32) -> (f64, f64) {
        let x = self.origin_x_m + self.dx_m * f64::from(col);
        let y = self.origin_y_m + self.dy_m * f64::from(row);
        self.projection.unproject(x, y)
    }

    pub fn nearest_cell(&self, lon_deg: f64, lat_deg: f64) -> Option<(u32, u32)> {
        let (x, y) = self.projection.project(lon_deg, lat_deg);
        let col_f = ((x - self.origin_x_m) / self.dx_m).round();
        let row_f = ((y - self.origin_y_m) / self.dy_m).round();
        if col_f < 0.0 || row_f < 0.0 {
            return None;
        }
        let col = col_f as u32;
        let row = row_f as u32;
        if col >= self.width || row >= self.height {
            return None;
        }
        Some((row, col))
    }
}

/// This [`ForecastGrid`]'s geometry: either of the two real grid shapes
/// this crate's providers use. See this module's doc comment for why this
/// is an enum of two concrete shapes rather than one generalized formula.
#[derive(Debug, Clone, PartialEq)]
pub enum GridGeometry {
    RegularLatLon(RegularLatLonGrid),
    LambertConformal(LambertConformalGrid),
}

impl GridGeometry {
    pub fn width(&self) -> u32 {
        match self {
            Self::RegularLatLon(g) => g.width,
            Self::LambertConformal(g) => g.width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Self::RegularLatLon(g) => g.height,
            Self::LambertConformal(g) => g.height,
        }
    }

    pub fn point_count(&self) -> usize {
        self.width() as usize * self.height() as usize
    }

    /// World `(longitude, latitude)` for grid cell `(row, col)`, longitude
    /// normalized to `[0, 360)` for both variants (matching
    /// [`RegularLatLonGrid::lon_for_col`]'s existing convention, so
    /// [`ForecastGrid::subset`] and any other provider-agnostic caller
    /// never has to branch on which variant it has).
    pub fn lon_lat_for_cell(&self, row: u32, col: u32) -> (f64, f64) {
        let (lon, lat) = match self {
            Self::RegularLatLon(g) => (g.lon_for_col(col), g.lat_for_row(row)),
            Self::LambertConformal(g) => g.lon_lat_for_cell(row, col),
        };
        (lon.rem_euclid(360.0), lat)
    }

    /// Nearest grid cell to a world `(lon_deg, lat_deg)` position, or
    /// `None` if it falls outside the grid. Accepts longitude in either
    /// the `[0, 360)` or `(-180, 180]` convention for both variants (the
    /// regular-lat-lon formula only ever uses a difference from its own
    /// `origin_lon_deg`, and [`crate::projection::LccProjection::project`]
    /// explicitly normalizes the longitude difference it computes).
    pub fn nearest_cell(&self, lon_deg: f64, lat_deg: f64) -> Option<(u32, u32)> {
        match self {
            Self::RegularLatLon(g) => g.nearest_cell(lon_deg, lat_deg),
            Self::LambertConformal(g) => g.nearest_cell(lon_deg, lat_deg),
        }
    }
}

/// A decoded field's provider-native identity, preserved alongside its
/// canonical [`ForecastVariable`] -- `FORECASTING.md`: "canonical fields
/// use stable RadarPro identifiers while preserving native names as
/// metadata." Nothing about a provider's own vocabulary is discarded at
/// the provider boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeVariableMetadata {
    /// The provider's own variable name for this field, verbatim (e.g.
    /// GEFS/HRRR's shared `.idx` `VARNAME`, `"TMP"`).
    pub provider_variable_name: String,
    /// The provider's own level/layer description, verbatim (e.g. `"2 m
    /// above ground"`).
    pub provider_level_name: String,
    /// Native (as-decoded, unconverted) physical unit, e.g. `"K"`.
    pub native_unit: &'static str,
}

/// One fully decoded, normalized forecast field: metadata plus a row-major
/// grid of values. Generalizes S07's `provider-gefs::field::GriddedField`
/// to be provider-agnostic (any [`ForecastProvider`][crate::provider::ForecastProvider]
/// can produce one) while preserving every piece of metadata
/// GLOBAL_CONTRACT and the S08 stage file require: canonical variable
/// identity plus provider-native identity, run/init time, forecast lead,
/// valid time, ensemble statistic/member identity (or its absence, for a
/// deterministic provider), resolution/geometry, and the decoded grid
/// values with their world-position mapping.
///
/// # This is forecast guidance, never observed radar
///
/// GLOBAL_CONTRACT.md: "Never call model precipitation 'future radar'" /
/// "Observations and forecasts must always be distinguishable." Every
/// [`ForecastGrid`] carries its own run/init time, forecast lead, and
/// valid time explicitly -- nothing built from this type is, or should
/// ever be labeled as, an observation.
#[derive(Debug, Clone, PartialEq)]
pub struct ForecastGrid {
    pub variable: ForecastVariable,
    pub native: NativeVariableMetadata,
    /// The producing provider's stable identifier (`ModelMetadata::provider_id`),
    /// kept on the grid itself so a caller holding just a `ForecastGrid`
    /// (e.g. deep in the shared render path) can still attribute it,
    /// without this ever being used to branch rendering behavior -- see
    /// `crate::gpu`'s module doc.
    pub provider_id: &'static str,
    /// Native physical unit of every value in `values` (never mutated by
    /// display-unit conversion helpers in [`crate::render`] --
    /// GLOBAL_CONTRACT: "display-unit changes never mutate source data").
    pub unit: &'static str,
    /// The model run's initialization ("reference") time, UTC.
    pub run_time: UtcTimestamp,
    /// Forecast lead time, in whole hours, from `run_time`.
    pub forecast_lead_hours: u32,
    /// `run_time + forecast_lead_hours`.
    pub valid_time: UtcTimestamp,
    /// Which ensemble statistic or member this field represents, or `None`
    /// for a deterministic provider (see [`crate::ensemble`]'s module doc).
    pub ensemble: Option<EnsembleStatistic>,
    pub geometry: GridGeometry,
    /// Row-major values, `values[row * geometry.width() + col]`, native
    /// unit (`unit`), length always `geometry.point_count()`.
    pub values: Vec<f32>,
}

impl ForecastGrid {
    pub fn value_at(&self, row: u32, col: u32) -> f32 {
        self.values[row as usize * self.geometry.width() as usize + col as usize]
    }

    /// Convert a native-unit Kelvin value to Celsius. A pure function over
    /// a single value -- never mutates `self.values` -- so display-unit
    /// changes never touch source data (GLOBAL_CONTRACT).
    pub fn kelvin_to_celsius(kelvin: f32) -> f32 {
        kelvin - 273.15
    }

    pub fn kelvin_to_fahrenheit(kelvin: f32) -> f32 {
        Self::kelvin_to_celsius(kelvin) * 9.0 / 5.0 + 32.0
    }

    /// The value nearest a world `(lon_deg, lat_deg)` position, plus its
    /// valid time/unit/ensemble identity, as a [`crate::point::PointForecast`] --
    /// `None` if the position falls outside this grid.
    pub fn point_forecast(
        &self,
        lon_deg: f64,
        lat_deg: f64,
    ) -> Option<crate::point::PointForecast> {
        let (row, col) = self.geometry.nearest_cell(lon_deg, lat_deg)?;
        Some(crate::point::PointForecast {
            variable: self.variable,
            lat_deg,
            lon_deg,
            valid_time: self.valid_time,
            value: self.value_at(row, col),
            unit: self.unit,
            ensemble: self.ensemble,
        })
    }

    /// A rectangular lat/lon subset of this field: the tightest axis-
    /// aligned grid-index rectangle (in `(row, col)` space) containing
    /// every cell whose own `(lon, lat)` (from [`GridGeometry::lon_lat_for_cell`])
    /// falls inside the requested box. For a [`RegularLatLonGrid`], this is
    /// exactly the requested box (a row's latitude and a column's
    /// longitude are each independent of the other axis, so the rectangle
    /// has no "extra" cells). For a [`LambertConformalGrid`], row/column
    /// are *not* lines of constant latitude/longitude, so the returned
    /// rectangle may include a modest border of cells just outside the
    /// exact requested box -- an axis-aligned array crop, not a scientific
    /// transformation of the data (every value keeps its own real decoded
    /// number; nothing is interpolated or fabricated), matching this
    /// method's original documented purpose (S07): crop the already-
    /// decoded grid down for render cost/VRAM before it reaches the GPU.
    ///
    /// Bounds are inclusive; `lon_min`/`lon_max` are in `[0, 360)` (matching
    /// [`GridGeometry::lon_lat_for_cell`]) and `lon_min <= lon_max` (no
    /// antimeridian wraparound support). Returns `None` if the requested
    /// box has no overlap with this grid.
    pub fn subset(
        &self,
        lat_min: f64,
        lat_max: f64,
        lon_min: f64,
        lon_max: f64,
    ) -> Option<ForecastGrid> {
        let width = self.geometry.width();
        let height = self.geometry.height();

        let mut row_range: Option<(u32, u32)> = None;
        let mut col_range: Option<(u32, u32)> = None;
        for row in 0..height {
            for col in 0..width {
                let (lon, lat) = self.geometry.lon_lat_for_cell(row, col);
                if lat < lat_min || lat > lat_max || lon < lon_min || lon > lon_max {
                    continue;
                }
                row_range = Some(match row_range {
                    None => (row, row),
                    Some((lo, hi)) => (lo.min(row), hi.max(row)),
                });
                col_range = Some(match col_range {
                    None => (col, col),
                    Some((lo, hi)) => (lo.min(col), hi.max(col)),
                });
            }
        }
        let (row_start, row_end) = row_range?;
        let (col_start, col_end) = col_range?;

        let new_width = col_end - col_start + 1;
        let new_height = row_end - row_start + 1;

        let mut values = Vec::with_capacity(new_width as usize * new_height as usize);
        for row in row_start..=row_end {
            for col in col_start..=col_end {
                values.push(self.value_at(row, col));
            }
        }

        let new_geometry = match &self.geometry {
            GridGeometry::RegularLatLon(g) => GridGeometry::RegularLatLon(RegularLatLonGrid {
                width: new_width,
                height: new_height,
                origin_lat_deg: g.lat_for_row(row_start),
                origin_lon_deg: g.lon_for_col(col_start),
                lat_step_deg: g.lat_step_deg,
                lon_step_deg: g.lon_step_deg,
            }),
            GridGeometry::LambertConformal(g) => {
                GridGeometry::LambertConformal(LambertConformalGrid {
                    width: new_width,
                    height: new_height,
                    projection: g.projection.clone(),
                    origin_x_m: g.origin_x_m + g.dx_m * f64::from(col_start),
                    origin_y_m: g.origin_y_m + g.dy_m * f64::from(row_start),
                    dx_m: g.dx_m,
                    dy_m: g.dy_m,
                })
            }
        };

        Some(ForecastGrid {
            variable: self.variable,
            native: self.native.clone(),
            provider_id: self.provider_id,
            unit: self.unit,
            run_time: self.run_time,
            forecast_lead_hours: self.forecast_lead_hours,
            valid_time: self.valid_time,
            ensemble: self.ensemble,
            geometry: new_geometry,
            values,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::{LccParams, HRRR_EARTH_RADIUS_M};

    fn sample_regular_field() -> ForecastGrid {
        // A tiny synthetic 4x3 grid (width=4 columns of longitude,
        // height=3 rows of latitude), matching real GEFS's north-first,
        // west-to-east convention: row 0 = 10N, row step -5 deg; col 0 =
        // 100E, col step +5 deg.
        let geometry = GridGeometry::RegularLatLon(RegularLatLonGrid {
            width: 4,
            height: 3,
            origin_lat_deg: 10.0,
            origin_lon_deg: 100.0,
            lat_step_deg: -5.0,
            lon_step_deg: 5.0,
        });
        #[rustfmt::skip]
        let values = vec![
            0.0, 1.0, 2.0, 3.0,
            10.0, 11.0, 12.0, 13.0,
            20.0, 21.0, 22.0, 23.0,
        ];
        ForecastGrid {
            variable: ForecastVariable::Temperature2m,
            native: NativeVariableMetadata {
                provider_variable_name: "TMP".to_string(),
                provider_level_name: "2 m above ground".to_string(),
                native_unit: "K",
            },
            provider_id: "gefs",
            unit: "K",
            run_time: UtcTimestamp::new(2026, 9, 12, 12, 0, 0),
            forecast_lead_hours: 0,
            valid_time: UtcTimestamp::new(2026, 9, 12, 12, 0, 0),
            ensemble: Some(EnsembleStatistic::Control),
            geometry,
            values,
        }
    }

    fn sample_lambert_field() -> ForecastGrid {
        let projection = LccProjection::new(LccParams {
            earth_radius_m: HRRR_EARTH_RADIUS_M,
            standard_parallel_1_deg: 38.5,
            standard_parallel_2_deg: 38.5,
            latitude_of_origin_deg: 38.5,
            central_meridian_deg: -97.5,
        });
        let (origin_x_m, origin_y_m) = projection.project(-122.71953, 21.138123);
        let geometry = GridGeometry::LambertConformal(LambertConformalGrid {
            width: 3,
            height: 3,
            projection,
            origin_x_m,
            origin_y_m,
            dx_m: 3000.0,
            dy_m: 3000.0,
        });
        ForecastGrid {
            variable: ForecastVariable::Temperature2m,
            native: NativeVariableMetadata {
                provider_variable_name: "TMP".to_string(),
                provider_level_name: "2 m above ground".to_string(),
                native_unit: "K",
            },
            provider_id: "hrrr",
            unit: "K",
            run_time: UtcTimestamp::new(2026, 9, 12, 12, 0, 0),
            forecast_lead_hours: 0,
            valid_time: UtcTimestamp::new(2026, 9, 12, 12, 0, 0),
            ensemble: None,
            geometry,
            values: vec![0.0; 9],
        }
    }

    #[test]
    fn value_at_reads_row_major() {
        let field = sample_regular_field();
        assert_eq!(field.value_at(0, 0), 0.0);
        assert_eq!(field.value_at(0, 3), 3.0);
        assert_eq!(field.value_at(2, 0), 20.0);
        assert_eq!(field.value_at(1, 2), 12.0);
    }

    #[test]
    fn kelvin_conversions_are_correct_and_never_mutate_source() {
        let field = sample_regular_field();
        let original = field.values.clone();
        assert!((ForecastGrid::kelvin_to_celsius(273.15) - 0.0).abs() < 1e-6);
        assert!((ForecastGrid::kelvin_to_fahrenheit(273.15) - 32.0).abs() < 1e-6);
        assert!((ForecastGrid::kelvin_to_celsius(373.15) - 100.0).abs() < 1e-6);
        assert_eq!(
            field.values, original,
            "conversion must not mutate source data"
        );
    }

    #[test]
    fn subset_crops_a_regular_grid_to_exactly_the_requested_box() {
        let field = sample_regular_field();
        let subset = field.subset(0.0, 10.0, 100.0, 110.0).unwrap();
        assert_eq!(subset.geometry.width(), 3);
        assert_eq!(subset.geometry.height(), 3);
        assert_eq!(subset.value_at(0, 0), 0.0);
        assert_eq!(subset.value_at(2, 2), 22.0);
        assert_eq!(subset.variable, field.variable);
        assert_eq!(subset.run_time, field.run_time);
        assert_eq!(subset.valid_time, field.valid_time);
        assert_eq!(subset.ensemble, field.ensemble);
    }

    #[test]
    fn subset_returns_none_for_a_box_with_no_overlap() {
        let field = sample_regular_field();
        assert!(field.subset(50.0, 60.0, 100.0, 110.0).is_none());
        assert!(field.subset(0.0, 10.0, 200.0, 210.0).is_none());
    }

    #[test]
    fn subset_also_works_for_a_lambert_conformal_grid() {
        let field = sample_lambert_field();
        // The whole 3x3 synthetic grid's own bounding box, with margin, so
        // every cell is included -- proves subset() does not panic or
        // mis-crop for a non-regular grid geometry.
        let subset = field.subset(15.0, 30.0, 230.0, 245.0).unwrap();
        assert_eq!(subset.geometry.width(), 3);
        assert_eq!(subset.geometry.height(), 3);
        assert!(matches!(subset.geometry, GridGeometry::LambertConformal(_)));
    }

    #[test]
    fn nearest_cell_finds_exact_cell_centers_on_a_regular_grid() {
        let field = sample_regular_field();
        assert_eq!(field.geometry.nearest_cell(100.0, 10.0), Some((0, 0)));
        assert_eq!(field.geometry.nearest_cell(115.0, 10.0), Some((0, 3)));
        assert_eq!(field.geometry.nearest_cell(100.0, 0.0), Some((2, 0)));
    }

    #[test]
    fn nearest_cell_returns_none_outside_a_regular_grid() {
        let field = sample_regular_field();
        assert_eq!(field.geometry.nearest_cell(50.0, 10.0), None);
        assert_eq!(field.geometry.nearest_cell(100.0, 50.0), None);
    }

    #[test]
    fn nearest_cell_finds_exact_cell_centers_on_a_lambert_grid() {
        let field = sample_lambert_field();
        // Cell (0,0) is exactly the projection's own known first point.
        assert_eq!(
            field.geometry.nearest_cell(-122.71953, 21.138123),
            Some((0, 0))
        );
        // Cell (0,1): one dx east in projected space.
        let (lon, lat) = field.geometry.lon_lat_for_cell(0, 1);
        assert_eq!(field.geometry.nearest_cell(lon, lat), Some((0, 1)));
        // Cell (1,0): one dy "north" in projected space.
        let (lon, lat) = field.geometry.lon_lat_for_cell(1, 0);
        assert_eq!(field.geometry.nearest_cell(lon, lat), Some((1, 0)));
    }

    #[test]
    fn nearest_cell_matches_real_gefs_geometry_north_pole() {
        // The real, empirically-verified GEFS grid: (row 0, col 0) is
        // exactly the North Pole.
        let geometry = GridGeometry::RegularLatLon(RegularLatLonGrid {
            width: 1440,
            height: 721,
            origin_lat_deg: 90.0,
            origin_lon_deg: 0.0,
            lat_step_deg: -0.25,
            lon_step_deg: 0.25,
        });
        assert_eq!(geometry.nearest_cell(0.0, 90.0), Some((0, 0)));
        assert_eq!(geometry.nearest_cell(0.0, -90.0), Some((720, 0)));
        assert_eq!(geometry.nearest_cell(0.0, 0.0), Some((360, 0)));
    }

    #[test]
    fn point_forecast_carries_variable_valid_time_and_value() {
        let field = sample_regular_field();
        let point = field.point_forecast(100.0, 10.0).unwrap();
        assert_eq!(point.variable, ForecastVariable::Temperature2m);
        assert_eq!(point.valid_time, field.valid_time);
        assert_eq!(point.value, 0.0);
        assert_eq!(point.unit, "K");
    }

    #[test]
    fn point_forecast_is_none_outside_the_grid() {
        let field = sample_regular_field();
        assert!(field.point_forecast(0.0, 0.0).is_none());
    }
}
