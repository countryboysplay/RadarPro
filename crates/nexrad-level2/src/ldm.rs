//! Archive II Volume Header Record and LDM Compressed Record framing.

use crate::cursor::Cursor;
use crate::DecodeError;
use radar_types::Timestamp;

/// Length in bytes of the Archive II Volume Header Record.
pub(crate) const VOLUME_HEADER_LEN: usize = 24;

/// Length in bytes of the Volume Header Record's tape-filename field
/// (`"AR2Vxxyy."`).
const FILENAME_LEN: usize = 9;
const EXTENSION_NUMBER_LEN: usize = 3;

/// Documented Archive II format version codes (Archive II/User ICD).
/// Anything else is a version this crate has never been told how to
/// interpret, so it is rejected rather than guessed at.
const KNOWN_FORMAT_VERSIONS: [[u8; 2]; 6] = [*b"02", *b"03", *b"04", *b"05", *b"06", *b"07"];

#[derive(Debug)]
pub(crate) struct VolumeHeader {
    pub(crate) icao: String,
    pub(crate) start_time: Timestamp,
}

/// Parse the 24-byte Archive II Volume Header Record at the start of the
/// file.
pub(crate) fn parse_volume_header(bytes: &[u8]) -> Result<VolumeHeader, DecodeError> {
    if bytes.len() < VOLUME_HEADER_LEN {
        return Err(DecodeError::FileTooShort {
            len: bytes.len(),
            min: VOLUME_HEADER_LEN,
        });
    }

    let mut cursor = Cursor::new(bytes, 0);
    let filename = cursor.take(FILENAME_LEN)?;
    if !filename.starts_with(b"AR2V00") || filename[8] != b'.' {
        return Err(DecodeError::BadVolumeHeaderMagic {
            found: filename.to_vec(),
        });
    }
    let version: [u8; 2] = [filename[6], filename[7]];
    if !KNOWN_FORMAT_VERSIONS.contains(&version) {
        return Err(DecodeError::UnknownFormatVersion {
            version: String::from_utf8_lossy(&version).into_owned(),
        });
    }

    cursor.take(EXTENSION_NUMBER_LEN)?; // extension number: not part of the canonical model

    let modified_julian_date = cursor.read_u32()?;
    let millis_of_day = cursor.read_u32()?;
    let icao = cursor.read_ascii(4)?;

    let start_time = Timestamp::from_nexrad_date_time(modified_julian_date, millis_of_day)
        .ok_or_else(|| {
            DecodeError::Internal(format!(
                "volume header date/time overflow (modified_julian_date={modified_julian_date}, millis_of_day={millis_of_day})"
            ))
        })?;

    Ok(VolumeHeader { icao, start_time })
}

/// One LDM Compressed Record's framing: the byte range of its compressed
/// payload within the file, and where the next record (or EOF) starts.
#[derive(Debug)]
pub(crate) struct LdmRecord<'a> {
    pub(crate) compressed: &'a [u8],
    /// Absolute file offset of `compressed[0]`, for error messages.
    pub(crate) offset: usize,
    /// Absolute file offset immediately after this record.
    pub(crate) next_offset: usize,
}

/// Read one LDM Compressed Record's framing (4-byte signed control word +
/// that many bytes of bzip2-compressed data) starting at `offset`.
pub(crate) fn read_ldm_record(bytes: &[u8], offset: usize) -> Result<LdmRecord<'_>, DecodeError> {
    if offset.checked_add(4).is_none_or(|end| end > bytes.len()) {
        return Err(DecodeError::TruncatedLdmRecord {
            offset,
            need: 4,
            available: bytes.len().saturating_sub(offset),
        });
    }
    let control_word = i32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]);
    let size = ldm_record_size(control_word, offset)?;

    let data_offset = offset + 4;
    let data_end = data_offset
        .checked_add(size)
        .ok_or_else(|| DecodeError::Internal("LDM record size overflow".to_string()))?;
    if data_end > bytes.len() {
        return Err(DecodeError::TruncatedLdmRecord {
            offset,
            need: size,
            available: bytes.len().saturating_sub(data_offset),
        });
    }

    Ok(LdmRecord {
        compressed: &bytes[data_offset..data_end],
        offset: data_offset,
        next_offset: data_end,
    })
}

/// Convert an LDM Compressed Record's signed control word into a byte
/// count. Per the Archive II/User ICD the control word's sign can vary;
/// only its magnitude is the record length. `i32::unsigned_abs` is used
/// (rather than `abs()`) so `i32::MIN` — an otherwise panic-inducing input
/// straight from untrusted bytes — is handled without overflow.
pub(crate) fn ldm_record_size(control_word: i32, offset: usize) -> Result<usize, DecodeError> {
    if control_word == 0 {
        return Err(DecodeError::ZeroLdmRecordLength { offset });
    }
    Ok(control_word.unsigned_abs() as usize)
}

/// Decompress one LDM Compressed Record's bzip2 payload.
pub(crate) fn decompress_bzip2(compressed: &[u8], offset: usize) -> Result<Vec<u8>, DecodeError> {
    use std::io::Read;

    let mut reader = bzip2_rs::DecoderReader::new(compressed);
    let mut decompressed = Vec::new();
    reader
        .read_to_end(&mut decompressed)
        .map_err(|source| DecodeError::Bzip2Error { offset, source })?;
    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ktlx_header_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"AR2V0006.");
        bytes.extend_from_slice(b"684");
        bytes.extend_from_slice(&0x0000_4da4u32.to_be_bytes());
        bytes.extend_from_slice(&0x0003_91d5u32.to_be_bytes());
        bytes.extend_from_slice(b"KTLX");
        bytes
    }

    /// Literal ground-truth bytes from `KTLX20240601_000353_V06`, as given
    /// in the S01 task brief (hand-verified against the Archive II/User
    /// ICD by the orchestrating session before handoff).
    #[test]
    fn parses_ktlx_ground_truth_volume_header() {
        let bytes = ktlx_header_bytes();
        assert_eq!(bytes.len(), VOLUME_HEADER_LEN);

        let header = parse_volume_header(&bytes).expect("valid header");

        assert_eq!(header.icao, "KTLX");
        let civil = header.start_time.to_civil_utc();
        assert_eq!(civil.year, 2024);
        assert_eq!(civil.month, 6);
        assert_eq!(civil.day, 1);
        assert_eq!(civil.hour, 0);
        assert_eq!(civil.minute, 3);
        assert_eq!(civil.second, 53);
    }

    #[test]
    fn rejects_empty_input() {
        let err = parse_volume_header(&[]).unwrap_err();
        assert!(matches!(err, DecodeError::FileTooShort { len: 0, .. }));
    }

    #[test]
    fn rejects_truncated_volume_header() {
        let bytes = &ktlx_header_bytes()[..10];
        let err = parse_volume_header(bytes).unwrap_err();
        assert!(matches!(err, DecodeError::FileTooShort { len: 10, .. }));
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = ktlx_header_bytes();
        bytes[0] = b'X';
        let err = parse_volume_header(&bytes).unwrap_err();
        assert!(matches!(err, DecodeError::BadVolumeHeaderMagic { .. }));
    }

    #[test]
    fn rejects_unknown_format_version() {
        let mut bytes = ktlx_header_bytes();
        bytes[6] = b'9';
        bytes[7] = b'9';
        let err = parse_volume_header(&bytes).unwrap_err();
        assert!(matches!(err, DecodeError::UnknownFormatVersion { .. }));
    }

    #[test]
    fn ldm_record_size_takes_absolute_value_of_signed_control_word() {
        assert_eq!(ldm_record_size(2248, 0).unwrap(), 2248);
        assert_eq!(ldm_record_size(-2248, 0).unwrap(), 2248);
        // i32::MIN must not panic on `.abs()`-style overflow.
        assert_eq!(ldm_record_size(i32::MIN, 0).unwrap(), 1 << 31);
    }

    #[test]
    fn ldm_record_size_rejects_zero() {
        let err = ldm_record_size(0, 42).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::ZeroLdmRecordLength { offset: 42 }
        ));
    }

    #[test]
    fn read_ldm_record_rejects_truncated_control_word() {
        let bytes = [0u8, 0, 0];
        let err = read_ldm_record(&bytes, 0).unwrap_err();
        assert!(matches!(err, DecodeError::TruncatedLdmRecord { .. }));
    }

    #[test]
    fn read_ldm_record_rejects_declared_size_past_end_of_buffer() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&100i32.to_be_bytes()); // claims 100 bytes follow
        bytes.extend_from_slice(&[0u8; 10]); // only 10 actually present
        let err = read_ldm_record(&bytes, 0).unwrap_err();
        assert!(matches!(err, DecodeError::TruncatedLdmRecord { .. }));
    }

    #[test]
    fn decompress_bzip2_rejects_corrupt_magic() {
        // Correct-length but not-actually-bzip2 bytes.
        let garbage = [b'X', b'X', b'h', b'9', 0, 0, 0, 0];
        let err = decompress_bzip2(&garbage, 0).unwrap_err();
        assert!(matches!(err, DecodeError::Bzip2Error { .. }));
    }
}
