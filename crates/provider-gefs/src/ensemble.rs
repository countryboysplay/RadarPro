//! Ensemble identity extraction: control / perturbed member / mean.
//!
//! This is the whole reason GEFS replaced WeatherNext 3 for this stage
//! (S07 stage file: "ensemble statistics: mean, spread, and member access
//! -- this is the probabilistic angle WeatherNext 3 would have provided").
//! [`identity_from_prod_def`] extracts `forecast_core::ensemble::EnsembleStatistic`
//! (S08: generalized from this crate's own S07 `EnsembleIdentity`, moved to
//! `forecast-core` so a deterministic provider like HRRR is not forced to
//! define a matching enum it has no use for) from the *decoded GRIB2
//! Product Definition Section* -- never from the object key's filename
//! alone (`decode::decode_field` cross-checks the two agree, see its
//! module docs).
//!
//! # A real gap in `grib` 0.18.5's public API
//!
//! [`grib::ProdDefinition`] exposes `parameter_category`/`parameter_number`/
//! `generating_process`/`forecast_time`/`fixed_surfaces`, but has **no**
//! accessor for the ensemble-specific fields WMO GRIB2 Product Definition
//! Templates 4.1 ("individual ensemble forecast") and 4.2 ("derived
//! forecast based on all ensemble members") both carry: type of ensemble
//! forecast / perturbation number (4.1), derived forecast type (4.2).
//! Verified by reading `grib-0.18.5`'s own source
//! (`src/datatypes/sections.rs`): `ProdDefinition`'s only public way to
//! reach template-specific bytes at all is `iter()`, which yields the raw,
//! already section-bounds-checked template payload.
//!
//! Per the S07 stage file ("If it can't handle a section/encoding GEFS
//! actually uses, document the gap in an ADR rather than writing a partial
//! from-scratch parser"), this is exactly that kind of gap -- not a
//! decoding gap (grid/packing/values are fully handled by `grib`), just a
//! missing metadata *accessor*. Rather than hand-rolling a general PDT
//! parser, this module reads only the two specific, fixed-offset fields
//! WMO's spec defines for templates 4.1/4.2, from the bytes `grib` itself
//! already validated and handed back -- documented fully in
//! `docs/adr/0011-gefs-grib2-crate-selection-and-limitations.md`, with the
//! exact byte offsets cross-checked against three real, live-fetched
//! messages (`gec00`, `gep01`, `geavg`) in this module's tests.
//!
//! # Templates 4.11/4.12 (added when decoding `CloudCover`)
//!
//! GEFS's `pgrb2sp25` product group has no *instantaneous* cloud-cover
//! message at all -- every real `TCDC` message observed is a 0-3-hour
//! time-average, which WMO's spec puts under Template 4.11 ("individual
//! ensemble forecast ... in a continuous or non-continuous time interval")
//! for a member/control forecast, or 4.12 (4.2's time-interval
//! counterpart) for the ensemble mean, never plain 4.1/4.2. Both templates
//! are documented (WMO Manual on Codes, FM 92 GRIB2, Template 4.11/4.12
//! definitions) as 4.1/4.2's exact octet-for-octet prefix through the
//! ensemble-type/perturbation-number (4.11) or derived-type (4.12) field,
//! with a time-range extension appended *after* that point -- so the same
//! fixed offsets below apply unchanged. Confirmed empirically, not just
//! from the spec: a real GEFS `TCDC` `gec00` message decodes template
//! number 11 with byte 29 = 1 (identical value/offset to a real 4.1 `TMP`
//! `gec00` message's "unperturbed low-res control"), and a real `TCDC`
//! `geavg` message decodes template number 12 with byte 29 = 0 (identical
//! to a real 4.2 `TMP` `geavg` message's "unweighted mean of all
//! members").

use crate::error::GefsError;
use forecast_core::ensemble::EnsembleStatistic;

/// Byte offset **into `ProdDefinition::iter()`'s full raw payload**
/// (`grib`'s `ProdDefinition` stores the 2-byte "number of coordinate
/// values" + 2-byte "template number" prefix as part of that same raw
/// payload, *before* the template's own octet 1 (parameter category) --
/// confirmed empirically: `num_coordinates()`/`prod_tmpl_num()` read
/// `payload[0..2]`/`payload[2..4]` respectively, per `grib`'s own source).
///
/// This is the "type of ensemble forecast" field in Template 4.1: WMO
/// GRIB2 template 4.1 octet 26 (1-based, counting from the template's own
/// octet 1), i.e. template-relative 0-based offset 25, plus the 4-byte
/// prefix above, giving raw offset 29. Cross-checked against three real,
/// live-fetched messages in this module's tests (`gec00`: byte 29 is 1,
/// "unperturbed low-res control"; `gep01`: byte 29 is 3, "positively
/// perturbed", byte 30 is 1, "perturbation number 1").
/// Also the correct offset for Template 4.11 ("individual ensemble
/// forecast, control and perturbed, at a horizontal level or in a
/// horizontal layer, in a continuous or non-continuous time interval") --
/// WMO defines 4.11 as 4.1's exact octet-for-octet prefix (through the
/// "number of forecasts in the ensemble" field) with a time-range
/// extension appended afterward, so the ensemble-type/perturbation-number
/// fields land at the identical raw offset. Confirmed empirically: a real
/// GEFS `TCDC` (cloud cover) `gec00` message -- cloud cover is a
/// 0-3-hour-average quantity, so it necessarily uses 4.11, never plain
/// 4.1 -- has byte 29 = 1 ("unperturbed low-res control"), the exact same
/// value a real 4.1 `TMP` `gec00` message has at the same offset.
const TEMPLATE_4_1_ENSEMBLE_TYPE_OFFSET: usize = 29;
const TEMPLATE_4_1_PERTURBATION_NUMBER_OFFSET: usize = 30;
/// Template 4.2 octet 26 (1-based) = template-relative offset 25 + the
/// same 4-byte prefix = raw offset 29: "derived forecast type" (WMO Code
/// Table 4.7). Cross-checked against a real `geavg` message: byte 29 = 0
/// "unweighted mean of all members".
///
/// Also the correct offset for Template 4.12 (4.2's time-interval
/// counterpart, same relationship as 4.1/4.11 above): confirmed
/// empirically against a real GEFS `TCDC` `geavg` message (byte 29 = 0,
/// "unweighted mean of all members" -- identical value and offset to a
/// real 4.2 `TMP` `geavg` message).
const TEMPLATE_4_2_DERIVED_TYPE_OFFSET: usize = 29;

/// Extract [`EnsembleStatistic`] from a decoded submessage's Product
/// Definition Section. `url` is used only to attribute a returned error to
/// the message it came from.
pub fn identity_from_prod_def(
    url: &str,
    prod_def: &grib::ProdDefinition,
) -> Result<EnsembleStatistic, GefsError> {
    let template_number = prod_def.prod_tmpl_num();
    let raw: Vec<u8> = prod_def.iter().copied().collect();

    match template_number {
        1 | 11 => {
            let ensemble_type = *raw.get(TEMPLATE_4_1_ENSEMBLE_TYPE_OFFSET).ok_or_else(|| {
                GefsError::TruncatedProductDefinition {
                    url: url.to_string(),
                    template_number,
                    actual: raw.len(),
                }
            })?;
            let perturbation_number = *raw
                .get(TEMPLATE_4_1_PERTURBATION_NUMBER_OFFSET)
                .ok_or_else(|| GefsError::TruncatedProductDefinition {
                    url: url.to_string(),
                    template_number,
                    actual: raw.len(),
                })?;
            match ensemble_type {
                0 | 1 => Ok(EnsembleStatistic::Control),
                2 | 3 => Ok(EnsembleStatistic::Member(perturbation_number)),
                other => Err(GefsError::UnrecognizedEnsembleType {
                    url: url.to_string(),
                    ensemble_type: other,
                }),
            }
        }
        2 | 12 => {
            let derived_type = *raw.get(TEMPLATE_4_2_DERIVED_TYPE_OFFSET).ok_or_else(|| {
                GefsError::TruncatedProductDefinition {
                    url: url.to_string(),
                    template_number,
                    actual: raw.len(),
                }
            })?;
            match derived_type {
                0 => Ok(EnsembleStatistic::Mean),
                other => Err(GefsError::UnrecognizedDerivedForecastType {
                    url: url.to_string(),
                    derived_type: other,
                }),
            }
        }
        other => Err(GefsError::UnsupportedProductTemplate {
            url: url.to_string(),
            template_number: other,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prod_def_from_raw(
        num_coordinates: u16,
        template_number: u16,
        template_bytes: &[u8],
    ) -> grib::ProdDefinition {
        let mut payload = Vec::new();
        payload.extend_from_slice(&num_coordinates.to_be_bytes());
        payload.extend_from_slice(&template_number.to_be_bytes());
        payload.extend_from_slice(template_bytes);
        grib::ProdDefinition::from_payload(payload.into_boxed_slice()).unwrap()
    }

    // Raw template 4.1 payload bytes, captured verbatim from a real,
    // live-fetched GEC00 (control) TMP-2m f000 message
    // (gefs.20260912/12/atmos/pgrb2sp25/gec00.t12z.pgrb2s.0p25.f000,
    // message #11) via this project's own `grib` probe. See this module's
    // doc comment and `docs/adr/0011-...` for how this was verified.
    const REAL_GEC00_TEMPLATE_4_1: &[u8] = &[
        0, 0, 4, 0, 107, 0, 0, 0, 1, 0, 0, 0, 0, 103, 0, 0, 0, 0, 2, 255, 0, 0, 0, 0, 0, 1, 0, 30,
    ];
    // Same, for a real GEP01 (perturbed member 1) fetch.
    const REAL_GEP01_TEMPLATE_4_1: &[u8] = &[
        0, 0, 4, 0, 107, 0, 0, 0, 1, 0, 0, 0, 0, 103, 0, 0, 0, 0, 2, 255, 0, 0, 0, 0, 0, 3, 1, 30,
    ];
    // Real GEAVG (ensemble mean) template 4.2 payload.
    const REAL_GEAVG_TEMPLATE_4_2: &[u8] = &[
        0, 0, 4, 0, 107, 0, 0, 0, 1, 0, 0, 0, 0, 103, 0, 0, 0, 0, 2, 255, 0, 0, 0, 0, 0, 0, 30,
    ];

    // Raw template 4.11 payload bytes, captured verbatim from a real,
    // live-fetched GEC00 (control) TCDC (cloud cover) f003 message
    // (gefs.20260913/12/atmos/pgrb2sp25/gec00.t12z.pgrb2s.0p25.f003,
    // message #28) -- cloud cover has no instantaneous message in this
    // product group, so this is the only real template 4.11 payload this
    // crate's own tests can be built from (see this module's doc comment).
    const REAL_GEC00_TCDC_TEMPLATE_4_11: &[u8] = &[
        6, 1, 4, 0, 107, 0, 0, 0, 1, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0, 255, 0, 0, 0, 0, 0, 1, 0, 30,
        7, 234, 9, 13, 15, 0, 0, 1, 0, 0, 0, 0, 0, 2, 1, 0, 0, 0, 3, 255, 0, 0, 0, 0,
    ];
    // Same, for a real GEAVG (ensemble mean) TCDC f003 message -- template
    // 4.12.
    const REAL_GEAVG_TCDC_TEMPLATE_4_12: &[u8] = &[
        6, 1, 4, 0, 107, 0, 0, 0, 1, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 30, 7,
        234, 9, 13, 15, 0, 0, 1, 0, 0, 0, 0, 0, 2, 1, 0, 0, 0, 3, 255, 0, 0, 0, 0,
    ];

    #[test]
    fn real_gec00_message_is_identified_as_control() {
        let prod_def = prod_def_from_raw(0, 1, REAL_GEC00_TEMPLATE_4_1);
        assert_eq!(
            identity_from_prod_def("u", &prod_def).unwrap(),
            EnsembleStatistic::Control
        );
    }

    #[test]
    fn real_gep01_message_is_identified_as_member_one() {
        let prod_def = prod_def_from_raw(0, 1, REAL_GEP01_TEMPLATE_4_1);
        assert_eq!(
            identity_from_prod_def("u", &prod_def).unwrap(),
            EnsembleStatistic::Member(1)
        );
    }

    #[test]
    fn real_geavg_message_is_identified_as_mean() {
        let prod_def = prod_def_from_raw(0, 2, REAL_GEAVG_TEMPLATE_4_2);
        assert_eq!(
            identity_from_prod_def("u", &prod_def).unwrap(),
            EnsembleStatistic::Mean
        );
    }

    #[test]
    fn real_tcdc_gec00_template_4_11_message_is_identified_as_control() {
        // Confirms template 4.11 (time-interval ensemble forecast) is
        // handled identically to plain 4.1 -- see this module's doc
        // comment.
        let prod_def = prod_def_from_raw(0, 11, REAL_GEC00_TCDC_TEMPLATE_4_11);
        assert_eq!(
            identity_from_prod_def("u", &prod_def).unwrap(),
            EnsembleStatistic::Control
        );
    }

    #[test]
    fn real_tcdc_geavg_template_4_12_message_is_identified_as_mean() {
        // Confirms template 4.12 (time-interval derived-forecast) is
        // handled identically to plain 4.2 -- see this module's doc
        // comment.
        let prod_def = prod_def_from_raw(0, 12, REAL_GEAVG_TCDC_TEMPLATE_4_12);
        assert_eq!(
            identity_from_prod_def("u", &prod_def).unwrap(),
            EnsembleStatistic::Mean
        );
    }

    #[test]
    fn unsupported_template_number_is_a_clear_error_not_a_panic() {
        let prod_def = prod_def_from_raw(0, 0, &[0u8; 40]);
        let err = identity_from_prod_def("u", &prod_def).unwrap_err();
        assert!(matches!(
            err,
            GefsError::UnsupportedProductTemplate {
                template_number: 0,
                ..
            }
        ));
    }

    #[test]
    fn truncated_template_4_1_payload_is_a_clear_error_not_a_panic() {
        let prod_def = prod_def_from_raw(0, 1, &[0u8; 5]);
        let err = identity_from_prod_def("u", &prod_def).unwrap_err();
        assert!(matches!(
            err,
            GefsError::TruncatedProductDefinition {
                template_number: 1,
                ..
            }
        ));
    }

    #[test]
    fn unrecognized_ensemble_type_is_a_clear_error_not_a_panic() {
        // Index 25 here is relative to `REAL_GEC00_TEMPLATE_4_1` (the
        // template-only bytes, no 4-byte prefix) -- this crate's own
        // `TEMPLATE_4_1_ENSEMBLE_TYPE_OFFSET` (29) is relative to the
        // *full* raw payload `prod_def_from_raw` builds below, which is 4
        // bytes longer.
        let mut bytes = REAL_GEC00_TEMPLATE_4_1.to_vec();
        bytes[25] = 99;
        let prod_def = prod_def_from_raw(0, 1, &bytes);
        let err = identity_from_prod_def("u", &prod_def).unwrap_err();
        assert!(matches!(
            err,
            GefsError::UnrecognizedEnsembleType {
                ensemble_type: 99,
                ..
            }
        ));
    }

    #[test]
    fn unrecognized_derived_forecast_type_is_a_clear_error_not_a_panic() {
        // Same relative-vs-absolute-offset note as above: index 25 here is
        // relative to the template-only fixture bytes.
        let mut bytes = REAL_GEAVG_TEMPLATE_4_2.to_vec();
        bytes[25] = 7;
        let prod_def = prod_def_from_raw(0, 2, &bytes);
        let err = identity_from_prod_def("u", &prod_def).unwrap_err();
        assert!(matches!(
            err,
            GefsError::UnrecognizedDerivedForecastType {
                derived_type: 7,
                ..
            }
        ));
    }
}
