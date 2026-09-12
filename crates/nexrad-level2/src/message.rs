//! Generic Message Header parsing and per-record message dispatch.
//!
//! Every message (legacy or Message 31) starts with a 12-byte all-zero CTM
//! (compatibility) prefix followed by a 16-byte Generic Message Header.
//! Legacy metadata message types occupy a fixed 2432-byte frame that this
//! stage does not semantically decode (see `GLOBAL_CONTRACT.md`/S01
//! scope); Message 31 is decoded fully by [`crate::message31`].

use crate::cursor::Cursor;
use crate::{message31, DecodeError, SiteLocation};
use radar_types::Sweep;

const CTM_PREFIX_LEN: usize = 12;
const MESSAGE_HEADER_LEN: usize = 16;
/// Combined length of the CTM prefix and Generic Message Header that
/// precedes every message, legacy or Message 31.
const MESSAGE_PREFIX_LEN: usize = CTM_PREFIX_LEN + MESSAGE_HEADER_LEN;
/// Fixed on-disk size of a legacy metadata message frame, CTM prefix and
/// header included (Archive II/User ICD §7.3.5).
const LEGACY_FRAME_LEN: usize = 2432;

/// The Generic Message Header (Archive II/User ICD Table II), starting
/// immediately after the 12-byte CTM prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GenericMessageHeader {
    pub(crate) size_halfwords: u16,
    pub(crate) redundant_channel: u8,
    pub(crate) message_type: u8,
    pub(crate) id_sequence_number: u16,
    pub(crate) julian_date: u16,
    pub(crate) millis_of_day: u32,
    pub(crate) segment_count: u16,
    pub(crate) segment_number: u16,
}

pub(crate) fn parse_generic_message_header(
    cursor: &mut Cursor<'_>,
) -> Result<GenericMessageHeader, DecodeError> {
    Ok(GenericMessageHeader {
        size_halfwords: cursor.read_u16()?,
        redundant_channel: cursor.read_u8()?,
        message_type: cursor.read_u8()?,
        id_sequence_number: cursor.read_u16()?,
        julian_date: cursor.read_u16()?,
        millis_of_day: cursor.read_u32()?,
        segment_count: cursor.read_u16()?,
        segment_number: cursor.read_u16()?,
    })
}

/// Legacy metadata message types that occupy a fixed 2432-byte frame and
/// are only skipped, never semantically decoded, this stage: RDA status
/// (2), performance/maintenance data (3), volume coverage pattern data
/// (5), clutter filter bypass map (13), clutter map data (15), and
/// clutter filter map (18).
///
/// Message type `0` is included deliberately: the Archive II Metadata
/// Record reserves a *fixed* number of 2432-byte slots per legacy message
/// type regardless of how many that type's current content actually
/// needs (e.g. 77 slots reserved for type 15), and every unused slot in
/// that reservation is zero-filled — an all-zero CTM prefix, header
/// (message type 0, zero segments), and payload. This is not inferred:
/// it was confirmed by hex-inspecting both real, ground-truthed fixtures
/// (`KTLX20240601_000353_V06`, `KFTG20240601_000116_V06`) byte-for-byte,
/// where the reserved-but-unused slots between each real legacy message
/// and the next are exactly these all-zero 2432-byte frames.
fn is_legacy_metadata_message_type(message_type: u8) -> bool {
    matches!(message_type, 0 | 2 | 3 | 5 | 13 | 15 | 18)
}

/// What a decompressed LDM record's Message 31 records (if any) revealed
/// about the volume as a whole.
#[derive(Debug, Default)]
pub(crate) struct RecordOutcome {
    pub(crate) site_location: Option<SiteLocation>,
    pub(crate) volume_coverage_pattern: Option<u16>,
}

/// Walk every message in one decompressed LDM record, appending any
/// Message 31 radial data onto `sweeps` and reporting the site
/// location/VCP number if a Message 31 in this record carried one.
pub(crate) fn process_record(
    buf: &[u8],
    sweeps: &mut Vec<Sweep>,
) -> Result<RecordOutcome, DecodeError> {
    let mut outcome = RecordOutcome::default();
    let mut cursor = Cursor::new(buf, 0);

    while cursor.remaining() > 0 {
        if cursor.remaining() < MESSAGE_PREFIX_LEN {
            return Err(DecodeError::TruncatedMessage {
                offset: cursor.absolute_position(),
                need: MESSAGE_PREFIX_LEN,
                available: cursor.remaining(),
            });
        }

        let message_start = cursor.position();
        cursor.take(CTM_PREFIX_LEN)?;
        let header = parse_generic_message_header(&mut cursor)?;

        if header.message_type == 31 {
            let parsed = message31::parse_message31(buf, cursor.position())?;
            if parsed.site_location.is_some() {
                outcome.site_location = parsed.site_location;
            }
            if parsed.volume_coverage_pattern.is_some() {
                outcome.volume_coverage_pattern = parsed.volume_coverage_pattern;
            }
            let next = cursor.position() + parsed.payload_len;
            push_radial(sweeps, parsed.elevation_number, parsed.radial);
            cursor.seek_to(next)?;
        } else if is_legacy_metadata_message_type(header.message_type) {
            let consumed = cursor.position() - message_start;
            let skip = LEGACY_FRAME_LEN.checked_sub(consumed).ok_or_else(|| {
                DecodeError::Internal(format!(
                    "legacy message frame at offset {message_start} is shorter than its own prefix and header"
                ))
            })?;
            cursor.take(skip)?;
        } else {
            return Err(DecodeError::UnsupportedMessageType {
                offset: message_start,
                message_type: header.message_type,
            });
        }
    }

    Ok(outcome)
}

/// Append a decoded radial to `sweeps`, starting a new [`Sweep`] whenever
/// the elevation number changes from the previous radial.
fn push_radial(sweeps: &mut Vec<Sweep>, elevation_number: u8, radial: radar_types::Radial) {
    if let Some(last) = sweeps.last_mut() {
        if last.elevation_number == elevation_number {
            last.radials.push(radial);
            return;
        }
    }
    sweeps.push(Sweep {
        elevation_number,
        elevation_angle_deg: radial.elevation_angle_deg,
        radials: vec![radial],
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_bytes(message_type: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1234u16.to_be_bytes()); // size (halfwords)
        bytes.push(0); // redundant channel
        bytes.push(message_type);
        bytes.extend_from_slice(&5u16.to_be_bytes()); // id sequence number
        bytes.extend_from_slice(&19876u16.to_be_bytes()); // julian date
        bytes.extend_from_slice(&233_941u32.to_be_bytes()); // millis of day
        bytes.extend_from_slice(&1u16.to_be_bytes()); // segment count
        bytes.extend_from_slice(&1u16.to_be_bytes()); // segment number
        bytes
    }

    #[test]
    fn parses_generic_message_header_fields() {
        let bytes = header_bytes(31);
        assert_eq!(bytes.len(), MESSAGE_HEADER_LEN);
        let mut cursor = Cursor::new(&bytes, 0);
        let header = parse_generic_message_header(&mut cursor).expect("valid header");

        assert_eq!(header.size_halfwords, 1234);
        assert_eq!(header.redundant_channel, 0);
        assert_eq!(header.message_type, 31);
        assert_eq!(header.id_sequence_number, 5);
        assert_eq!(header.julian_date, 19876);
        assert_eq!(header.millis_of_day, 233_941);
        assert_eq!(header.segment_count, 1);
        assert_eq!(header.segment_number, 1);
    }

    #[test]
    fn rejects_truncated_message_header() {
        let bytes = [0u8; 5]; // far short of CTM prefix + header
        let err = process_record(&bytes, &mut Vec::new()).unwrap_err();
        assert!(matches!(err, DecodeError::TruncatedMessage { .. }));
    }

    #[test]
    fn rejects_unsupported_message_type() {
        let mut bytes = vec![0u8; CTM_PREFIX_LEN];
        bytes.extend_from_slice(&header_bytes(99));
        let err = process_record(&bytes, &mut Vec::new()).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::UnsupportedMessageType {
                message_type: 99,
                ..
            }
        ));
    }

    #[test]
    fn legacy_message_type_skips_exactly_one_fixed_frame() {
        let mut bytes = vec![0u8; CTM_PREFIX_LEN];
        bytes.extend_from_slice(&header_bytes(2));
        bytes.resize(LEGACY_FRAME_LEN, 0xAB); // pad out to one full legacy frame
        let outcome = process_record(&bytes, &mut Vec::new()).expect("valid legacy frame");
        assert!(outcome.site_location.is_none());
        assert!(outcome.volume_coverage_pattern.is_none());
    }

    #[test]
    fn zero_padded_legacy_slot_is_skipped_like_any_other_legacy_frame() {
        // Confirmed against real fixtures: unused reserved legacy-message
        // slots in the Metadata Record are all-zero 2432-byte frames
        // (message type 0, zero segments) and must be skipped, not
        // treated as an unsupported message type.
        let bytes = vec![0u8; LEGACY_FRAME_LEN];
        let outcome = process_record(&bytes, &mut Vec::new()).expect("zero-padded slot is valid");
        assert!(outcome.site_location.is_none());
        assert!(outcome.volume_coverage_pattern.is_none());
    }

    #[test]
    fn legacy_message_type_rejects_truncated_frame() {
        let mut bytes = vec![0u8; CTM_PREFIX_LEN];
        bytes.extend_from_slice(&header_bytes(2));
        // Frame is declared as legacy (fixed 2432 bytes) but the buffer
        // ends far short of that.
        let err = process_record(&bytes, &mut Vec::new()).unwrap_err();
        assert!(matches!(err, DecodeError::UnexpectedEnd { .. }));
    }
}
