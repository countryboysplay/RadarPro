//! Structured errors for every fallible operation in this crate.
//!
//! Per GLOBAL_CONTRACT.md ("Remote data is unreliable and untrusted" / "No
//! uncontrolled panics on malformed input"), every network response and
//! every byte of a fetched GRIB2 message is untrusted: a missing member, a
//! missing forecast hour, an absent field, a malformed `.idx` file, or a
//! GRIB2 grid/packing template this crate does not (yet) support must
//! always come back as one of these variants -- never a panic, and never a
//! silent fallback to the wrong data.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GefsError {
    #[error("invalid bucket URL {url:?}: {message}")]
    InvalidBucketUrl { url: String, message: String },

    #[error("network request to {url} failed: {source}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    /// A 404 (or other "this object does not exist") response -- the
    /// expected, non-exceptional way of finding out a requested
    /// member/run-hour/forecast-hour genuinely does not exist (e.g.
    /// `gep31`, which this project has confirmed live does not exist: only
    /// `gep01..gep30` do). Kept distinct from [`GefsError::Request`] so
    /// callers (and this crate's own tests) can tell "doesn't exist" apart
    /// from "the network/service is having a bad day".
    #[error("no such object at {url} (HTTP {status})")]
    NotFound { url: String, status: u16 },

    #[error("unexpected HTTP status {status} for {url}")]
    HttpStatus { url: String, status: u16 },

    #[error("malformed ListObjectsV2 XML response from {url}: {message}")]
    MalformedListing { url: String, message: String },

    #[error(
        "malformed .idx file at {url}: line {line_number} ({line:?}) does not match the \
         expected \"N:offset:d=YYYYMMDDHH:VAR:LEVEL:step:ENS=...\" format"
    )]
    MalformedIdxLine {
        url: String,
        line_number: usize,
        line: String,
    },

    #[error("no message matching variable {variable:?} / level {level:?} found in .idx at {url}")]
    FieldNotFoundInIdx {
        url: String,
        variable: String,
        level: String,
    },

    #[error(
        "byte range for the last message in {url} needs the object's total Content-Length, \
         but the HEAD response did not include one"
    )]
    MissingContentLength { url: String },

    #[error("GRIB2 parsing failed for {url}: {message}")]
    Grib2Parse { url: String, message: String },

    #[error(
        "{url} contains no GRIB2 submessages (0 decodable messages in the fetched byte range)"
    )]
    NoGrib2Submessage { url: String },

    #[error(
        "{url}: unsupported Grid Definition Template 3.{template_number} (this crate only \
         supports Template 3.0, regular latitude/longitude -- the only template GEFS's \
         pgrb2sp25 product has been empirically observed to use)"
    )]
    UnsupportedGridTemplate { url: String, template_number: u16 },

    #[error(
        "{url}: decoded values do not cover every grid cell exactly once when placed by the \
         GRIB2 message's own (i, j) scanning order -- refusing to render a grid with silently \
         missing or double-assigned cells"
    )]
    IncompleteGridCoverage { url: String },

    #[error(
        "{url}: unsupported Product Definition Template 4.{template_number} (this crate only \
         supports 4.1, individual ensemble forecast, and 4.2, derived forecast from all \
         ensemble members -- the two templates GEFS's control/perturbed members and ensemble \
         mean have been empirically observed to use)"
    )]
    UnsupportedProductTemplate { url: String, template_number: u16 },

    #[error(
        "{url}: unrecognized ensemble type code {ensemble_type} in Product Definition Template \
         4.1 (WMO Code Table 4.6) -- refusing to guess whether this is a control or perturbed \
         member"
    )]
    UnrecognizedEnsembleType { url: String, ensemble_type: u8 },

    #[error(
        "{url}: unrecognized derived-forecast type code {derived_type} in Product Definition \
         Template 4.2 (WMO Code Table 4.7) -- refusing to guess what ensemble statistic this is"
    )]
    UnrecognizedDerivedForecastType { url: String, derived_type: u8 },

    #[error(
        "{url}: decoded value count {decoded} does not match the grid's declared point count \
         {expected} (ni={ni} x nj={nj}) -- refusing to reshape a mismatched value count into a \
         grid, which could silently mis-map every value to the wrong cell"
    )]
    GridSizeMismatch {
        url: String,
        decoded: usize,
        expected: usize,
        ni: u32,
        nj: u32,
    },

    #[error(
        "{url}: parameter category/number {category}/{number} does not match the requested \
         field {expected_field:?} -- refusing to label the wrong physical field with the \
         requested one's name"
    )]
    UnexpectedField {
        url: String,
        category: u8,
        number: u8,
        expected_field: String,
    },

    #[error(
        "{url}: Product Definition Template 4.{template_number}'s raw payload is only {actual} \
         byte(s) long, too short to contain the ensemble-identity field(s) this template is \
         supposed to carry"
    )]
    TruncatedProductDefinition {
        url: String,
        template_number: u16,
        actual: usize,
    },

    #[error(
        "no published GEFS run found for product group {group:?} in the last {lookback_days} \
         day(s) -- either no network access, NOAA's service is unavailable, or a genuine \
         bucket-layout change"
    )]
    NoPublishedRunFound { group: String, lookback_days: u32 },

    #[error("operation was cancelled")]
    Cancelled,
}

impl GefsError {
    /// `true` for the specific, expected "this object does not exist"
    /// condition (HTTP 404) -- e.g. a member/forecast-hour that is simply
    /// not part of a given run. Lets callers (and tests) distinguish "the
    /// thing you asked for doesn't exist" from every other failure mode
    /// without matching on HTTP status codes themselves.
    pub fn is_not_found(&self) -> bool {
        matches!(self, GefsError::NotFound { .. })
    }
}
