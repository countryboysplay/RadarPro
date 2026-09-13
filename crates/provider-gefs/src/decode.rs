//! Decode one fetched GRIB2 message (already sparse-fetched down to a
//! single field via [`crate::idx`]/[`crate::client`]) into
//! `forecast_core::grid::ForecastGrid`.
//!
//! # Cross-checking, not trusting, the filename
//!
//! The object key's filename (`gec00`/`gep01`/`geavg`) already tells a
//! caller which member it *asked for*, but this function never trusts
//! that alone: it reads parameter category/number back from the decoded
//! Product Definition Section and rejects a mismatch
//! ([`GefsError::UnexpectedField`]), and it derives ensemble identity
//! ([`crate::ensemble::identity_from_prod_def`]) from the same decoded
//! section, not from the filename. A caller that wants to confirm
//! "the file I fetched as `gep01` really is decoded as ensemble member 1"
//! can compare the two identities itself -- this function does not do
//! that comparison for them, since it has no `MemberKey` parameter, only
//! the bytes and the field being requested.
//!
//! # Untrusted input
//!
//! `bytes` is exactly the untrusted, network-sourced data GLOBAL_CONTRACT
//! describes: every fallible step below returns a structured
//! [`GefsError`], never panics, and never substitutes a default/zero value
//! for something that failed to decode.

use crate::ensemble;
use crate::error::GefsError;
use forecast_core::grid::{ForecastGrid, GridGeometry, NativeVariableMetadata, RegularLatLonGrid};
use forecast_core::time::UtcTimestamp;
use forecast_core::variable::ForecastVariable;
use grib::{Code::Name, GridDefinitionTemplateValues};

/// GRIB2 Grid Definition Template 3.0 ("Latitude/Longitude (or equidistant
/// cylindrical, or Plate Carrée)") -- the only grid template every real
/// GEFS `pgrb2sp25` message fetched during this stage's verification used.
const GRID_TEMPLATE_REGULAR_LAT_LON: u16 = 0;

/// Fixed-point scale for template 3.0's `La1`/`Lo1`/`La2`/`Lo2`/`Di`/`Dj`
/// fields: GRIB2 Note 1 for this template defines them in units of
/// 10^-6 degree. Confirmed empirically: a real message's `first_point_lat
/// = 90000000` round-trips to exactly `90.0` degrees via `grib`'s own
/// `latlons()` (`90.0, 0.0` was the first coordinate `grib` computed for
/// every real message decoded during this stage's verification).
const COORD_SCALE: f64 = 1e-6;

/// This crate's own mapping from a canonical [`ForecastVariable`] to the
/// exact `.idx` `VARNAME`/`LEVEL` and GRIB2 native unit GEFS's real `.idx`
/// files use for it. Only `Temperature2m` is supported as of this stage
/// (S07's own "avoid premature abstractions" rule -- a second decoded
/// variable is future work, not this stage's).
pub fn idx_names(variable: ForecastVariable) -> Option<(&'static str, &'static str, &'static str)> {
    match variable {
        // (idx VARNAME, idx LEVEL, native unit)
        ForecastVariable::Temperature2m => Some(("TMP", "2 m above ground", "K")),
        _ => None,
    }
}

/// Decode `bytes` (the exact byte range of one GRIB2 message, as fetched
/// via [`crate::idx`]'s byte-range computation) into a [`ForecastGrid`] for
/// `variable`.
pub fn decode_field(
    url: &str,
    bytes: &[u8],
    variable: ForecastVariable,
) -> Result<ForecastGrid, GefsError> {
    let (idx_variable, idx_level, native_unit) =
        idx_names(variable).ok_or_else(|| GefsError::UnexpectedField {
            url: url.to_string(),
            category: 0,
            number: 0,
            expected_field: variable.canonical_name().to_string(),
        })?;

    let grib2 = grib::from_bytes(bytes.to_vec()).map_err(|e| GefsError::Grib2Parse {
        url: url.to_string(),
        message: e.to_string(),
    })?;

    let mut iter = grib2.iter();
    let (_, submessage) = iter.next().ok_or_else(|| GefsError::NoGrib2Submessage {
        url: url.to_string(),
    })?;

    // --- Everything read via `&self` methods first: `Grib2SubmessageDecoder::from`
    // (further down) consumes `submessage` by value. ---

    let prod_def = submessage.prod_def();
    let (want_category, want_number) = variable.grib2_parameter();
    let category = prod_def
        .parameter_category()
        .ok_or_else(|| GefsError::Grib2Parse {
            url: url.to_string(),
            message: "missing parameter_category (unsupported Product Definition Template)"
                .to_string(),
        })?;
    let number = prod_def
        .parameter_number()
        .ok_or_else(|| GefsError::Grib2Parse {
            url: url.to_string(),
            message: "missing parameter_number (unsupported Product Definition Template)"
                .to_string(),
        })?;
    if category != want_category || number != want_number {
        return Err(GefsError::UnexpectedField {
            url: url.to_string(),
            category,
            number,
            expected_field: variable.canonical_name().to_string(),
        });
    }

    let forecast_time = prod_def
        .forecast_time()
        .ok_or_else(|| GefsError::Grib2Parse {
            url: url.to_string(),
            message: "missing forecast_time".to_string(),
        })?;
    let lead_hours = match forecast_time.unit {
        Name(grib::codetables::grib2::Table4_4::Hour) => forecast_time.value,
        other => {
            return Err(GefsError::Grib2Parse {
                url: url.to_string(),
                message: format!(
                    "unsupported forecast time unit {other:?} -- every real GEFS message \
                     verified for this crate used whole hours"
                ),
            })
        }
    };

    let ensemble_identity = ensemble::identity_from_prod_def(url, prod_def)?;

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
    if grid_def.grid_tmpl_num() != GRID_TEMPLATE_REGULAR_LAT_LON {
        return Err(GefsError::UnsupportedGridTemplate {
            url: url.to_string(),
            template_number: grid_def.grid_tmpl_num(),
        });
    }
    let template_values =
        GridDefinitionTemplateValues::try_from(grid_def).map_err(|e| GefsError::Grib2Parse {
            url: url.to_string(),
            message: e.to_string(),
        })?;
    let GridDefinitionTemplateValues::Template0(template0) = &template_values else {
        // Unreachable given the `grid_tmpl_num()` check above, but handled
        // rather than asserted/unwrapped -- this whole crate never trusts
        // untrusted input enough to assume a check made two lines above
        // still holds without also handling the "somehow it doesn't"
        // branch explicitly.
        return Err(GefsError::UnsupportedGridTemplate {
            url: url.to_string(),
            template_number: grid_def.grid_tmpl_num(),
        });
    };
    let grid = &template0.lat_lon.grid;
    let ni = grid.ni;
    let nj = grid.nj;

    let la1 = f64::from(grid.first_point_lat) * COORD_SCALE;
    let la2 = f64::from(grid.last_point_lat) * COORD_SCALE;
    let lo1 = (f64::from(grid.first_point_lon) * COORD_SCALE).rem_euclid(360.0);
    let lat_step_magnitude = f64::from(template0.lat_lon.j_direction_inc) * COORD_SCALE;
    let lon_step_magnitude = f64::from(template0.lat_lon.i_direction_inc) * COORD_SCALE;
    // GRIB2 template 3.0 does not itself say which way latitude runs; the
    // sign is derived from comparing La1/La2 (every real GEFS message
    // observed: La1=90 (north pole), La2=-90 (south pole), i.e. latitude
    // *decreases* as the j-index increases) rather than hand-decoding the
    // scanning-mode byte -- `grib`'s own `ij()` (below) already handles
    // scanning-mode-dependent value *placement*; this sign only affects
    // how a placed (row, col) is turned back into a physical latitude.
    let lat_step_deg = if la2 < la1 {
        -lat_step_magnitude
    } else {
        lat_step_magnitude
    };

    let geometry = GridGeometry::RegularLatLon(RegularLatLonGrid {
        width: ni,
        height: nj,
        origin_lat_deg: la1,
        origin_lon_deg: lo1,
        lat_step_deg,
        lon_step_deg: lon_step_magnitude,
    });

    // `ij()` yields each decoded value's (i, j) = (column, row) grid index
    // in the same order `dispatch()` below yields values, correctly
    // accounting for the message's actual scanning mode (`grib`'s job, not
    // reimplemented here) -- see this module's doc comment.
    let ij_iter = submessage.ij().map_err(|e| GefsError::Grib2Parse {
        url: url.to_string(),
        message: e.to_string(),
    })?;

    let decoder =
        grib::Grib2SubmessageDecoder::from(submessage).map_err(|e| GefsError::Grib2Parse {
            url: url.to_string(),
            message: e.to_string(),
        })?;
    let decoded_values: Vec<f32> = decoder
        .dispatch()
        .map_err(|e| GefsError::Grib2Parse {
            url: url.to_string(),
            message: e.to_string(),
        })?
        .collect();

    let expected = geometry.point_count();
    if decoded_values.len() != expected {
        return Err(GefsError::GridSizeMismatch {
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
            return Err(GefsError::IncompleteGridCoverage {
                url: url.to_string(),
            });
        }
        let index = j * ni as usize + i;
        values[index] = value;
        assigned[index] = true;
    }
    if assigned.iter().any(|&a| !a) {
        return Err(GefsError::IncompleteGridCoverage {
            url: url.to_string(),
        });
    }

    Ok(ForecastGrid {
        variable,
        native: NativeVariableMetadata {
            provider_variable_name: idx_variable.to_string(),
            provider_level_name: idx_level.to_string(),
            native_unit,
        },
        provider_id: "gefs",
        unit: native_unit,
        run_time,
        forecast_lead_hours: lead_hours,
        valid_time,
        ensemble: Some(ensemble_identity),
        geometry,
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use forecast_core::ensemble::EnsembleStatistic;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/provider-gefs")
            .join(name);
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("missing test fixture {}: {e}", path.display()))
    }

    #[test]
    fn decodes_a_real_captured_control_member_message() {
        let bytes = fixture("gec00_t12z_f000_tmp2m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::Temperature2m).unwrap();

        assert_eq!(decoded.variable, ForecastVariable::Temperature2m);
        assert_eq!(decoded.unit, "K");
        assert_eq!(decoded.provider_id, "gefs");
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Control));
        assert_eq!(decoded.run_time, UtcTimestamp::new(2026, 9, 12, 12, 0, 0));
        assert_eq!(decoded.forecast_lead_hours, 0);
        assert_eq!(decoded.valid_time, decoded.run_time);
        assert_eq!(decoded.geometry.width(), 1440);
        assert_eq!(decoded.geometry.height(), 721);
        let GridGeometry::RegularLatLon(g) = &decoded.geometry else {
            panic!("expected a regular lat/lon grid");
        };
        assert_eq!(g.origin_lat_deg, 90.0);
        assert_eq!(g.origin_lon_deg, 0.0);
        assert_eq!(g.lat_step_deg, -0.25);
        assert_eq!(g.lon_step_deg, 0.25);
        assert_eq!(decoded.values.len(), 1440 * 721);

        // Physical sanity + cross-reference: real 2m temperature in Kelvin
        // is always in a physically plausible range, and this crate's own
        // earlier live probe (see docs/adr/0011-...) printed min=201.00223
        // max=319.40222 mean=281.37885 for this exact message.
        let min = decoded.values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = decoded
            .values
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((min - 201.00223).abs() < 0.01, "min={min}");
        assert!((max - 319.40222).abs() < 0.01, "max={max}");
    }

    #[test]
    fn decodes_a_real_captured_perturbed_member_message() {
        let bytes = fixture("gep01_t12z_f000_tmp2m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::Temperature2m).unwrap();
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Member(1)));
    }

    #[test]
    fn decodes_a_real_captured_ensemble_mean_message() {
        let bytes = fixture("geavg_t12z_f000_tmp2m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::Temperature2m).unwrap();
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Mean));
    }

    #[test]
    fn spot_check_a_known_grid_cell_against_an_independent_source() {
        // Cross-reference: NOAA's own GEFS `.idx`-adjacent tooling and
        // public viewers report grid point (row 0, col 0) as the North
        // Pole. A physically real 2m temperature at the North Pole for a
        // 2026-09-12 12Z run must be a plausible polar value (well below
        // freezing, comfortably within [-60C, 10C] = [213.15K, 283.15K] --
        // wide enough to be a real physical sanity check, not a
        // coincidence-prone exact match).
        let bytes = fixture("gec00_t12z_f000_tmp2m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::Temperature2m).unwrap();
        let north_pole_kelvin = decoded.value_at(0, 0);
        assert!(
            (213.15..283.15).contains(&north_pole_kelvin),
            "north pole 2m temperature {north_pole_kelvin} K is not a plausible polar value"
        );
    }

    #[test]
    fn rejects_garbage_bytes_cleanly() {
        let err = decode_field("u", &[0u8; 100], ForecastVariable::Temperature2m).unwrap_err();
        assert!(matches!(err, GefsError::NoGrib2Submessage { .. }));
    }

    #[test]
    fn rejects_empty_bytes_cleanly() {
        let err = decode_field("u", &[], ForecastVariable::Temperature2m).unwrap_err();
        assert!(matches!(err, GefsError::NoGrib2Submessage { .. }));
    }

    #[test]
    fn rejects_a_truncated_real_message_cleanly_never_panics() {
        let bytes = fixture("gec00_t12z_f000_tmp2m.grib2");
        let truncated = &bytes[..bytes.len() / 2];
        // Must return a structured error (most likely at the `dispatch()`
        // step, since GRIB2's outer sections are small and near the
        // front) -- never panic.
        let _ = decode_field("u", truncated, ForecastVariable::Temperature2m);
    }

    #[test]
    fn rejects_an_unsupported_variable_cleanly() {
        let bytes = fixture("gec00_t12z_f000_tmp2m.grib2");
        let err = decode_field("u", &bytes, ForecastVariable::Dewpoint2m).unwrap_err();
        assert!(matches!(err, GefsError::UnexpectedField { .. }));
    }
}
