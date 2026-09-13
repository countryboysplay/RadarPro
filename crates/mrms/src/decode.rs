//! Decode one fetched, already-decompressed MRMS GRIB2 message into an
//! [`crate::grid::MrmsGrid`].
//!
//! # Discipline 209: local-use codes, verified not to affect decoding
//!
//! Every real MRMS message fetched during this stage's verification
//! declares GRIB2 Section 0 discipline byte `209` (WMO Table 0.0's "local
//! use" range 192-254), not a standard meteorological discipline --
//! confirmed by reading the raw decompressed bytes directly (`bytes[6]`).
//! Section 4 (Product Definition)'s `parameter_category`/`parameter_number`
//! are therefore center-local codes (confirmed different between the two
//! products this crate decodes: reflectivity uses category 10/number 0,
//! `PrecipRate` uses category 6/number 1), not a standard WMO Table 4.2
//! lookup -- this module never attempts to interpret what they *mean*, it
//! only records them as diagnostic metadata; physical field identity comes
//! entirely from which [`MrmsProduct`]/URL was fetched (matching
//! `DATA_SOURCES.md`'s explicit guidance).
//!
//! Empirically confirmed **not** to affect `grib`'s actual decode path: the
//! same `grib` 0.18.5 Section 5 Template 5.41 (PNG) unpacking that
//! discipline-0 messages would use decodes discipline-209 MRMS messages
//! identically -- `grib`'s packing/decode logic is driven entirely by
//! Section 5's own template number, never by Section 0's discipline byte.
//! See `docs/adr/0014-mrms-grib2-png-unpack-and-local-discipline.md` for the
//! full empirical trail (a throwaway probe program decoding real messages).
//!
//! # Missing-value sentinels, verified not assumed
//!
//! See `crate::grid`'s module doc for the [`crate::grid::MrmsCellValue`]
//! shape this module classifies raw decoded floats into. The two exact
//! sentinel values below were found empirically (a throwaway probe
//! decoding a real message and mapping decoded cells back to lon/lat: the
//! `-999.0`/`-3.0` sentinel geographically traces exactly the CONUS
//! network's ocean-side domain edges, while `-99.0` fills the interior
//! landmass wherever there is currently no echo -- see the ADR for the full
//! evidence, including an ASCII-rendered coverage map).
//!
//! # Untrusted input
//!
//! `bytes` is exactly the untrusted, network-sourced data GLOBAL_CONTRACT
//! describes: every fallible step below returns a structured [`MrmsError`],
//! never panics.

use forecast_core::grid::{GridGeometry, RegularLatLonGrid};
use forecast_core::time::UtcTimestamp;
use grib::GridDefinitionTemplateValues;

use crate::error::MrmsError;
use crate::grid::{MrmsCellValue, MrmsGrid};
use crate::keys::MrmsProduct;

/// GRIB2 Grid Definition Template 3.0 ("Latitude/Longitude") -- the only
/// grid template every real MRMS CONUS message fetched during this stage's
/// verification used (same family GEFS uses).
const GRID_TEMPLATE_REGULAR_LAT_LON: u16 = 0;

/// GRIB2 Product Definition Template 4.0 ("Analysis or forecast at a
/// horizontal level") -- the deterministic, non-ensemble template every
/// real MRMS message fetched during this stage's verification used.
const PROD_TEMPLATE_DETERMINISTIC: u16 = 0;

/// Fixed-point scale for template 3.0's `La1`/`Lo1`/`La2`/`Lo2`/`Di`/`Dj`
/// fields (10^-6 degree, same convention `provider-gefs` uses for the same
/// template).
const COORD_SCALE: f64 = 1e-6;

/// Reflectivity sentinel: cell is outside the radar mosaic's domain
/// entirely. Verified empirically -- see this module's doc comment.
const REFLECTIVITY_NO_COVERAGE_SENTINEL: f32 = -999.0;
/// Reflectivity sentinel: cell has coverage but no significant echo.
/// Verified empirically -- see this module's doc comment.
const REFLECTIVITY_MISSING_SENTINEL: f32 = -99.0;
/// `PrecipRate` sentinel: cell is outside the radar mosaic's domain
/// entirely. Verified empirically -- see this module's doc comment.
/// `PrecipRate` has no separate "covered but no rain" sentinel: a genuine
/// `0.0` mm/hr reading already means exactly that, physically.
const PRECIP_RATE_NO_COVERAGE_SENTINEL: f32 = -3.0;

/// How close a decoded raw value must be to a known sentinel to be
/// classified as one. Real quantized values (PNG-packed, scale/offset
/// decoded) are spaced by at least 0.1 (PrecipRate) or 0.5 (reflectivity)
/// near these sentinels in every message observed, so this tolerance
/// cannot misclassify a genuine physical reading as a sentinel.
const SENTINEL_TOLERANCE: f32 = 1e-3;

fn classify_raw_value(product: MrmsProduct, raw: f32) -> MrmsCellValue {
    match product {
        MrmsProduct::ReflectivityQcComposite => {
            if (raw - REFLECTIVITY_NO_COVERAGE_SENTINEL).abs() < SENTINEL_TOLERANCE {
                MrmsCellValue::NoCoverage
            } else if (raw - REFLECTIVITY_MISSING_SENTINEL).abs() < SENTINEL_TOLERANCE {
                MrmsCellValue::Missing
            } else {
                MrmsCellValue::Value(raw)
            }
        }
        MrmsProduct::PrecipRate => {
            if (raw - PRECIP_RATE_NO_COVERAGE_SENTINEL).abs() < SENTINEL_TOLERANCE {
                MrmsCellValue::NoCoverage
            } else {
                MrmsCellValue::Value(raw)
            }
        }
    }
}

/// Decode `bytes` (one whole, already-gunzipped GRIB2 message) into an
/// [`MrmsGrid`] for `product`.
pub fn decode_field(url: &str, bytes: &[u8], product: MrmsProduct) -> Result<MrmsGrid, MrmsError> {
    let grib2 = grib::from_bytes(bytes.to_vec()).map_err(|e| MrmsError::Grib2Parse {
        url: url.to_string(),
        message: e.to_string(),
    })?;

    let mut iter = grib2.iter();
    let (_, submessage) = iter.next().ok_or_else(|| MrmsError::NoGrib2Submessage {
        url: url.to_string(),
    })?;

    let prod_def = submessage.prod_def();
    if prod_def.prod_tmpl_num() != PROD_TEMPLATE_DETERMINISTIC {
        return Err(MrmsError::UnsupportedProductTemplate {
            url: url.to_string(),
            template_number: prod_def.prod_tmpl_num(),
        });
    }

    // MRMS is a plain observation snapshot with no lead-time concept (see
    // `crate::grid`'s module doc): every real message verified for this
    // crate declared a `forecast_time` of exactly zero (whatever the unit),
    // meaning Section 1's reference time already *is* the observation's
    // valid time. A nonzero offset would mean that assumption doesn't hold
    // for some message this crate hasn't seen -- refuse to guess, rather
    // than silently mislabeling a lagged/offset time as the observation
    // instant.
    if let Some(forecast_time) = prod_def.forecast_time() {
        if forecast_time.value != 0 {
            return Err(MrmsError::Grib2Parse {
                url: url.to_string(),
                message: format!(
                    "MRMS message declared a nonzero forecast_time ({forecast_time:?}) -- this \
                     crate's observation-only data model assumes Section 1's reference time IS \
                     the observation's valid time, which only holds for a zero offset"
                ),
            });
        }
    }

    let identification = submessage.identification();
    let dt = identification.ref_time_unchecked();
    let valid_time = UtcTimestamp::new(
        i64::from(dt.year),
        u32::from(dt.month),
        u32::from(dt.day),
        u32::from(dt.hour),
        u32::from(dt.minute),
        u32::from(dt.second),
    );

    let grid_def = submessage.grid_def();
    if grid_def.grid_tmpl_num() != GRID_TEMPLATE_REGULAR_LAT_LON {
        return Err(MrmsError::UnsupportedGridTemplate {
            url: url.to_string(),
            template_number: grid_def.grid_tmpl_num(),
        });
    }
    let template_values =
        GridDefinitionTemplateValues::try_from(grid_def).map_err(|e| MrmsError::Grib2Parse {
            url: url.to_string(),
            message: e.to_string(),
        })?;
    let GridDefinitionTemplateValues::Template0(template0) = &template_values else {
        return Err(MrmsError::UnsupportedGridTemplate {
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
    // Same sign-derivation approach as `provider-gefs::decode` (see its
    // comment): MRMS's real La1=54.995N > La2=20.005N, i.e. latitude
    // decreases as the row/j-index increases, confirmed empirically.
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

    let ij_iter = submessage.ij().map_err(|e| MrmsError::Grib2Parse {
        url: url.to_string(),
        message: e.to_string(),
    })?;

    let decoder =
        grib::Grib2SubmessageDecoder::from(submessage).map_err(|e| MrmsError::Grib2Parse {
            url: url.to_string(),
            message: e.to_string(),
        })?;
    let decoded_values: Vec<f32> = decoder
        .dispatch()
        .map_err(|e| MrmsError::Grib2Parse {
            url: url.to_string(),
            message: e.to_string(),
        })?
        .collect();

    let expected = geometry.point_count();
    if decoded_values.len() != expected {
        return Err(MrmsError::GridSizeMismatch {
            url: url.to_string(),
            decoded: decoded_values.len(),
            expected,
            ni,
            nj,
        });
    }

    let mut values = vec![None; expected];
    for ((i, j), raw) in ij_iter.zip(decoded_values) {
        if i >= ni as usize || j >= nj as usize {
            return Err(MrmsError::IncompleteGridCoverage {
                url: url.to_string(),
            });
        }
        let index = j * ni as usize + i;
        values[index] = Some(classify_raw_value(product, raw));
    }
    if values.iter().any(Option::is_none) {
        return Err(MrmsError::IncompleteGridCoverage {
            url: url.to_string(),
        });
    }
    let values: Vec<MrmsCellValue> = values.into_iter().map(|v| v.unwrap()).collect();

    Ok(MrmsGrid {
        product,
        unit: product.native_unit(),
        valid_time,
        geometry,
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/mrms")
            .join(name);
        let compressed = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("missing test fixture {}: {e}", path.display()));
        crate::client::gunzip(name, &compressed).expect("fixture must be valid gzip")
    }

    #[test]
    fn decodes_a_real_captured_reflectivity_message() {
        let bytes = fixture("mrms_reflectivity_qc_composite_20260913-050438.grib2.gz");
        let decoded = decode_field("u", &bytes, MrmsProduct::ReflectivityQcComposite).unwrap();

        assert_eq!(decoded.unit, "dBZ");
        assert_eq!(decoded.product, MrmsProduct::ReflectivityQcComposite);
        assert_eq!(decoded.valid_time, UtcTimestamp::new(2026, 9, 13, 5, 4, 38));
        assert_eq!(decoded.geometry.width(), 7000);
        assert_eq!(decoded.geometry.height(), 3500);
        assert_eq!(decoded.values.len(), 7000 * 3500);
        let GridGeometry::RegularLatLon(g) = &decoded.geometry else {
            panic!("expected a regular lat/lon grid");
        };
        assert!((g.origin_lat_deg - 54.995).abs() < 1e-6);
        assert!((g.origin_lon_deg - 230.005).abs() < 1e-6);
        assert!((g.lat_step_deg - (-0.01)).abs() < 1e-9);
        assert!((g.lon_step_deg - 0.01).abs() < 1e-9);

        // Physical sanity + cross-reference: this crate's own live probe
        // (see docs/adr/0014-...) printed min=-999 (before classification)
        // max=63.5 for this exact message's raw decoded values.
        let stats = decoded.stats();
        assert!((stats.max - 63.5).abs() < 0.01, "max={}", stats.max);
        assert!(
            stats.min > -50.0,
            "min={} (a sentinel leaked through)",
            stats.min
        );
        assert!(stats.no_coverage_count > 0);
        assert!(stats.missing_count > 0);
        assert!(stats.valid_count > 0);
    }

    #[test]
    fn decodes_a_real_captured_precip_rate_message() {
        let bytes = fixture("mrms_precip_rate_20260913-050400.grib2.gz");
        let decoded = decode_field("u", &bytes, MrmsProduct::PrecipRate).unwrap();

        assert_eq!(decoded.unit, "mm/hr");
        assert_eq!(decoded.product, MrmsProduct::PrecipRate);
        assert_eq!(decoded.valid_time, UtcTimestamp::new(2026, 9, 13, 5, 4, 0));
        assert_eq!(decoded.geometry.width(), 7000);
        assert_eq!(decoded.geometry.height(), 3500);

        let stats = decoded.stats();
        assert!((stats.max - 175.0).abs() < 0.01, "max={}", stats.max);
        assert!(
            stats.min >= 0.0,
            "min={} (a sentinel leaked through)",
            stats.min
        );
        assert!(stats.no_coverage_count > 0);
        // PrecipRate has no distinct "covered but dry" sentinel -- a real
        // 0.0 mm/hr reading is itself a `Value`, never `Missing`.
        assert_eq!(stats.missing_count, 0);
    }

    #[test]
    fn no_coverage_geography_matches_the_ocean_not_the_conus_interior() {
        // Same real spot checks documented in the ADR: deep ocean, far from
        // any contributing radar, must classify as NoCoverage; a CONUS
        // interior point must never be NoCoverage (it may be Missing/no
        // current echo, or a real Value, but the radar network does cover
        // it).
        let bytes = fixture("mrms_reflectivity_qc_composite_20260913-050438.grib2.gz");
        let decoded = decode_field("u", &bytes, MrmsProduct::ReflectivityQcComposite).unwrap();

        // This grid's origin longitude is in the [0, 360) convention
        // (230.005 == -129.995), so query longitudes here use that same
        // convention (-65 == 295.0, -97.5 == 262.5) -- matching every
        // native harness's own CONUS bounding box convention in this
        // workspace (e.g. `provider-gefs`/`provider-hrrr`'s harnesses).
        let deep_atlantic = decoded.geometry.nearest_cell(295.0, 30.0).unwrap();
        assert_eq!(
            decoded.value_at(deep_atlantic.0, deep_atlantic.1),
            MrmsCellValue::NoCoverage
        );

        let oklahoma = decoded.geometry.nearest_cell(262.5, 35.5).unwrap();
        assert_ne!(
            decoded.value_at(oklahoma.0, oklahoma.1),
            MrmsCellValue::NoCoverage
        );
    }

    #[test]
    fn rejects_garbage_bytes_cleanly() {
        let err = decode_field("u", &[0u8; 100], MrmsProduct::ReflectivityQcComposite).unwrap_err();
        assert!(matches!(err, MrmsError::NoGrib2Submessage { .. }));
    }

    #[test]
    fn rejects_empty_bytes_cleanly() {
        let err = decode_field("u", &[], MrmsProduct::ReflectivityQcComposite).unwrap_err();
        assert!(matches!(err, MrmsError::NoGrib2Submessage { .. }));
    }

    #[test]
    fn rejects_a_truncated_real_message_cleanly_never_panics() {
        let bytes = fixture("mrms_reflectivity_qc_composite_20260913-050438.grib2.gz");
        let truncated = &bytes[..bytes.len() / 2];
        let _ = decode_field("u", truncated, MrmsProduct::ReflectivityQcComposite);
    }

    #[test]
    fn classify_raw_value_never_misclassifies_a_plausible_real_reading() {
        // Values close to but not exactly the sentinels must classify as
        // real data, never as a sentinel by accident.
        assert_eq!(
            classify_raw_value(MrmsProduct::ReflectivityQcComposite, -98.5),
            MrmsCellValue::Value(-98.5)
        );
        assert_eq!(
            classify_raw_value(MrmsProduct::ReflectivityQcComposite, 0.0),
            MrmsCellValue::Value(0.0)
        );
        assert_eq!(
            classify_raw_value(MrmsProduct::PrecipRate, 0.0),
            MrmsCellValue::Value(0.0)
        );
        assert_eq!(
            classify_raw_value(MrmsProduct::PrecipRate, -2.5),
            MrmsCellValue::Value(-2.5)
        );
    }
}
