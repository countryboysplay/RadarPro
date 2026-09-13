//! Decode one fetched GRIB2 message (already sparse-fetched down to a
//! single field via [`crate::idx`]/[`crate::client`]) into a
//! `forecast_core::grid::ForecastGrid`.
//!
//! # HRRR's real grid: Lambert Conformal Conic, not GEFS's regular lat/lon
//!
//! Confirmed empirically (2026-09-12, live bucket, `hrrr.t12z.wrfsfcf00.grib2`,
//! `TMP:2 m above ground`): GRIB2 Grid Definition Template **3.30**
//! (Lambert Conformal Conic), `ni=1799, nj=1059` (the real, well-known HRRR
//! CONUS 3km grid size), `earth_shape=6` (spherical, radius 6,371,229.0 m),
//! `latin1=latin2=lad=38.5°N` (a tangent cone), `lov=-97.5°`, `Dx=Dy=3000 m`.
//! `grib` 0.18.5 decodes this template's grid geometry and values fully
//! with `default-features = false` -- no PROJ/C-toolchain dependency needed
//! (see `docs/adr/0012-hrrr-lambert-conformal-grid.md` for the full
//! empirical trail). This module derives a `forecast_core::grid::LambertConformalGrid`
//! from the template's own declared parameters, using
//! `forecast_core::projection::LccProjection` -- the from-scratch forward/
//! inverse projection that ADR justifies -- for the render-path direction
//! `grib` itself has no public API for (world position -> grid cell); the
//! decode-time direction (grid index -> lat/lon, used only to derive this
//! grid's own spacing below, never to place decoded values) still goes
//! through `grib`'s own built-in Lambert implementation.
//!
//! # Deriving grid spacing without hand-decoding scanning-mode bits
//!
//! GRIB2's `Dx`/`Dy` fields are unsigned magnitudes; their real sign (which
//! screen/array direction they actually step in) depends on the message's
//! scanning-mode byte (WMO Code Table 3.4). Rather than hand-decode that
//! byte's bit semantics (a real, if narrow, chance to introduce a silent
//! sign error), this module derives the *real*, already-scanning-mode-
//! correct spacing empirically from the same decoded message: it asks
//! `grib` for the real lat/lon of grid indices `(0,0)`, `(1,0)`, and
//! `(0,1)` (via `Template3_30::latlons()`, zipped positionally with
//! `submessage.ij()` -- both iterate in the same order, since `grib`'s own
//! `latlons_unchecked()` is implemented in terms of the same `ij()` call),
//! forward-projects all three with this module's own [`forecast_core::projection::LccProjection`],
//! and differences them. Cross-checked in this module's tests against a
//! real, live-captured HRRR message: the derived spacing matches the
//! message's own declared `Dx`/`Dy` (3000 m) to within `f32` rounding.
//!
//! # Cross-checking, not trusting, the filename/request
//!
//! Mirrors `provider-gefs::decode`'s own stance: parameter category/number
//! are read back from the decoded Product Definition Section and checked
//! against the field being requested, never assumed from the request
//! alone.
//!
//! # Untrusted input
//!
//! `bytes` is exactly the untrusted, network-sourced data GLOBAL_CONTRACT
//! describes: every fallible step below returns a structured [`HrrrError`],
//! never panics.

use crate::error::HrrrError;
use forecast_core::grid::{
    ForecastGrid, GridGeometry, LambertConformalGrid, NativeVariableMetadata,
};
use forecast_core::projection::{LccParams, LccProjection};
use forecast_core::time::UtcTimestamp;
use forecast_core::variable::ForecastVariable;
use grib::{Code::Name, GridDefinitionTemplateValues, LatLons};

/// GRIB2 Grid Definition Template 3.30 ("Lambert Conformal") -- the only
/// grid template every real HRRR `conus` surface message fetched during
/// this stage's verification used.
const GRID_TEMPLATE_LAMBERT_CONFORMAL: u16 = 30;

/// WMO Code Table 3.2 value `6`: "Earth assumed spherical with radius =
/// 6,371,229.0 m" -- the only earth shape every real HRRR message fetched
/// during this stage's verification used.
const EARTH_SHAPE_SPHERICAL_6371229: u8 = 6;

/// GRIB2 Product Definition Template 4.0 ("Analysis or forecast at a
/// horizontal level or in a horizontal layer at a point in time") -- the
/// deterministic template every real HRRR message fetched during this
/// stage's verification used (HRRR has no ensemble, so this crate never
/// expects 4.1/4.2 the way `provider-gefs` does).
const PROD_TEMPLATE_DETERMINISTIC: u16 = 0;

/// Fixed-point scale for template 3.30's angle fields (WMO Note: units of
/// 10^-6 degree, same convention GEFS's template 3.0 uses).
const COORD_SCALE: f64 = 1e-6;

/// This crate's own mapping from a canonical [`ForecastVariable`] to the
/// exact `.idx` `VARNAME`/`LEVEL` HRRR's real `.idx` files use for it.
/// Only `Temperature2m` is supported as of this stage (matching
/// `provider-gefs`'s own "avoid premature abstractions" precedent -- a
/// second decoded variable is future work, not this stage's).
pub fn idx_names(variable: ForecastVariable) -> Option<(&'static str, &'static str)> {
    match variable {
        ForecastVariable::Temperature2m => Some(("TMP", "2 m above ground")),
        _ => None,
    }
}

/// Decode `bytes` (the exact byte range of one GRIB2 message) into a
/// [`ForecastGrid`] for `variable`.
pub fn decode_field(
    url: &str,
    bytes: &[u8],
    variable: ForecastVariable,
) -> Result<ForecastGrid, HrrrError> {
    let (want_idx_variable, want_idx_level) =
        idx_names(variable).ok_or_else(|| HrrrError::UnexpectedField {
            url: url.to_string(),
            category: 0,
            number: 0,
            expected_field: variable.canonical_name().to_string(),
        })?;

    let grib2 = grib::from_bytes(bytes.to_vec()).map_err(|e| HrrrError::Grib2Parse {
        url: url.to_string(),
        message: e.to_string(),
    })?;

    let mut iter = grib2.iter();
    let (_, submessage) = iter.next().ok_or_else(|| HrrrError::NoGrib2Submessage {
        url: url.to_string(),
    })?;

    let prod_def = submessage.prod_def();
    let (want_category, want_number) = variable.grib2_parameter();
    let category = prod_def
        .parameter_category()
        .ok_or_else(|| HrrrError::Grib2Parse {
            url: url.to_string(),
            message: "missing parameter_category (unsupported Product Definition Template)"
                .to_string(),
        })?;
    let number = prod_def
        .parameter_number()
        .ok_or_else(|| HrrrError::Grib2Parse {
            url: url.to_string(),
            message: "missing parameter_number (unsupported Product Definition Template)"
                .to_string(),
        })?;
    if category != want_category || number != want_number {
        return Err(HrrrError::UnexpectedField {
            url: url.to_string(),
            category,
            number,
            expected_field: variable.canonical_name().to_string(),
        });
    }

    if prod_def.prod_tmpl_num() != PROD_TEMPLATE_DETERMINISTIC {
        return Err(HrrrError::UnsupportedProductTemplate {
            url: url.to_string(),
            template_number: prod_def.prod_tmpl_num(),
        });
    }

    let forecast_time = prod_def
        .forecast_time()
        .ok_or_else(|| HrrrError::Grib2Parse {
            url: url.to_string(),
            message: "missing forecast_time".to_string(),
        })?;
    let lead_hours = match forecast_time.unit {
        Name(grib::codetables::grib2::Table4_4::Hour) => forecast_time.value,
        other => {
            return Err(HrrrError::Grib2Parse {
                url: url.to_string(),
                message: format!(
                    "unsupported forecast time unit {other:?} -- every real HRRR message \
                     verified for this crate used whole hours"
                ),
            })
        }
    };

    let identification = submessage.identification();
    let dt = identification.ref_time_unchecked();
    let run_time = UtcTimestamp::new(
        i64::from(dt.year),
        u32::from(dt.month),
        u32::from(dt.day),
        u32::from(dt.hour),
        u32::from(dt.minute),
        u32::from(dt.second),
    );
    let valid_time = run_time.plus_hours(i64::from(lead_hours));

    let grid_def = submessage.grid_def();
    if grid_def.grid_tmpl_num() != GRID_TEMPLATE_LAMBERT_CONFORMAL {
        return Err(HrrrError::UnsupportedGridTemplate {
            url: url.to_string(),
            template_number: grid_def.grid_tmpl_num(),
        });
    }
    let template_values =
        GridDefinitionTemplateValues::try_from(grid_def).map_err(|e| HrrrError::Grib2Parse {
            url: url.to_string(),
            message: e.to_string(),
        })?;
    let GridDefinitionTemplateValues::Template30(template30) = &template_values else {
        return Err(HrrrError::UnsupportedGridTemplate {
            url: url.to_string(),
            template_number: grid_def.grid_tmpl_num(),
        });
    };

    if template30.earth_shape.shape != EARTH_SHAPE_SPHERICAL_6371229 {
        return Err(HrrrError::UnsupportedEarthShape {
            url: url.to_string(),
            shape: template30.earth_shape.shape,
        });
    }
    let earth_radius_m = forecast_core::projection::HRRR_EARTH_RADIUS_M;

    let ni = template30.ni;
    let nj = template30.nj;
    let projection = LccProjection::new(LccParams {
        earth_radius_m,
        standard_parallel_1_deg: f64::from(template30.latin1) * COORD_SCALE,
        standard_parallel_2_deg: f64::from(template30.latin2) * COORD_SCALE,
        latitude_of_origin_deg: f64::from(template30.lad) * COORD_SCALE,
        central_meridian_deg: f64::from(template30.lov) * COORD_SCALE,
    });

    // Grid index (0,0) is exactly this template's own declared first
    // point, by GRIB2 definition -- no need for `grib`'s `latlons()` for
    // this one.
    let first_lat = f64::from(template30.first_point_lat) * COORD_SCALE;
    let first_lon = f64::from(template30.first_point_lon) * COORD_SCALE;
    let (origin_x_m, origin_y_m) = projection.project(first_lon, first_lat);

    // Derive the real, scanning-mode-correct signed dx/dy empirically (see
    // this module's doc comment) rather than hand-decoding the scanning
    // mode byte.
    let ij_for_geometry = submessage.ij().map_err(|e| HrrrError::Grib2Parse {
        url: url.to_string(),
        message: e.to_string(),
    })?;
    let latlons = template30.latlons().map_err(|e| HrrrError::Grib2Parse {
        url: url.to_string(),
        message: e.to_string(),
    })?;
    let mut point_1_0: Option<(f32, f32)> = None;
    let mut point_0_1: Option<(f32, f32)> = None;
    for ((i, j), (lat, lon)) in ij_for_geometry.zip(latlons) {
        if i == 1 && j == 0 {
            point_1_0 = Some((lat, lon));
        }
        if i == 0 && j == 1 {
            point_0_1 = Some((lat, lon));
        }
        if point_1_0.is_some() && point_0_1.is_some() {
            break;
        }
    }
    let (lat10, lon10) = point_1_0.ok_or_else(|| HrrrError::MissingGeometryReferencePoint {
        url: url.to_string(),
    })?;
    let (lat01, lon01) = point_0_1.ok_or_else(|| HrrrError::MissingGeometryReferencePoint {
        url: url.to_string(),
    })?;
    let (x10, _) = projection.project(f64::from(lon10), f64::from(lat10));
    let (_, y01) = projection.project(f64::from(lon01), f64::from(lat01));
    let dx_m = x10 - origin_x_m;
    let dy_m = y01 - origin_y_m;

    let geometry = GridGeometry::LambertConformal(LambertConformalGrid {
        width: ni,
        height: nj,
        projection,
        origin_x_m,
        origin_y_m,
        dx_m,
        dy_m,
    });

    // Placing decoded values uses `grib`'s own `ij()` (a *fresh* call --
    // the one above was already consumed deriving geometry), exactly like
    // `provider-gefs::decode` does: correct regardless of the message's
    // actual scanning mode, since `grib` already resolved that.
    let ij_iter = submessage.ij().map_err(|e| HrrrError::Grib2Parse {
        url: url.to_string(),
        message: e.to_string(),
    })?;
    let decoder =
        grib::Grib2SubmessageDecoder::from(submessage).map_err(|e| HrrrError::Grib2Parse {
            url: url.to_string(),
            message: e.to_string(),
        })?;
    let decoded_values: Vec<f32> = decoder
        .dispatch()
        .map_err(|e| HrrrError::Grib2Parse {
            url: url.to_string(),
            message: e.to_string(),
        })?
        .collect();

    let expected = geometry.point_count();
    if decoded_values.len() != expected {
        return Err(HrrrError::GridSizeMismatch {
            url: url.to_string(),
            decoded: decoded_values.len(),
            expected,
            ni,
            nj,
        });
    }

    let mut values = vec![f32::NAN; expected];
    let mut assigned = vec![false; expected];
    for ((i, j), value) in ij_iter.zip(decoded_values) {
        if i >= ni as usize || j >= nj as usize {
            return Err(HrrrError::IncompleteGridCoverage {
                url: url.to_string(),
            });
        }
        let index = j * ni as usize + i;
        values[index] = value;
        assigned[index] = true;
    }
    if assigned.iter().any(|&a| !a) {
        return Err(HrrrError::IncompleteGridCoverage {
            url: url.to_string(),
        });
    }

    Ok(ForecastGrid {
        variable,
        native: NativeVariableMetadata {
            provider_variable_name: want_idx_variable.to_string(),
            provider_level_name: want_idx_level.to_string(),
            native_unit: "K",
        },
        provider_id: "hrrr",
        unit: "K",
        run_time,
        forecast_lead_hours: lead_hours,
        valid_time,
        // HRRR is deterministic -- confirmed empirically: every real
        // message decoded for this stage used Product Definition Template
        // 4.0, which carries no ensemble metadata at all (see this
        // module's doc comment and `forecast_core::ensemble`'s doc
        // comment for why this is `None`, not a faked identity).
        ensemble: None,
        geometry,
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/provider-hrrr")
            .join(name);
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("missing test fixture {}: {e}", path.display()))
    }

    #[test]
    fn decodes_a_real_captured_hrrr_message() {
        let bytes = fixture("hrrr_t12z_f00_tmp2m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::Temperature2m).unwrap();

        assert_eq!(decoded.variable, ForecastVariable::Temperature2m);
        assert_eq!(decoded.unit, "K");
        assert_eq!(decoded.provider_id, "hrrr");
        assert_eq!(decoded.ensemble, None, "HRRR is deterministic");
        assert_eq!(decoded.run_time, UtcTimestamp::new(2026, 9, 12, 12, 0, 0));
        assert_eq!(decoded.forecast_lead_hours, 0);
        assert_eq!(decoded.valid_time, decoded.run_time);
        assert_eq!(decoded.geometry.width(), 1799);
        assert_eq!(decoded.geometry.height(), 1059);
        assert_eq!(decoded.values.len(), 1799 * 1059);
        assert!(matches!(
            decoded.geometry,
            GridGeometry::LambertConformal(_)
        ));

        // Physical sanity + cross-reference: this crate's own earlier live
        // probe (see docs/adr/0012-...) printed min=265.5608 max=308.1233
        // for this exact message.
        let min = decoded.values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = decoded
            .values
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((min - 265.5608).abs() < 0.01, "min={min}");
        assert!((max - 308.1233).abs() < 0.01, "max={max}");
    }

    #[test]
    fn derived_grid_spacing_matches_hrrrs_real_declared_3km_dx_dy() {
        let bytes = fixture("hrrr_t12z_f00_tmp2m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::Temperature2m).unwrap();
        let GridGeometry::LambertConformal(grid) = &decoded.geometry else {
            panic!("expected a Lambert Conformal grid");
        };
        // Derived without ever reading the scanning-mode byte -- see this
        // module's doc comment -- but must still match HRRR's own
        // declared 3000 m spacing to within `f32` rounding of the
        // `grib`-provided lat/lon this derivation is based on (`grib`'s
        // public `latlons()` returns `f32`, whose ~7 significant digits at
        // these coordinate magnitudes cost up to a couple of meters here --
        // confirmed empirically, see this crate's ADR).
        assert!((grid.dx_m - 3000.0).abs() < 2.0, "dx_m={}", grid.dx_m);
        assert!((grid.dy_m - 3000.0).abs() < 2.0, "dy_m={}", grid.dy_m);
    }

    #[test]
    fn spot_check_the_first_grid_cell_against_grib2s_own_declared_first_point() {
        let bytes = fixture("hrrr_t12z_f00_tmp2m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::Temperature2m).unwrap();
        let (lon, lat) = decoded.geometry.lon_lat_for_cell(0, 0);
        // Real, live-verified first point for this exact message.
        assert!((lat - 21.138123).abs() < 1e-3, "lat={lat}");
        assert!((lon.rem_euclid(360.0) - (-122.71953f64).rem_euclid(360.0)).abs() < 1e-3);
    }

    #[test]
    fn rejects_garbage_bytes_cleanly() {
        let err = decode_field("u", &[0u8; 100], ForecastVariable::Temperature2m).unwrap_err();
        assert!(matches!(err, HrrrError::NoGrib2Submessage { .. }));
    }

    #[test]
    fn rejects_empty_bytes_cleanly() {
        let err = decode_field("u", &[], ForecastVariable::Temperature2m).unwrap_err();
        assert!(matches!(err, HrrrError::NoGrib2Submessage { .. }));
    }

    #[test]
    fn rejects_a_truncated_real_message_cleanly_never_panics() {
        let bytes = fixture("hrrr_t12z_f00_tmp2m.grib2");
        let truncated = &bytes[..bytes.len() / 2];
        let _ = decode_field("u", truncated, ForecastVariable::Temperature2m);
    }

    #[test]
    fn rejects_an_unsupported_variable_cleanly() {
        let bytes = fixture("hrrr_t12z_f00_tmp2m.grib2");
        let err = decode_field("u", &bytes, ForecastVariable::Dewpoint2m).unwrap_err();
        assert!(matches!(err, HrrrError::UnexpectedField { .. }));
    }
}
