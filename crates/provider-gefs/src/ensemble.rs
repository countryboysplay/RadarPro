//! Ensemble identity extraction: control / perturbed member / mean.
//!
//! This is the whole reason GEFS replaced WeatherNext 3 for this stage
//! (S07 stage file: "ensemble statistics: mean, spread, and member access
//! -- this is the probabilistic angle WeatherNext 3 would have provided").
//! [`EnsembleIdentity`] is this crate's canonical representation of "which
//! ensemble statistic or member does this decoded field represent", read
//! from the *decoded GRIB2 Product Definition Section* -- never from the
//! object key's filename alone (`decode::decode_field` cross-checks the
//! two agree, see its module docs).
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

use crate::error::GefsError;

/// Which ensemble statistic or member a decoded field represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnsembleIdentity {
    /// The unperturbed control forecast (WMO Code Table 4.6 values 0 or 1
    /// -- "unperturbed high-resolution control" or "unperturbed
    /// low-resolution control"; GEFS's `gec00` has always been observed
    /// using value 1).
    Control,
    /// A perturbed ensemble member, carrying its perturbation number
    /// (WMO Code Table 4.6 values 2 "negatively perturbed" or 3
    /// "positively perturbed" -- both map to this variant, since a
    /// member's *number* is the identity that matters for spread/member
    /// access, not the sign of its perturbation).
    Member(u8),
    /// The ensemble mean (WMO Code Table 4.7 value 0, "unweighted mean of
    /// all members").
    Mean,
}

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
const TEMPLATE_4_1_ENSEMBLE_TYPE_OFFSET: usize = 29;
const TEMPLATE_4_1_PERTURBATION_NUMBER_OFFSET: usize = 30;
/// Template 4.2 octet 26 (1-based) = template-relative offset 25 + the
/// same 4-byte prefix = raw offset 29: "derived forecast type" (WMO Code
/// Table 4.7). Cross-checked against a real `geavg` message: byte 29 = 0
/// "unweighted mean of all members".
const TEMPLATE_4_2_DERIVED_TYPE_OFFSET: usize = 29;

/// Extract [`EnsembleIdentity`] from a decoded submessage's Product
/// Definition Section. `url` is used only to attribute a returned error to
/// the message it came from.
pub fn identity_from_prod_def(
    url: &str,
    prod_def: &grib::ProdDefinition,
) -> Result<EnsembleIdentity, GefsError> {
    let template_number = prod_def.prod_tmpl_num();
    let raw: Vec<u8> = prod_def.iter().copied().collect();

    match template_number {
        1 => {
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
                0 | 1 => Ok(EnsembleIdentity::Control),
                2 | 3 => Ok(EnsembleIdentity::Member(perturbation_number)),
                other => Err(GefsError::UnrecognizedEnsembleType {
                    url: url.to_string(),
                    ensemble_type: other,
                }),
            }
        }
        2 => {
            let derived_type = *raw.get(TEMPLATE_4_2_DERIVED_TYPE_OFFSET).ok_or_else(|| {
                GefsError::TruncatedProductDefinition {
                    url: url.to_string(),
                    template_number,
                    actual: raw.len(),
                }
            })?;
            match derived_type {
                0 => Ok(EnsembleIdentity::Mean),
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

    #[test]
    fn real_gec00_message_is_identified_as_control() {
        let prod_def = prod_def_from_raw(0, 1, REAL_GEC00_TEMPLATE_4_1);
        assert_eq!(
            identity_from_prod_def("u", &prod_def).unwrap(),
            EnsembleIdentity::Control
        );
    }

    #[test]
    fn real_gep01_message_is_identified_as_member_one() {
        let prod_def = prod_def_from_raw(0, 1, REAL_GEP01_TEMPLATE_4_1);
        assert_eq!(
            identity_from_prod_def("u", &prod_def).unwrap(),
            EnsembleIdentity::Member(1)
        );
    }

    #[test]
    fn real_geavg_message_is_identified_as_mean() {
        let prod_def = prod_def_from_raw(0, 2, REAL_GEAVG_TEMPLATE_4_2);
        assert_eq!(
            identity_from_prod_def("u", &prod_def).unwrap(),
            EnsembleIdentity::Mean
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
