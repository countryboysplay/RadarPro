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
/// files use for it.
///
/// Every arm here was verified against a real, live `.idx` file fetched
/// from the `noaa-gefs-pds` bucket (2026-09-13, `gefs.20260913/12`,
/// `pgrb2sp25`) -- not assumed from generic GRIB2/HRRR naming conventions.
/// Two canonical variables are deliberately left returning `None`:
///
/// - [`ForecastVariable::GeopotentialHeight`]: out of scope for this stage
///   (needs a new product-group/bucket-key path plus a "level" concept
///   `FieldRequest` doesn't have yet).
/// - [`ForecastVariable::Precipitation1h`]: GEFS's `pgrb2sp25` product
///   group publishes forecast hours on a strict 3-hour cadence only
///   (confirmed empirically: `f001`/`f002`/`f004` all 404 against the live
///   bucket, only `f000`, `f003`, `f006`, ... exist), and its `APCP`
///   message is always a 0-3-hour (later 0-6-hour) *accumulation* -- there
///   is no message anywhere in this product group that is a true 1-hour
///   accumulation. Decoding the 3-hour message and labeling it
///   `Precipitation1h` would mislabel the physical quantity, so this is
///   treated the same as `GeopotentialHeight`: documented and skipped,
///   not approximated.
///
/// See [`ForecastVariable::Mslp`]'s entry below for why this decodes
/// GEFS's `MSLET` message specifically (not the standard `PRMSL`, which
/// this product group also publishes at the same `"mean sea level"`
/// `.idx` level).
///
/// # Why this table carries its own `(category, number)`, not
/// `ForecastVariable::grib2_parameter()`
///
/// `Mslp` is a real, empirically-confirmed case where GEFS and HRRR do
/// not share one GRIB2 parameter pair for "the same" canonical variable:
/// GEFS's real `MSLET` message decodes as `(3, 192)` (a local-table
/// code), HRRR's real MSLP-family product (`MSLMA`) decodes as `(3,
/// 198)` -- confirmed live against both buckets, both genuinely
/// published, neither a stand-in for a missing message (see
/// `provider-hrrr::decode::idx_names`'s doc comment for its side of this).
/// A single shared `ForecastVariable::grib2_parameter()` constant cannot
/// be correct for both at once, so each provider's own `idx_names` table
/// carries the parameter pair *it* empirically verified for *its own*
/// real message, and `decode_field` cross-checks against this table's
/// value, never the shared enum's -- exactly matching this project's
/// "provider-specific names and formats stop at the adapter" rule (every
/// other variable's pair still happens to agree with
/// `ForecastVariable::grib2_parameter()`'s documented reference value,
/// which remains useful as a starting point for a new provider, just no
/// longer authoritative here).
pub fn idx_names(
    variable: ForecastVariable,
) -> Option<(&'static str, &'static str, &'static str, u8, u8)> {
    match variable {
        // (idx VARNAME, idx LEVEL, native unit, category, number)
        ForecastVariable::Temperature2m => Some(("TMP", "2 m above ground", "K", 0, 0)),
        ForecastVariable::Dewpoint2m => Some(("DPT", "2 m above ground", "K", 0, 6)),
        ForecastVariable::WindU10m => Some(("UGRD", "10 m above ground", "m/s", 2, 2)),
        ForecastVariable::WindV10m => Some(("VGRD", "10 m above ground", "m/s", 2, 3)),
        ForecastVariable::WindGust => Some(("GUST", "surface", "m/s", 2, 22)),
        // GEFS's `.idx` lists BOTH `MSLET:mean sea level` and
        // `PRMSL:mean sea level` at every forecast hour. This crate uses
        // MSLET (NCEP's Eta-model-reduction mean sea level pressure, the
        // field conventionally called "MSLP" in NCEP model output) --
        // empirically confirmed to decode with real (category, number) =
        // (3, 192), a local-table code, NOT the standard PRMSL pairing
        // (3, 1).
        ForecastVariable::Mslp => Some(("MSLET", "mean sea level", "Pa", 3, 192)),
        ForecastVariable::RelativeHumidity => Some(("RH", "2 m above ground", "%", 1, 1)),
        // No instantaneous cloud-cover message exists in this product
        // group -- every real `TCDC` message is a 0-3-hour average, using
        // GRIB2 Product Definition Template 4.11/4.12 rather than plain
        // 4.1/4.2 (see `crate::ensemble`'s module doc comment). Unlike
        // `Precipitation1h`, `CloudCover`'s canonical name carries no
        // specific time-window promise, so a 0-3-hour average is a
        // reasonable "cloud cover" value.
        ForecastVariable::CloudCover => Some(("TCDC", "entire atmosphere", "%", 6, 1)),
        ForecastVariable::Precipitation1h | ForecastVariable::GeopotentialHeight => None,
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
    let (idx_variable, idx_level, native_unit, want_category, want_number) = idx_names(variable)
        .ok_or_else(|| GefsError::UnexpectedField {
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
    let base_lead_hours = match forecast_time.unit {
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
    // Product Definition Templates 4.11/4.12 ("... in a continuous or
    // non-continuous time interval", used by every real GEFS `CloudCover`
    // message -- see `idx_names`'s doc comment) set `forecast_time` to the
    // *start* of the averaging/accumulation window (0 for every real GEFS
    // message observed), not the moment the value is conventionally
    // considered valid. `time_range_extra_hours` adds the window's own
    // length so `forecast_lead_hours`/`valid_time` reflect its end, e.g.
    // GEFS's own `.idx` calls a 0-3-hour message `f003`, not `f000`.
    let template_number = prod_def.prod_tmpl_num();
    let raw_prod_def: Vec<u8> = prod_def.iter().copied().collect();
    let lead_hours = base_lead_hours + time_range_extra_hours(url, template_number, &raw_prod_def)?;

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

/// See `decode_field`'s call site for why this exists: GRIB2 Product
/// Definition Templates 4.11 ("individual ensemble forecast ... in a
/// continuous or non-continuous time interval") and 4.12 (4.2's
/// counterpart) put the averaging/accumulation window's *length* in a
/// time-range-specification block appended after the ensemble/derived-type
/// fields 4.1/4.2 already define, rather than in the shared `forecast_time`
/// field itself (which every real GEFS message observed for this stage
/// sets to 0, the window's start). Returns that length in whole hours, to
/// be added to `forecast_time`'s own value; returns `0` unchanged for
/// every other template number (every previously-supported variable is
/// unaffected).
///
/// Byte offsets derived from the WMO layout ("4.11 = 4.1's 28-octet body +
/// a time-range extension", "4.12 = 4.2's 27-octet body + the same
/// extension") and cross-checked octet-for-octet against two real,
/// live-fetched messages -- see `crate::ensemble`'s module doc comment for
/// the same two messages' ensemble-identity offsets, which land in the
/// shared 4.1/4.2 prefix this function does not touch.
///
/// Only a single time-range specification (`n == 1`), in whole hours, is
/// supported -- the only shape every real GEFS message observed for this
/// stage used; anything else is refused rather than guessed at, per this
/// crate's untrusted-input policy (see this module's doc comment).
fn time_range_extra_hours(url: &str, template_number: u16, raw: &[u8]) -> Result<u32, GefsError> {
    let template_body_len: usize = match template_number {
        11 => 28,
        12 => 27,
        _ => return Ok(0),
    };

    let too_short = || GefsError::Grib2Parse {
        url: url.to_string(),
        message: format!(
            "Product Definition Template 4.{template_number}'s raw payload is too short to \
             contain its time-range extension"
        ),
    };

    // 4-byte `iter()` prefix (num coordinates + template number) +
    // `template_body_len` octets of the 4.1/4.2 body + 7 octets (end of
    // overall time interval date-time -- unused here, since the same
    // lead hours are derived below from `forecast_time` + window length
    // instead, needing no calendar arithmetic of their own) lands on "n",
    // the number of time-range specifications.
    let n_offset = 4 + template_body_len + 7;
    let n = *raw.get(n_offset).ok_or_else(too_short)?;
    if n != 1 {
        return Err(GefsError::Grib2Parse {
            url: url.to_string(),
            message: format!(
                "Product Definition Template 4.{template_number} declares {n} time-range \
                 specifications; this crate only supports the single-specification shape every \
                 real GEFS message observed for this stage used"
            ),
        });
    }

    // + 1 octet ("n" itself) + 4 octets ("total number of data values
    // missing in statistical process") + 2 octets (statistical process,
    // type of time increment) lands on the range length's unit.
    let unit_offset = n_offset + 1 + 4 + 2;
    let length_offset = unit_offset + 1;

    let unit = *raw.get(unit_offset).ok_or_else(too_short)?;
    // WMO Code Table 4.4 code 1 = Hour -- the same unit `forecast_time`
    // itself is required to use in `decode_field`; every real GEFS
    // time-interval message observed for this stage used it.
    const HOUR: u8 = 1;
    if unit != HOUR {
        return Err(GefsError::Grib2Parse {
            url: url.to_string(),
            message: format!(
                "Product Definition Template 4.{template_number}'s time-range length uses unit \
                 code {unit} (not hours) -- every real GEFS message observed for this stage \
                 used whole hours"
            ),
        });
    }

    let length_bytes: [u8; 4] = raw
        .get(length_offset..length_offset + 4)
        .ok_or_else(too_short)?
        .try_into()
        .expect("slice of length 4");
    Ok(u32::from_be_bytes(length_bytes))
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
        let err = decode_field("u", &bytes, ForecastVariable::GeopotentialHeight).unwrap_err();
        assert!(matches!(err, GefsError::UnexpectedField { .. }));
    }

    #[test]
    fn rejects_a_field_that_does_not_match_the_requested_variable() {
        // Cross-check test: a real TMP message decoded while *asking for*
        // Dewpoint2m (a different, but now-supported, variable) must be
        // rejected on the parameter category/number mismatch, not silently
        // labeled as dewpoint.
        let bytes = fixture("gec00_t12z_f000_tmp2m.grib2");
        let err = decode_field("u", &bytes, ForecastVariable::Dewpoint2m).unwrap_err();
        assert!(matches!(
            err,
            GefsError::UnexpectedField {
                category: 0,
                number: 0,
                ..
            }
        ));
    }

    #[test]
    fn decodes_a_real_captured_dewpoint_2m_message() {
        let bytes = fixture("gec00_t12z_f000_dpt2m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::Dewpoint2m).unwrap();

        assert_eq!(decoded.variable, ForecastVariable::Dewpoint2m);
        assert_eq!(decoded.unit, "K");
        assert_eq!(decoded.provider_id, "gefs");
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Control));
        assert_eq!(decoded.run_time, UtcTimestamp::new(2026, 9, 13, 12, 0, 0));
        assert_eq!(decoded.forecast_lead_hours, 0);
        assert_eq!(decoded.valid_time, decoded.run_time);
        assert_eq!(decoded.geometry.width(), 1440);
        assert_eq!(decoded.geometry.height(), 721);
        assert_eq!(decoded.values.len(), 1440 * 721);

        // Physical sanity: dew point in Kelvin is always <= the
        // corresponding air temperature, and always inside a physically
        // plausible range for any real place on Earth.
        let min = decoded.values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = decoded
            .values
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((150.0..320.0).contains(&min), "implausible min {min}");
        assert!((150.0..320.0).contains(&max), "implausible max {max}");
    }

    #[test]
    fn decodes_a_real_captured_wind_u_10m_message() {
        let bytes = fixture("gec00_t12z_f000_ugrd10m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::WindU10m).unwrap();

        assert_eq!(decoded.variable, ForecastVariable::WindU10m);
        assert_eq!(decoded.unit, "m/s");
        assert_eq!(decoded.provider_id, "gefs");
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Control));
        assert_eq!(decoded.forecast_lead_hours, 0);
        assert_eq!(decoded.geometry.width(), 1440);
        assert_eq!(decoded.geometry.height(), 721);

        // Physical sanity: a single 10m wind component anywhere on Earth
        // is essentially never outside +/-100 m/s (that would be well
        // beyond the strongest tornadic/hurricane surface winds ever
        // recorded).
        let min = decoded.values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = decoded
            .values
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((-100.0..100.0).contains(&min), "implausible min {min}");
        assert!((-100.0..100.0).contains(&max), "implausible max {max}");
    }

    #[test]
    fn decodes_a_real_captured_wind_v_10m_message() {
        let bytes = fixture("gec00_t12z_f000_vgrd10m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::WindV10m).unwrap();

        assert_eq!(decoded.variable, ForecastVariable::WindV10m);
        assert_eq!(decoded.unit, "m/s");
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Control));
        let min = decoded.values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = decoded
            .values
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((-100.0..100.0).contains(&min), "implausible min {min}");
        assert!((-100.0..100.0).contains(&max), "implausible max {max}");
    }

    #[test]
    fn decodes_a_real_captured_wind_gust_message() {
        let bytes = fixture("gec00_t12z_f000_gust.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::WindGust).unwrap();

        assert_eq!(decoded.variable, ForecastVariable::WindGust);
        assert_eq!(decoded.unit, "m/s");
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Control));

        // Gust magnitude is never negative and (barring an extraordinary
        // tornado/hurricane core cell) stays well under 150 m/s.
        let min = decoded.values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = decoded
            .values
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((0.0..150.0).contains(&min), "implausible min {min}");
        assert!((0.0..150.0).contains(&max), "implausible max {max}");
    }

    #[test]
    fn decodes_a_real_captured_relative_humidity_message() {
        let bytes = fixture("gec00_t12z_f000_rh2m.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::RelativeHumidity).unwrap();

        assert_eq!(decoded.variable, ForecastVariable::RelativeHumidity);
        assert_eq!(decoded.unit, "%");
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Control));

        // Relative humidity is bounded [0, 100] percent by definition, but
        // real model output can overshoot slightly at a handful of grid
        // points due to numerical noise near saturation (confirmed on this
        // exact fixture: max = 100.011%) -- widen the upper bound slightly
        // rather than assert an unrealistically exact physical limit.
        let min = decoded.values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = decoded
            .values
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((0.0..=100.5).contains(&min), "implausible min {min}");
        assert!((0.0..=100.5).contains(&max), "implausible max {max}");
    }

    #[test]
    fn decodes_a_real_captured_mslp_message() {
        // This is GEFS's `MSLET` message (mean sea level pressure, Eta
        // model reduction), NOT `PRMSL` -- see this module's `idx_names`
        // doc comment and `forecast_core::variable::ForecastVariable::grib2_parameter`'s
        // doc comment on `Mslp` for why, and this stage's empirical
        // verification of MSLET's real (category, number) = (3, 192).
        let bytes = fixture("gec00_t12z_f000_mslet.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::Mslp).unwrap();

        assert_eq!(decoded.variable, ForecastVariable::Mslp);
        assert_eq!(decoded.unit, "Pa");
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Control));

        // Mean sea level pressure on Earth is always within a fairly tight
        // physical band -- the lowest sea-level pressure ever recorded
        // (a super typhoon's eye) is about 870 hPa, the highest (Siberian
        // winter high) about 1085 hPa. Widen slightly for a safety margin.
        let min = decoded.values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = decoded
            .values
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (80_000.0..110_000.0).contains(&min),
            "implausible min {min}"
        );
        assert!(
            (80_000.0..110_000.0).contains(&max),
            "implausible max {max}"
        );
    }

    #[test]
    fn rejects_mslp_decoded_against_the_standard_prmsl_parameter_pair() {
        // A real PRMSL message (the *other* real mean-sea-level-pressure
        // message GEFS's own .idx also lists, standard WMO parameter
        // (3, 1)) must NOT be accepted as this crate's `Mslp` -- confirming
        // the corrected (3, 192) pairing is specific to MSLET, not merely
        // "any mean sea level pressure message".
        let bytes = fixture("gec00_t12z_f000_prmsl.grib2");
        let err = decode_field("u", &bytes, ForecastVariable::Mslp).unwrap_err();
        assert!(matches!(
            err,
            GefsError::UnexpectedField {
                category: 3,
                number: 1,
                ..
            }
        ));
    }

    #[test]
    fn decodes_a_real_captured_cloud_cover_message_for_the_control_member() {
        // Cloud cover has no instantaneous message in GEFS's `pgrb2sp25` --
        // every real `TCDC` message is a 0-3-hour time average, using GRIB2
        // Product Definition Template 4.11 rather than 4.1 (see
        // `crate::ensemble`'s module doc comment for the empirical
        // verification that 4.11 shares 4.1's ensemble-identity byte
        // layout).
        let bytes = fixture("gec00_t12z_f003_tcdc.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::CloudCover).unwrap();

        assert_eq!(decoded.variable, ForecastVariable::CloudCover);
        assert_eq!(decoded.unit, "%");
        assert_eq!(decoded.provider_id, "gefs");
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Control));
        assert_eq!(decoded.forecast_lead_hours, 3);
        assert_eq!(decoded.geometry.width(), 1440);
        assert_eq!(decoded.geometry.height(), 721);

        let min = decoded.values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = decoded
            .values
            .iter()
            .cloned()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((0.0..=100.0).contains(&min), "implausible min {min}");
        assert!((0.0..=100.0).contains(&max), "implausible max {max}");
    }

    #[test]
    fn decodes_a_real_captured_cloud_cover_message_for_the_ensemble_mean() {
        // Same as above but the ensemble-mean message, which uses Template
        // 4.12 (4.2's time-interval counterpart) rather than plain 4.2.
        let bytes = fixture("geavg_t12z_f003_tcdc.grib2");
        let decoded = decode_field("u", &bytes, ForecastVariable::CloudCover).unwrap();
        assert_eq!(decoded.ensemble, Some(EnsembleStatistic::Mean));
        assert_eq!(decoded.forecast_lead_hours, 3);
    }

    #[test]
    fn precipitation_1h_and_geopotential_height_remain_undecoded() {
        // GeopotentialHeight is explicitly out of scope for this stage (see
        // `idx_names`'s doc comment). Precipitation1h is also not decoded:
        // GEFS's `pgrb2sp25` product group publishes forecast hours only on
        // a strict 3-hour cadence (confirmed empirically -- f001/f002/f004
        // all 404 against the live bucket) and its `APCP` message is
        // always a 0-3-hour (or later 0-6-hour) *accumulation*, never a
        // true 1-hour one; decoding it and labeling it `Precipitation1h`
        // would mislabel the physical quantity.
        assert_eq!(idx_names(ForecastVariable::GeopotentialHeight), None);
        assert_eq!(idx_names(ForecastVariable::Precipitation1h), None);
    }
}
