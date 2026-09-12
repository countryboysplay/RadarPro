//! `nexrad-level2` — NEXRAD Archive II / Level II (WSR-88D) decoder.
//!
//! Decodes a complete Archive II Level II volume file into the canonical
//! polar domain model in `radar-types` (`Volume -> Sweep -> Radial ->
//! Moment`): the Volume Header Record, LDM Compressed Record framing
//! (bzip2), legacy metadata message framing (skipped, not semantically
//! decoded), and Message Type 31 ("Digital Radar Data Generic Format")
//! decoded in full for the REF, VEL, SW, ZDR, PHI, and RHO (correlation
//! coefficient) moments.
//!
//! # Threat model
//!
//! Input bytes are untrusted and network-sourced (per
//! `GLOBAL_CONTRACT.md`: "Remote data is unreliable and untrusted" / "No
//! uncontrolled panics on malformed input"). Every multi-byte read in this
//! crate goes through [`cursor::Cursor`], which bounds-checks before
//! touching the slice; [`decode_volume`] returns a structured
//! [`DecodeError`] rather than panicking, indexing out of bounds, or
//! silently substituting a placeholder value for malformed or truncated
//! input.
//!
//! # Scope (S01)
//!
//! Per the S01 stage brief, this stage decodes REF, VEL, SW, ZDR, PHI, and
//! RHO (as `MomentKind::CorrelationCoefficient`/"CC"). It does not decode
//! "CFP" (clutter filter power removed) or KDP (a derived product, not a
//! wire moment at all), and it does not semantically decode legacy
//! metadata message content (RDA status, VCP definition tables, clutter
//! maps) — those frames are recognized by message type and skipped whole.

mod cursor;
mod ldm;
mod message;
mod message31;

use radar_types::{Site, Sweep, Volume};
use thiserror::Error;

/// A radar site's WGS84 location, as decoded from a Message 31 "VOL" data
/// constant block (the Archive II Volume Header Record only carries the
/// site's ICAO identifier, not its coordinates).
#[derive(Debug)]
pub(crate) struct SiteLocation {
    pub(crate) latitude_deg: f64,
    pub(crate) longitude_deg: f64,
    pub(crate) height_m: f64,
}

/// Errors returned while decoding a NEXRAD Archive II Level II volume.
///
/// Every variant is a structured description of a specific malformed- or
/// truncated-input condition; `decode_volume` never panics on
/// attacker-controlled bytes.
#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("input is too short to contain a NEXRAD Archive II volume header: {len} bytes, need at least {min}")]
    FileTooShort { len: usize, min: usize },

    #[error("bad volume header: expected an \"AR2V00xx.\" filename field, found {found:?}")]
    BadVolumeHeaderMagic { found: Vec<u8> },

    #[error("unrecognized Archive II format version {version:?} in volume header filename")]
    UnknownFormatVersion { version: String },

    #[error("LDM compressed record at offset {offset} is truncated: need {need} more bytes, only {available} available")]
    TruncatedLdmRecord {
        offset: usize,
        need: usize,
        available: usize,
    },

    #[error("LDM compressed record control word at offset {offset} is zero, which is not a valid record length")]
    ZeroLdmRecordLength { offset: usize },

    #[error("corrupt or truncated bzip2 stream in LDM compressed record starting at offset {offset}: {source}")]
    Bzip2Error {
        offset: usize,
        #[source]
        source: std::io::Error,
    },

    #[error("truncated message at decompressed offset {offset}: need {need} bytes, only {available} available")]
    TruncatedMessage {
        offset: usize,
        need: usize,
        available: usize,
    },

    #[error("unsupported message type {message_type} at offset {offset} (only 2, 3, 5, 13, 15, 18, and 31 are supported)")]
    UnsupportedMessageType { offset: usize, message_type: u8 },

    #[error("message 31 at offset {offset} declares an unsupported payload compression indicator {indicator} (only 0/uncompressed is supported)")]
    UnsupportedMessage31Compression { offset: usize, indicator: u8 },

    #[error("message 31 at offset {offset} declares a radial length of {radial_length} bytes, which extends past the end of the decompressed record ({available} bytes available)")]
    Message31RadialLengthOutOfBounds {
        offset: usize,
        radial_length: usize,
        available: usize,
    },

    #[error("data block pointer (value {pointer}) at message offset {offset} points outside the message 31 payload (length {payload_len})")]
    DataBlockPointerOutOfBounds {
        offset: usize,
        pointer: u32,
        payload_len: usize,
    },

    #[error("unrecognized data block type tag {tag:?} at offset {offset}")]
    UnknownDataBlockType { offset: usize, tag: [u8; 4] },

    #[error("invalid data word size {word_size} in data moment block at offset {offset}: must be 8 or 16")]
    InvalidDataWordSize { offset: usize, word_size: u8 },

    #[error("data moment block at offset {offset} declares {gate_count} gates of {word_size}-bit words, extending past the end of the message (payload length {payload_len})")]
    DataMomentOutOfBounds {
        offset: usize,
        gate_count: u16,
        word_size: u8,
        payload_len: usize,
    },

    #[error("data moment block at offset {offset} has a zero scale factor, which would make gate values undefined")]
    InvalidMomentScale { offset: usize },

    #[error("invalid radial status byte {value:#04x} at offset {offset}")]
    InvalidRadialStatus { offset: usize, value: u8 },

    #[error("invalid azimuth resolution spacing byte {value:#04x} at offset {offset}")]
    InvalidAzimuthResolution { offset: usize, value: u8 },

    #[error("non-ASCII bytes in a text field at offset {offset}: {bytes:?}")]
    NonAsciiField { offset: usize, bytes: Vec<u8> },

    #[error("reached the end of input while expecting {need} bytes at offset {offset} ({available} available)")]
    UnexpectedEnd {
        offset: usize,
        need: usize,
        available: usize,
    },

    #[error("volume contains no decodable message 31 radial data")]
    NoRadialData,

    #[error("internal invariant violated: {0}")]
    Internal(String),
}

/// Decode a complete NEXRAD Archive II Level II volume from `bytes`.
///
/// `bytes` should be the entire contents of a `.../SITE_YYYYMMDD_HHMMSS`
/// (or similar) Archive II Level II file. Returns a structured
/// [`DecodeError`] — never panics — on malformed, truncated, or otherwise
/// unsupported input.
pub fn decode_volume(bytes: &[u8]) -> Result<Volume, DecodeError> {
    let header = ldm::parse_volume_header(bytes)?;

    let mut offset = ldm::VOLUME_HEADER_LEN;
    let mut sweeps: Vec<Sweep> = Vec::new();
    let mut site_location: Option<SiteLocation> = None;
    let mut volume_coverage_pattern: Option<u16> = None;

    while offset < bytes.len() {
        let record = ldm::read_ldm_record(bytes, offset)?;
        let decompressed = ldm::decompress_bzip2(record.compressed, record.offset)?;
        let outcome = message::process_record(&decompressed, &mut sweeps)?;

        if outcome.site_location.is_some() {
            site_location = outcome.site_location;
        }
        if outcome.volume_coverage_pattern.is_some() {
            volume_coverage_pattern = outcome.volume_coverage_pattern;
        }

        offset = record.next_offset;
    }

    if sweeps.is_empty() {
        return Err(DecodeError::NoRadialData);
    }
    let site_location = site_location.ok_or(DecodeError::NoRadialData)?;
    let volume_coverage_pattern = volume_coverage_pattern.ok_or(DecodeError::NoRadialData)?;

    Ok(Volume {
        site: Site::new(
            header.icao,
            site_location.latitude_deg,
            site_location.longitude_deg,
            site_location.height_m,
        ),
        start_time: header.start_time,
        volume_coverage_pattern,
        sweeps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_volume_rejects_empty_input() {
        let err = decode_volume(&[]).unwrap_err();
        assert!(matches!(err, DecodeError::FileTooShort { len: 0, .. }));
    }

    #[test]
    fn decode_volume_rejects_truncated_volume_header() {
        let bytes = b"AR2V0006.68";
        let err = decode_volume(bytes).unwrap_err();
        assert!(matches!(err, DecodeError::FileTooShort { .. }));
    }

    #[test]
    fn decode_volume_rejects_bad_magic() {
        let mut bytes = vec![0u8; ldm::VOLUME_HEADER_LEN];
        bytes[0..4].copy_from_slice(b"XXXX");
        let err = decode_volume(&bytes).unwrap_err();
        assert!(matches!(err, DecodeError::BadVolumeHeaderMagic { .. }));
    }

    #[test]
    fn decode_volume_rejects_no_ldm_records() {
        // A structurally valid volume header followed by nothing at all
        // has no radial data.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"AR2V0006.");
        bytes.extend_from_slice(b"001");
        bytes.extend_from_slice(&19876u32.to_be_bytes());
        bytes.extend_from_slice(&233_941u32.to_be_bytes());
        bytes.extend_from_slice(b"KTLX");
        assert_eq!(bytes.len(), ldm::VOLUME_HEADER_LEN);

        let err = decode_volume(&bytes).unwrap_err();
        assert!(matches!(err, DecodeError::NoRadialData));
    }
}
