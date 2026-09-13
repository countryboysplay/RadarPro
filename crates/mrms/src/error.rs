//! Structured errors for every fallible operation in this crate.
//!
//! Per GLOBAL_CONTRACT.md ("Remote data is unreliable and untrusted" / "No
//! uncontrolled panics on malformed input"), every network response, every
//! byte of a fetched (gzip-wrapped) GRIB2 message, and every listed object
//! key is untrusted -- never a panic, never a silent fallback to the wrong
//! data.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MrmsError {
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

    #[error("gzip decompression failed for {url}: {message}")]
    GzipDecompress { url: String, message: String },

    #[error("GRIB2 parsing failed for {url}: {message}")]
    Grib2Parse { url: String, message: String },

    #[error("{url} contains no GRIB2 submessages (0 decodable messages in the fetched object)")]
    NoGrib2Submessage { url: String },

    #[error(
        "{url}: unsupported Grid Definition Template 3.{template_number} (this crate only \
         supports Template 3.0, regular lat/lon -- the only template real MRMS CONUS messages \
         have been empirically observed to use)"
    )]
    UnsupportedGridTemplate { url: String, template_number: u16 },

    #[error(
        "{url}: unsupported Product Definition Template 4.{template_number} (this crate only \
         supports 4.0, the deterministic \"analysis\" template every real MRMS message has been \
         empirically observed to use -- MRMS is an observation product with no ensemble concept)"
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
        "{url}: decoded values do not cover every grid cell exactly once when placed by the \
         GRIB2 message's own (i, j) scanning order -- refusing to render a grid with silently \
         missing or double-assigned cells"
    )]
    IncompleteGridCoverage { url: String },

    #[error(
        "no published MRMS snapshot found for product {product:?} in the last {lookback_days} \
         day(s) -- either no network access, NOAA's service is unavailable, or a genuine \
         bucket-layout change"
    )]
    NoPublishedSnapshotFound { product: String, lookback_days: u32 },
}
