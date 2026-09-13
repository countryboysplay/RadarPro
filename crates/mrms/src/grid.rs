//! This crate's decoded-grid type -- deliberately *not*
//! `forecast_core::grid::ForecastGrid`: MRMS is an observation product with
//! no run/lead-time/ensemble concept at all (GLOBAL_CONTRACT: "never call
//! model precipitation 'future radar'" implies the converse just as much --
//! never dress an observation up in forecast-shaped metadata it doesn't
//! have). [`MrmsGrid`] carries exactly one real-world timestamp
//! (`valid_time`, the instant the snapshot was observed) and reuses
//! [`forecast_core::grid::GridGeometry`] for its shared, provider-agnostic
//! regular-lat-lon geometry -- see `docs/adr/0014-mrms-grib2-png-unpack-and-local-discipline.md`.
//!
//! # Missing values are a first-class, explicit per-cell state
//!
//! Confirmed empirically against real decoded arrays (see `crate::decode`'s
//! module doc and the ADR): MRMS fills two structurally different
//! situations with sentinel numbers rather than omitting the cell --
//! "outside the radar mosaic's domain entirely" and, for reflectivity only,
//! "inside the domain but no significant echo detected". Neither is a real
//! physical dBZ/mm-per-hour reading, so [`MrmsCellValue`] makes both
//! explicit enum variants (mirroring `radar_types::GateValue`'s existing
//! `Missing`/`RangeFolded`/`Value(f32)` shape for NEXRAD) rather than
//! leaving a raw sentinel float for every downstream consumer to
//! rediscover and risk silently plotting as a real value.

use forecast_core::grid::GridGeometry;
use forecast_core::time::UtcTimestamp;

use crate::keys::MrmsProduct;

/// One decoded MRMS grid cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MrmsCellValue {
    /// This cell falls entirely outside the radar mosaic's domain (no
    /// contributing radar reaches it at all) -- confirmed empirically by
    /// this sentinel's geography exactly tracing the CONUS network's ocean-
    /// side domain edges (see the ADR). Distinct from [`Self::Missing`].
    NoCoverage,
    /// This cell has real radar coverage, but no significant return was
    /// recorded there (reflectivity only -- confirmed empirically: this
    /// sentinel fills the CONUS interior wherever there is currently no
    /// echo, never the out-of-network ocean areas [`Self::NoCoverage`]
    /// fills). Never produced for `PrecipRate`, whose "no precipitation"
    /// reading is a genuine physical `0.0` [`Self::Value`], not a sentinel.
    Missing,
    /// A real, physically meaningful reading, in the product's native unit
    /// ([`MrmsProduct::native_unit`]).
    Value(f32),
}

impl MrmsCellValue {
    /// The real physical value, or `None` for [`Self::NoCoverage`]/
    /// [`Self::Missing`] -- the one place a caller that wants a plain
    /// `Option<f32>` (rather than matching the full enum) can get one,
    /// without ever being able to mistake a sentinel for a real `0.0` or
    /// negative reading.
    pub fn value(&self) -> Option<f32> {
        match self {
            MrmsCellValue::Value(v) => Some(*v),
            _ => None,
        }
    }
}

/// Summary statistics over one [`MrmsGrid`]'s cells -- computed only from
/// [`MrmsCellValue::Value`] cells, per GLOBAL_CONTRACT's "preserve missing
/// values" (a min/max/mean that silently folded in `-999`/`-99` sentinels
/// would be scientifically meaningless).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MrmsGridStats {
    pub min: f32,
    pub max: f32,
    pub mean: f32,
    pub valid_count: usize,
    pub missing_count: usize,
    pub no_coverage_count: usize,
}

/// One fully decoded MRMS observation snapshot: metadata plus a row-major
/// grid of [`MrmsCellValue`]s.
#[derive(Debug, Clone, PartialEq)]
pub struct MrmsGrid {
    pub product: MrmsProduct,
    /// Native physical unit of every [`MrmsCellValue::Value`] in `values`
    /// (`"dBZ"`/`"mm/hr"` -- see [`MrmsProduct::native_unit`]).
    pub unit: &'static str,
    /// The real-world instant this snapshot observes -- MRMS's *only*
    /// timestamp (no run/lead-time pair; see this module's doc comment).
    pub valid_time: UtcTimestamp,
    pub geometry: GridGeometry,
    /// Row-major, `values[row * geometry.width() + col]`, length always
    /// `geometry.point_count()`.
    pub values: Vec<MrmsCellValue>,
}

impl MrmsGrid {
    pub fn value_at(&self, row: u32, col: u32) -> MrmsCellValue {
        self.values[row as usize * self.geometry.width() as usize + col as usize]
    }

    /// Compute [`MrmsGridStats`] over this grid -- see that type's doc
    /// comment for why sentinels are excluded from min/max/mean.
    pub fn stats(&self) -> MrmsGridStats {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        let mut sum = 0.0_f64;
        let mut valid_count = 0usize;
        let mut missing_count = 0usize;
        let mut no_coverage_count = 0usize;
        for cell in &self.values {
            match cell {
                MrmsCellValue::Value(v) => {
                    min = min.min(*v);
                    max = max.max(*v);
                    sum += f64::from(*v);
                    valid_count += 1;
                }
                MrmsCellValue::Missing => missing_count += 1,
                MrmsCellValue::NoCoverage => no_coverage_count += 1,
            }
        }
        let mean = if valid_count > 0 {
            (sum / valid_count as f64) as f32
        } else {
            f32::NAN
        };
        MrmsGridStats {
            min,
            max,
            mean,
            valid_count,
            missing_count,
            no_coverage_count,
        }
    }

    /// Build a plain `Vec<f32>` suitable for
    /// `forecast_core::gpu::render_forecast_grid`'s `display_values`
    /// parameter: every [`MrmsCellValue::Value`] passes through unchanged,
    /// and every [`MrmsCellValue::NoCoverage`]/[`MrmsCellValue::Missing`]
    /// cell becomes `fill_value` -- the caller's job is to choose a
    /// `fill_value` that renders as transparent in whatever palette LUT/
    /// domain it pairs with this (e.g. a value at or below the palette
    /// domain's minimum, which every palette in this codebase already
    /// clamps to its first, fully-transparent stop -- see
    /// `radar_render::palette::sample_stops`). This function itself makes
    /// no assumption about what "transparent" means for a given palette;
    /// it only guarantees a sentinel numeric value never reaches the GPU
    /// disguised as a real reading.
    pub fn display_values(&self, fill_value: f32) -> Vec<f32> {
        self.values
            .iter()
            .map(|cell| match cell {
                MrmsCellValue::Value(v) => *v,
                MrmsCellValue::NoCoverage | MrmsCellValue::Missing => fill_value,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forecast_core::grid::RegularLatLonGrid;

    fn sample_grid() -> MrmsGrid {
        let geometry = GridGeometry::RegularLatLon(RegularLatLonGrid {
            width: 2,
            height: 2,
            origin_lat_deg: 55.0,
            origin_lon_deg: 230.0,
            lat_step_deg: -0.01,
            lon_step_deg: 0.01,
        });
        MrmsGrid {
            product: MrmsProduct::ReflectivityQcComposite,
            unit: "dBZ",
            valid_time: UtcTimestamp::new(2026, 9, 13, 5, 4, 38),
            geometry,
            values: vec![
                MrmsCellValue::Value(20.0),
                MrmsCellValue::NoCoverage,
                MrmsCellValue::Missing,
                MrmsCellValue::Value(40.0),
            ],
        }
    }

    #[test]
    fn stats_excludes_sentinels_from_min_max_mean() {
        let grid = sample_grid();
        let stats = grid.stats();
        assert_eq!(stats.min, 20.0);
        assert_eq!(stats.max, 40.0);
        assert_eq!(stats.mean, 30.0);
        assert_eq!(stats.valid_count, 2);
        assert_eq!(stats.missing_count, 1);
        assert_eq!(stats.no_coverage_count, 1);
    }

    #[test]
    fn display_values_substitutes_fill_value_for_sentinels_only() {
        let grid = sample_grid();
        let display = grid.display_values(-999.0);
        assert_eq!(display, vec![20.0, -999.0, -999.0, 40.0]);
    }

    #[test]
    fn value_helper_is_none_for_sentinels() {
        assert_eq!(MrmsCellValue::Value(5.0).value(), Some(5.0));
        assert_eq!(MrmsCellValue::Missing.value(), None);
        assert_eq!(MrmsCellValue::NoCoverage.value(), None);
    }

    #[test]
    fn stats_of_all_sentinel_grid_has_nan_mean_and_zero_counts() {
        let mut grid = sample_grid();
        grid.values = vec![MrmsCellValue::NoCoverage; 4];
        let stats = grid.stats();
        assert_eq!(stats.valid_count, 0);
        assert_eq!(stats.no_coverage_count, 4);
        assert!(stats.mean.is_nan());
    }
}
