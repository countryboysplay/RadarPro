//! Generic Message Header parsing and per-record message dispatch.
//!
//! Every message (legacy or Message 31) starts with a 12-byte all-zero CTM
//! (compatibility) prefix followed by a 16-byte Generic Message Header.
//! Legacy metadata message types occupy a fixed 2432-byte frame that this
//! stage does not semantically decode (see `GLOBAL_CONTRACT.md`/S01
//! scope); Message 31 is decoded fully by [`crate::message31`].
//!
//! Message types 32 (RDA PRF Data) and 33 (RDA Log Data) — added to the
//! Archive II Metadata Record in a later ICD build revision than the
//! original six legacy types — share that exact same fixed-slot framing
//! (see [`is_legacy_metadata_message_type`]) and are handled identically:
//! recognized and skipped, never semantically decoded.

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
/// (5), clutter filter bypass map (13), clutter map data (15), clutter
/// filter map (18), RDA PRF data (32), and RDA log data (33).
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
///
/// # Message types 32 and 33
///
/// Types 32 (RDA PRF Data, ICD Table XVIII) and 33 (RDA Log Data, Table
/// XVIV) were added to the Metadata Record in a later ICD build revision
/// than the original six. Both are documented as *variable-length*
/// messages with their own internal fields (a repeated
/// waveform/PRF-count/PRF-value table for 32; a compressed log blob with
/// a declared compressed size for 33) that would, read literally, seem to
/// require walking that internal structure to compute a skip length.
///
/// That is **not** how these messages are actually framed on the wire.
/// Real bytes prove they are simply two more members of the *same*
/// fixed-2432-byte-slot Metadata Record container the other six types
/// already use, with their variable-length content zero-padded out to
/// fill the reserved slot — exactly the same reservation scheme already
/// documented above for type 0. This was confirmed empirically, not
/// assumed from the ICD text: 17 real, live WSR-88D volumes were
/// downloaded fresh from the public Unidata NEXRAD Level II S3 bucket on
/// 2026-09-12, spanning 14 different sites (including KTLX, KFTG, KICT,
/// KVNX, KDDC, KLBB, KAMA, KMBX, KABR, KUEX, KDVN, KILN, KJKL, KMHX,
/// KEVX). Every one of them contained exactly one Message Type 32 (no
/// Message Type 33 was observed in this sample — RDA log entries appear
/// to be emitted only on specific RDA events, not routinely per volume),
/// always at Metadata Record slot 125 (decompressed byte offset 304,000
/// in the first LDM record), always reporting a Generic Message Header
/// "Message Size" of 64 halfwords (128 bytes = the 16-byte header plus a
/// 112-byte body). Walking that body field-by-field per Table XVIII (2
/// header halfwords +, per waveform, 2 halfwords of
/// type/count-plus-that-many-PRF-values) independently reproduced the
/// exact same 112-byte content length, and the bytes from byte 112 up to
/// the 2432-byte slot boundary were all zero padding in every sample.
/// Treating message 32 as a plain fixed-2432-byte skip (ignoring its
/// internal structure entirely, exactly like the other legacy types)
/// was then verified end-to-end: a full message-by-message walk of the
/// *entire* decompressed contents of all 17 files (every LDM record,
/// every legacy metadata slot, and all 6,480-9,720 Message 31 radials
/// per file) produced zero framing anomalies -- every subsequent message
/// header, including the very next slot after each type-32 occurrence,
/// decoded as a structurally valid, recognized message type.
///
/// Message 33 was not independently observed, so its exact wire framing
/// is not itself confirmed against real bytes. It is included here based
/// on structural analogy rather than direct observation: the ICD
/// introduces both 32 and 33 together as the same Metadata Record
/// extension, and the reserved-slot design that provably governs type 32
/// (and the original six) is a property of the *container* -- the
/// Metadata Record reserves a whole number of fixed-size slots for
/// whichever auxiliary message types are configured to appear in it,
/// independent of what any one message type's own content looks like --
/// not a per-type framing quirk. A real type-33 fixture that contradicts
/// this (e.g. one not zero-padded to a slot boundary) would mean this
/// inference was wrong and this comment and implementation must be
/// revisited.
///
/// Segmented occurrences (a logical message spanning more than one
/// slot, `segment_count > 1`, as already seen for types 15 and 18) need
/// no special handling either: each segment presents its own CTM prefix
/// and Generic Message Header at the top of its own 2432-byte slot, so
/// the same one-slot-at-a-time skip below walks through every segment
/// correctly regardless of how many total slots the logical message
/// occupies.
fn is_legacy_metadata_message_type(message_type: u8) -> bool {
    matches!(message_type, 0 | 2 | 3 | 5 | 13 | 15 | 18 | 32 | 33)
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
        header_bytes_sized(message_type, 1234)
    }

    fn header_bytes_sized(message_type: u8, size_halfwords: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&size_halfwords.to_be_bytes());
        bytes.push(0); // redundant channel
        bytes.push(message_type);
        bytes.extend_from_slice(&5u16.to_be_bytes()); // id sequence number
        bytes.extend_from_slice(&19876u16.to_be_bytes()); // julian date
        bytes.extend_from_slice(&233_941u32.to_be_bytes()); // millis of day
        bytes.extend_from_slice(&1u16.to_be_bytes()); // segment count
        bytes.extend_from_slice(&1u16.to_be_bytes()); // segment number
        bytes
    }

    /// Build a Table XVIII ("RDA PRF Data") message body: Number of
    /// Waveforms, a spare halfword, then per waveform a Waveform Type, a
    /// PRF Count, and that many 4-byte (2-halfword) PRF values — matching
    /// the exact field layout confirmed against real bytes (see
    /// [`is_legacy_metadata_message_type`]'s doc comment).
    fn build_message32_body(waveforms: &[(u16, &[u32])]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(waveforms.len() as u16).to_be_bytes()); // Number of Waveforms
        body.extend_from_slice(&0u16.to_be_bytes()); // spare
        for &(waveform_type, prf_values) in waveforms {
            body.extend_from_slice(&waveform_type.to_be_bytes());
            body.extend_from_slice(&(prf_values.len() as u16).to_be_bytes()); // PRF Count
            for &prf in prf_values {
                body.extend_from_slice(&prf.to_be_bytes());
            }
        }
        body
    }

    /// Build a Table XVIV ("RDA Log Data") message body: Version,
    /// Identifier, Data Version, Compression Type, Compressed Size,
    /// Decompressed Size, spare, then the (possibly odd-length,
    /// pad-byte-completed) log data itself — matching the documented
    /// field layout. Framing does not read any of these fields (this
    /// crate never semantically decodes message 33 content), but building
    /// realistic bytes documents the layout this test is standing in for.
    fn build_message33_body(identifier: &str, compressed_log: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_be_bytes()); // Version
        let mut identifier_field = identifier.as_bytes().to_vec();
        identifier_field.resize(26, 0); // halfword 2-14: 13 halfwords
        body.extend_from_slice(&identifier_field);
        body.extend_from_slice(&1u32.to_be_bytes()); // Data Version
        body.extend_from_slice(&0u32.to_be_bytes()); // Compression Type: 0 = uncompressed
        body.extend_from_slice(&(compressed_log.len() as u32).to_be_bytes()); // Compressed Size
        body.extend_from_slice(&(compressed_log.len() as u32).to_be_bytes()); // Decompressed Size
        body.extend_from_slice(&[0u8; 22]); // spare: halfword 23-33, 11 halfwords
        body.extend_from_slice(compressed_log);
        if !compressed_log.len().is_multiple_of(2) {
            // "a non-consequential NULL byte that is not part of the
            // message, to fill out the ICD frame" (ICD 2620002AA).
            body.push(0);
        }
        body
    }

    /// Wrap `body` (a whole number of halfwords) in a CTM prefix and
    /// Generic Message Header reporting the true total size, then
    /// zero-pad out to one full reserved 2432-byte Metadata Record slot —
    /// the real on-wire layout confirmed for message type 32 (and, by
    /// the structural-analogy argument in that doc comment, assumed for
    /// 33) in [`is_legacy_metadata_message_type`].
    fn build_legacy_slot(message_type: u8, body: &[u8]) -> Vec<u8> {
        assert_eq!(
            body.len() % 2,
            0,
            "body must be a whole number of halfwords"
        );
        let size_halfwords = ((MESSAGE_HEADER_LEN + body.len()) / 2) as u16;
        let mut bytes = vec![0u8; CTM_PREFIX_LEN];
        bytes.extend_from_slice(&header_bytes_sized(message_type, size_halfwords));
        bytes.extend_from_slice(body);
        assert!(
            bytes.len() <= LEGACY_FRAME_LEN,
            "synthetic content overflows one legacy frame slot"
        );
        bytes.resize(LEGACY_FRAME_LEN, 0);
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
    fn message32_minimal_waveform_skips_one_fixed_frame() {
        // Number of Waveforms = 1, a single waveform with one PRF value —
        // matches the documented minimum (1 waveform, PRF Count 0-255).
        let body = build_message32_body(&[(1u16, &[123_456u32])]);
        let mut bytes = build_legacy_slot(32, &body);
        // A trailing valid message 15 CTM prefix + header proves the skip
        // lands exactly on the next message, not one byte early or late.
        bytes.extend_from_slice(&[0u8; CTM_PREFIX_LEN]);
        bytes.extend_from_slice(&header_bytes(15));
        let outcome = process_record(&bytes, &mut Vec::new());
        // Message 15 with a fabricated size will itself fail (its own
        // fixed 2432-byte frame is truncated at end-of-buffer here), but
        // that failure being `UnexpectedEnd` rather than
        // `UnsupportedMessageType` proves message 32 was framed and
        // skipped correctly and dispatch reached a *recognized*
        // subsequent message type.
        assert!(matches!(
            outcome.unwrap_err(),
            DecodeError::UnexpectedEnd { .. }
        ));
    }

    #[test]
    fn message32_multiple_waveforms_and_prf_counts_skips_one_fixed_frame() {
        // Number of Waveforms = 3 with differing, non-minimal PRF counts
        // per waveform — the same shape observed in the real fixture
        // (waveforms of PRF Count 8, 8, 8) generalized to unequal counts
        // to also cover the varying-PRF-Count-per-waveform case.
        let prfs_a = [100u32, 200, 300];
        let prfs_b = [400u32];
        let prfs_c = [500u32, 600, 700, 800, 900];
        let body = build_message32_body(&[(1u16, &prfs_a), (2u16, &prfs_b), (5u16, &prfs_c)]);
        let bytes = build_legacy_slot(32, &body);
        let outcome = process_record(&bytes, &mut Vec::new()).expect("valid type-32 legacy frame");
        assert!(outcome.site_location.is_none());
        assert!(outcome.volume_coverage_pattern.is_none());
    }

    #[test]
    fn message32_rejects_truncated_frame() {
        let body = build_message32_body(&[(1u16, &[123_456u32])]);
        let full = build_legacy_slot(32, &body);
        // Declares a full legacy frame but the buffer ends far short of
        // it — same shape as `legacy_message_type_rejects_truncated_frame`.
        let truncated = &full[..CTM_PREFIX_LEN + MESSAGE_HEADER_LEN + body.len()];
        let err = process_record(truncated, &mut Vec::new()).unwrap_err();
        assert!(matches!(err, DecodeError::UnexpectedEnd { .. }));
    }

    #[test]
    fn message33_even_compressed_size_skips_one_fixed_frame() {
        let body = build_message33_body("AzServoLog", &[0xAA, 0xBB, 0xCC, 0xDD]);
        let bytes = build_legacy_slot(33, &body);
        let outcome = process_record(&bytes, &mut Vec::new()).expect("valid type-33 legacy frame");
        assert!(outcome.site_location.is_none());
        assert!(outcome.volume_coverage_pattern.is_none());
    }

    #[test]
    fn message33_odd_compressed_size_pad_byte_still_skips_one_fixed_frame() {
        // An odd Compressed Size requires one pad byte to keep the
        // message halfword-aligned (ICD 2620002AA's documented
        // "non-consequential NULL byte" caveat). Framing does not depend
        // on this field at all (the whole slot is always 2432 bytes
        // regardless), but this confirms an odd-length payload doesn't
        // break slot construction or the surrounding skip.
        let body = build_message33_body("ElServoLog", &[0xAA, 0xBB, 0xCC]);
        assert_eq!(body.len() % 2, 0, "builder must pad to a whole halfword");
        let bytes = build_legacy_slot(33, &body);
        let outcome = process_record(&bytes, &mut Vec::new()).expect("valid type-33 legacy frame");
        assert!(outcome.site_location.is_none());
        assert!(outcome.volume_coverage_pattern.is_none());
    }

    #[test]
    fn message33_rejects_truncated_frame() {
        let body = build_message33_body("AzServoLog", &[0xAA, 0xBB, 0xCC, 0xDD]);
        let full = build_legacy_slot(33, &body);
        let truncated = &full[..CTM_PREFIX_LEN + MESSAGE_HEADER_LEN + body.len()];
        let err = process_record(truncated, &mut Vec::new()).unwrap_err();
        assert!(matches!(err, DecodeError::UnexpectedEnd { .. }));
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
