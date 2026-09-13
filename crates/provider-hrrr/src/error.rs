//! Structured errors for every fallible operation in this crate.
//!
//! Per GLOBAL_CONTRACT.md ("Remote data is unreliable and untrusted" / "No
//! uncontrolled panics on malformed input"), every network response and
//! every byte of a fetched GRIB2 message is untrusted -- never a panic,
//! never a silent fallback to the wrong data.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum HrrrError {
    #[error("invalid bucket URL {url:?}: {message}")]
    InvalidBucketUrl { url: String, message: String },

    #[error("network request to {url} failed: {source}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("no such object at {url} (HTTP {status})")]
    NotFound { url: String, status: u16 },

    #[error("unexpected HTTP status {status} for {url}")]
    HttpStatus { url: String, status: u16 },

    #[error("malformed ListObjectsV2 XML response from {url}: {message}")]
    MalformedListing { url: String, message: String },

    #[error(
        "malformed .idx file at {url}: line {line_number} ({line:?}) does not match the \
         expected \"N:offset:d=YYYYMMDDHH:VAR:LEVEL:step:\" format"
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
         supports Template 3.30, Lambert Conformal Conic -- the only template real HRRR `conus` \
         surface messages have been empirically observed to use)"
    )]
    UnsupportedGridTemplate { url: String, template_number: u16 },

    #[error(
        "{url}: unsupported Grid Definition Template 3.30 earth shape (WMO Code Table 3.2 value \
         {shape}) -- this crate only supports shape 6 (spherical, radius 6,371,229.0 m), the \
         only value real HRRR messages have been empirically observed to use"
    )]
    UnsupportedEarthShape { url: String, shape: u8 },

    #[error(
        "{url}: could not find grid index (1,0) and/or (0,1) while deriving this message's real \
         Lambert Conformal grid spacing -- the grid may be smaller than 2x2 or `grib`'s own \
         (i, j) enumeration order was not what this crate expected"
    )]
    MissingGeometryReferencePoint { url: String },

    #[error(
        "{url}: decoded values do not cover every grid cell exactly once when placed by the \
         GRIB2 message's own (i, j) scanning order -- refusing to render a grid with silently \
         missing or double-assigned cells"
    )]
    IncompleteGridCoverage { url: String },

    #[error(
        "{url}: unsupported Product Definition Template 4.{template_number} (this crate only \
         supports 4.0, \"analysis or forecast at a horizontal level\" -- the deterministic \
         template every real HRRR message has been empirically observed to use; HRRR has no \
         ensemble, so this crate never expects 4.1/4.2)"
    )]
    UnsupportedProductTemplate { url: String, template_number: u16 },

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
        "no published HRRR run found for product {product:?} in the last {lookback_hours} \
         hour(s) -- either no network access, NOAA's service is unavailable, or a genuine \
         bucket-layout change"
    )]
    NoPublishedRunFound {
        product: String,
        lookback_hours: u32,
    },

    /// This provider is deterministic (no ensemble); a request naming a
    /// specific ensemble statistic/member is a caller error, not something
    /// this provider can silently ignore or fake.
    #[error(
        "provider-hrrr is deterministic (HRRR publishes no ensemble); a FieldRequest naming an \
         ensemble statistic/member is not supported"
    )]
    EnsembleNotSupported,
}
