//! Message Type 31 ("Digital Radar Data Generic Format") decoding: the
//! Data Header Block, the VOL/ELV/RAD constant blocks, and per-moment Data
//! Moment blocks (Archive II/User ICD Tables XVII-A, XVII-B, XVII-E,
//! XVII-F, XVII-H).

use crate::cursor::Cursor;
use crate::{DecodeError, SiteLocation};
use radar_types::{
    AzimuthResolution, GateValue, Moment, MomentKind, MomentMap, Radial, RadialStatus,
    RadialStatusKind, Timestamp,
};

/// Length of the fixed portion of the Data Header Block, up through and
/// including the "Radial Length" field (Table XVII-A, bytes 0-19).
const DATA_HEADER_PROBE_LEN: usize = 20;

/// Result of decoding one Message 31 record.
#[derive(Debug)]
pub(crate) struct ParsedMessage31 {
    pub(crate) radial: Radial,
    pub(crate) elevation_number: u8,
    /// Present only when this radial carried a "VOL" data constant block.
    pub(crate) site_location: Option<SiteLocation>,
    /// Present only when this radial carried a "VOL" data constant block.
    pub(crate) volume_coverage_pattern: Option<u16>,
    /// Total bytes of this message's payload starting at the Data Header
    /// Block (the wire "Radial Length" field) — how far the caller must
    /// advance to reach the next message.
    pub(crate) payload_len: usize,
}

/// Parse a Message 31 record. `buf` is the whole decompressed LDM record;
/// `data_header_start` is the offset within `buf` where the Data Header
/// Block begins (immediately after the 12-byte CTM prefix + 16-byte
/// Generic Message Header).
pub(crate) fn parse_message31(
    buf: &[u8],
    data_header_start: usize,
) -> Result<ParsedMessage31, DecodeError> {
    if data_header_start > buf.len() {
        return Err(DecodeError::Internal(
            "message 31 data header starts beyond the end of the record".to_string(),
        ));
    }
    let remaining = &buf[data_header_start..];

    // First pass: read only far enough to learn the message's own
    // authoritative length (the "Radial Length" field), bounds-checked
    // against what's actually left in the buffer.
    let mut probe = Cursor::new(remaining, data_header_start);
    probe.take(4)?; // Radar Identifier (ASCII; not cross-checked against the volume header)
    let collection_millis_of_day = probe.read_u32()?;
    let collection_julian_date = probe.read_u16()?;
    probe.take(2)?; // Azimuth Number (re-read below)
    probe.take(4)?; // Azimuth Angle (re-read below)
    let compression_indicator = probe.read_u8()?;
    probe.take(1)?; // spare
    let radial_length = usize::from(probe.read_u16()?);
    debug_assert_eq!(probe.position(), DATA_HEADER_PROBE_LEN);

    // Message 31's own payload is never itself compressed in current
    // archives; we check rather than assume, per the ICD's own caveat
    // that other values are reserved for future use.
    if compression_indicator != 0 {
        return Err(DecodeError::UnsupportedMessage31Compression {
            offset: data_header_start + 16,
            indicator: compression_indicator,
        });
    }

    if radial_length > remaining.len() {
        return Err(DecodeError::Message31RadialLengthOutOfBounds {
            offset: data_header_start,
            radial_length,
            available: remaining.len(),
        });
    }
    let payload = &remaining[..radial_length];

    let collection_time = Timestamp::from_nexrad_date_time(
        u32::from(collection_julian_date),
        collection_millis_of_day,
    )
    .ok_or_else(|| {
        DecodeError::Internal(format!(
            "message 31 collection time overflow at offset {data_header_start}"
        ))
    })?;

    // Second pass: re-read from the start of `payload`, now bounds-checked
    // against the message's own declared length (`radial_length`) rather
    // than just however much of the record happens to remain.
    let mut cursor = Cursor::new(payload, data_header_start);
    cursor.take(4)?; // Radar Identifier
    cursor.take(4)?; // Collection Time (ms of day)
    cursor.take(2)?; // Modified Julian Date
    let azimuth_number = cursor.read_u16()?;
    let azimuth_angle_deg = cursor.read_f32()?;
    cursor.take(1)?; // Compression Indicator
    cursor.take(1)?; // spare
    cursor.take(2)?; // Radial Length

    let azimuth_resolution = match cursor.read_u8()? {
        1 => AzimuthResolution::Half,
        2 => AzimuthResolution::One,
        other => {
            return Err(DecodeError::InvalidAzimuthResolution {
                offset: cursor.absolute_position() - 1,
                value: other,
            })
        }
    };

    let radial_status_byte = cursor.read_u8()?;
    let radial_status = parse_radial_status(radial_status_byte, cursor.absolute_position() - 1)?;

    let elevation_number = cursor.read_u8()?;
    cursor.take(1)?; // Cut Sector Number: not retained in the canonical model this stage
    let elevation_angle_deg = cursor.read_f32()?;
    cursor.take(1)?; // Radial Spot Blanking Status: not retained this stage
    cursor.take(1)?; // Azimuth Indexing Mode: not retained this stage

    let data_block_count = usize::from(cursor.read_u16()?);
    let mut pointers = Vec::with_capacity(data_block_count);
    for _ in 0..data_block_count {
        pointers.push(cursor.read_u32()? as usize);
    }

    let mut site_location = None;
    let mut volume_coverage_pattern = None;
    let mut moments: MomentMap = MomentMap::new();

    for pointer in pointers {
        if pointer == 0 {
            // Only valid for per-moment pointers: this moment is absent
            // on this radial.
            continue;
        }
        let tag = require_block(payload, pointer, 4, data_header_start)?;
        match tag[0] {
            b'R' => match &tag[1..4] {
                b"VOL" => {
                    let vol = parse_vol_block(payload, pointer, data_header_start)?;
                    site_location = Some(SiteLocation {
                        latitude_deg: f64::from(vol.latitude_deg),
                        longitude_deg: f64::from(vol.longitude_deg),
                        height_m: f64::from(vol.site_height_m),
                    });
                    volume_coverage_pattern = Some(vol.volume_coverage_pattern);
                }
                b"ELV" => {
                    parse_elv_block(payload, pointer, data_header_start)?;
                }
                b"RAD" => {
                    parse_rad_block(payload, pointer, data_header_start)?;
                }
                _ => {
                    return Err(DecodeError::UnknownDataBlockType {
                        offset: data_header_start + pointer,
                        tag: [tag[0], tag[1], tag[2], tag[3]],
                    })
                }
            },
            b'D' => {
                let name = [tag[1], tag[2], tag[3]];
                if let Some(kind) = moment_kind_for_wire_name(&name) {
                    let moment = parse_data_moment_block(payload, pointer, data_header_start)?;
                    moments.insert(kind, moment);
                }
                // Else: a structurally valid data-moment block this stage
                // does not decode ("CFP", or any name outside the S01
                // scope) — skipped, not an error.
            }
            _ => {
                return Err(DecodeError::UnknownDataBlockType {
                    offset: data_header_start + pointer,
                    tag: [tag[0], tag[1], tag[2], tag[3]],
                })
            }
        }
    }

    let radial = Radial {
        azimuth_number,
        azimuth_angle_deg,
        azimuth_resolution,
        elevation_angle_deg,
        radial_status,
        collection_time,
        moments,
    };

    Ok(ParsedMessage31 {
        radial,
        elevation_number,
        site_location,
        volume_coverage_pattern,
        payload_len: radial_length,
    })
}

fn parse_radial_status(value: u8, offset: usize) -> Result<RadialStatus, DecodeError> {
    let bad_data = value & 0x80 != 0;
    let kind = match value & 0x7F {
        0 => RadialStatusKind::StartOfElevation,
        1 => RadialStatusKind::Intermediate,
        2 => RadialStatusKind::EndOfElevation,
        3 => RadialStatusKind::StartOfVolume,
        4 => RadialStatusKind::EndOfVolume,
        5 => RadialStatusKind::StartOfElevationLastInVcp,
        _ => return Err(DecodeError::InvalidRadialStatus { offset, value }),
    };
    Ok(RadialStatus { kind, bad_data })
}

fn moment_kind_for_wire_name(name: &[u8; 3]) -> Option<MomentKind> {
    match name {
        b"REF" => Some(MomentKind::Reflectivity),
        b"VEL" => Some(MomentKind::Velocity),
        b"SW " => Some(MomentKind::SpectrumWidth),
        b"ZDR" => Some(MomentKind::DifferentialReflectivity),
        b"PHI" => Some(MomentKind::DifferentialPhase),
        b"RHO" => Some(MomentKind::CorrelationCoefficient),
        // "CFP" (clutter filter power removed) and anything else: out of
        // scope for this stage.
        _ => None,
    }
}

/// Bounds-check that `len` bytes are available starting at `pointer`
/// within `payload`, returning that sub-slice.
fn require_block(
    payload: &[u8],
    pointer: usize,
    len: usize,
    data_header_start: usize,
) -> Result<&[u8], DecodeError> {
    let end = pointer
        .checked_add(len)
        .ok_or_else(|| DecodeError::Internal("data block length overflow".to_string()))?;
    if end > payload.len() {
        return Err(DecodeError::DataBlockPointerOutOfBounds {
            offset: data_header_start,
            pointer: pointer as u32,
            payload_len: payload.len(),
        });
    }
    Ok(&payload[pointer..end])
}

struct VolConstants {
    latitude_deg: f32,
    longitude_deg: f32,
    site_height_m: f32,
    volume_coverage_pattern: u16,
}

/// Volume Data Constant Block ("VOL", Table XVII-E), 52 bytes.
fn parse_vol_block(
    payload: &[u8],
    pointer: usize,
    data_header_start: usize,
) -> Result<VolConstants, DecodeError> {
    const VOL_BLOCK_LEN: usize = 52;
    let block = require_block(payload, pointer, VOL_BLOCK_LEN, data_header_start)?;
    let mut cursor = Cursor::new(block, data_header_start + pointer);

    cursor.take(4)?; // "RVOL"
    cursor.take(2)?; // LRTUP (block size; bounds already enforced structurally above)
    cursor.take(1)?; // major version
    cursor.take(1)?; // minor version
    let latitude_deg = cursor.read_f32()?;
    let longitude_deg = cursor.read_f32()?;
    let site_height_raw = cursor.read_i16()?;
    cursor.take(2)?; // tower height
    cursor.take(4)?; // calibration constant
    cursor.take(4)?; // horizontal tx power
    cursor.take(4)?; // vertical tx power
    cursor.take(4)?; // system differential reflectivity
    cursor.take(4)?; // initial system differential phase
    let volume_coverage_pattern = cursor.read_u16()?;
    // Remaining fields (processing status, ZDR bias estimate, spare) are
    // not retained by this stage's canonical model.

    Ok(VolConstants {
        latitude_deg,
        longitude_deg,
        site_height_m: f32::from(site_height_raw),
        volume_coverage_pattern,
    })
}

/// Elevation Data Constant Block ("ELV", Table XVII-F), 12 bytes.
///
/// Not retained by this stage's canonical model (`radar-types::Sweep` only
/// carries the elevation angle from the Data Header Block itself); parsed
/// only to bounds-check its declared extent and confirm the tag matches.
fn parse_elv_block(
    payload: &[u8],
    pointer: usize,
    data_header_start: usize,
) -> Result<(), DecodeError> {
    const ELV_BLOCK_LEN: usize = 12;
    let block = require_block(payload, pointer, ELV_BLOCK_LEN, data_header_start)?;
    let mut cursor = Cursor::new(block, data_header_start + pointer);
    cursor.take(4)?; // "RELV"
    cursor.take(2)?; // LRTUP
    cursor.take(2)?; // ATMOS (scaled two-way atmospheric attenuation)
    cursor.take(4)?; // calibration constant
    Ok(())
}

/// Radial Data Constant Block ("RAD", Table XVII-H), 28 bytes.
///
/// Not retained by this stage's canonical model; parsed only to
/// bounds-check its declared extent and confirm the tag matches.
fn parse_rad_block(
    payload: &[u8],
    pointer: usize,
    data_header_start: usize,
) -> Result<(), DecodeError> {
    const RAD_BLOCK_LEN: usize = 28;
    let block = require_block(payload, pointer, RAD_BLOCK_LEN, data_header_start)?;
    let mut cursor = Cursor::new(block, data_header_start + pointer);
    cursor.take(4)?; // "RRAD"
    cursor.take(2)?; // LRTUP
    cursor.take(2)?; // unambiguous range
    cursor.take(4)?; // noise level horizontal
    cursor.take(4)?; // noise level vertical
    cursor.take(2)?; // nyquist velocity
    cursor.take(2)?; // radial flags (spare)
    cursor.take(4)?; // calibration constant horizontal
    cursor.take(4)?; // calibration constant vertical
    Ok(())
}

/// Fixed length of a Data Moment Block's generic header (Table XVII-B,
/// bytes 0-27), before the gate data itself.
const DATA_MOMENT_HEADER_LEN: usize = 28;

/// Data Moment Block (Table XVII-B): generic header shared by REF, VEL,
/// SW, ZDR, PHI, and RHO.
fn parse_data_moment_block(
    payload: &[u8],
    pointer: usize,
    data_header_start: usize,
) -> Result<Moment, DecodeError> {
    let header = require_block(payload, pointer, DATA_MOMENT_HEADER_LEN, data_header_start)?;
    let mut cursor = Cursor::new(header, data_header_start + pointer);

    cursor.take(4)?; // Data Block Type + Data Moment Name (already dispatched by caller)
    cursor.take(4)?; // reserved
    let gate_count = cursor.read_u16()?;
    let first_gate_range_raw = cursor.read_i16()?;
    let gate_spacing_raw = cursor.read_i16()?;
    cursor.take(2)?; // TOVER
    cursor.take(2)?; // SNR threshold
    cursor.take(1)?; // control flags
    let data_word_size = cursor.read_u8()?;
    let scale = cursor.read_f32()?;
    let offset = cursor.read_f32()?;

    let bytes_per_gate = match data_word_size {
        8 => 1usize,
        16 => 2usize,
        other => {
            return Err(DecodeError::InvalidDataWordSize {
                offset: data_header_start + pointer + 19,
                word_size: other,
            })
        }
    };

    if scale == 0.0 {
        return Err(DecodeError::InvalidMomentScale {
            offset: data_header_start + pointer + 20,
        });
    }

    let gate_count_usize = usize::from(gate_count);
    let gates_len_bytes = gate_count_usize
        .checked_mul(bytes_per_gate)
        .ok_or_else(|| {
            DecodeError::Internal("data moment gate data length overflow".to_string())
        })?;
    let gate_data_start = pointer
        .checked_add(DATA_MOMENT_HEADER_LEN)
        .ok_or_else(|| DecodeError::Internal("data moment gate data start overflow".to_string()))?;
    let gate_data_end = gate_data_start
        .checked_add(gates_len_bytes)
        .ok_or_else(|| DecodeError::Internal("data moment gate data end overflow".to_string()))?;
    if gate_data_end > payload.len() {
        return Err(DecodeError::DataMomentOutOfBounds {
            offset: data_header_start + pointer,
            gate_count,
            word_size: data_word_size,
            payload_len: payload.len(),
        });
    }
    let gate_bytes = &payload[gate_data_start..gate_data_end];

    let mut gates = Vec::with_capacity(gate_count_usize);
    for i in 0..gate_count_usize {
        let raw: u16 = if bytes_per_gate == 1 {
            u16::from(gate_bytes[i])
        } else {
            u16::from_be_bytes([gate_bytes[2 * i], gate_bytes[2 * i + 1]])
        };
        gates.push(decode_gate_value(raw, scale, offset));
    }

    Ok(Moment {
        first_gate_range_km: f32::from(first_gate_range_raw) * 0.001,
        gate_spacing_km: f32::from(gate_spacing_raw) * 0.001,
        scale,
        offset,
        gates,
    })
}

/// Decode one raw gate value per the Archive II/User ICD's universal
/// convention: `0` is "below signal threshold" (missing), `1` is
/// range-folded, and `>= 2` converts to physical units via this moment's
/// own scale/offset (`(raw - offset) / scale`) — never a hardcoded
/// typical scale/offset, since the ICD documents these as varying radial
/// to radial.
pub(crate) fn decode_gate_value(raw: u16, scale: f32, offset: f32) -> GateValue {
    match raw {
        0 => GateValue::Missing,
        1 => GateValue::RangeFolded,
        n => GateValue::Value((f32::from(n) - offset) / scale),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_gate_value_zero_is_missing() {
        assert!(matches!(
            decode_gate_value(0, 2.0, 66.0),
            GateValue::Missing
        ));
    }

    #[test]
    fn decode_gate_value_one_is_range_folded() {
        assert!(matches!(
            decode_gate_value(1, 2.0, 66.0),
            GateValue::RangeFolded
        ));
    }

    #[test]
    fn decode_gate_value_reflectivity_scale_offset() {
        // REF: offset=66, scale=2 -> N=2 => -32.0 dBZ, N=255 => 94.5 dBZ
        // (ICD Table XVII-I documented range), using the formula on
        // per-block scale/offset values (never hardcoded).
        match decode_gate_value(2, 2.0, 66.0) {
            GateValue::Value(v) => assert!((v - (-32.0)).abs() < 1e-4),
            other => panic!("expected Value, got {other:?}"),
        }
        match decode_gate_value(255, 2.0, 66.0) {
            GateValue::Value(v) => assert!((v - 94.5).abs() < 1e-4),
            other => panic!("expected Value, got {other:?}"),
        }
    }

    #[test]
    fn decode_gate_value_velocity_scale_offset() {
        // VEL: offset=129, scale=2 -> N=2 => -63.5 m/s, N=255 => 63.0 m/s.
        match decode_gate_value(2, 2.0, 129.0) {
            GateValue::Value(v) => assert!((v - (-63.5)).abs() < 1e-4),
            other => panic!("expected Value, got {other:?}"),
        }
        match decode_gate_value(255, 2.0, 129.0) {
            GateValue::Value(v) => assert!((v - 63.0).abs() < 1e-4),
            other => panic!("expected Value, got {other:?}"),
        }
    }

    #[test]
    fn moment_kind_for_wire_name_maps_documented_moments() {
        assert_eq!(
            moment_kind_for_wire_name(b"REF"),
            Some(MomentKind::Reflectivity)
        );
        assert_eq!(
            moment_kind_for_wire_name(b"VEL"),
            Some(MomentKind::Velocity)
        );
        assert_eq!(
            moment_kind_for_wire_name(b"SW "),
            Some(MomentKind::SpectrumWidth)
        );
        assert_eq!(
            moment_kind_for_wire_name(b"ZDR"),
            Some(MomentKind::DifferentialReflectivity)
        );
        assert_eq!(
            moment_kind_for_wire_name(b"PHI"),
            Some(MomentKind::DifferentialPhase)
        );
        assert_eq!(
            moment_kind_for_wire_name(b"RHO"),
            Some(MomentKind::CorrelationCoefficient)
        );
    }

    #[test]
    fn moment_kind_for_wire_name_excludes_cfp_and_unknown_names() {
        assert_eq!(moment_kind_for_wire_name(b"CFP"), None);
        assert_eq!(moment_kind_for_wire_name(b"KDP"), None);
        assert_eq!(moment_kind_for_wire_name(b"???"), None);
    }

    #[test]
    fn parse_radial_status_decodes_kind_and_bad_data_bit() {
        let status = parse_radial_status(0x00, 0).unwrap();
        assert_eq!(status.kind, RadialStatusKind::StartOfElevation);
        assert!(!status.bad_data);

        let status = parse_radial_status(0x85, 0).unwrap();
        assert_eq!(status.kind, RadialStatusKind::StartOfElevationLastInVcp);
        assert!(status.bad_data);
    }

    #[test]
    fn parse_radial_status_rejects_undocumented_values() {
        let err = parse_radial_status(6, 0).unwrap_err();
        assert!(matches!(err, DecodeError::InvalidRadialStatus { .. }));
    }

    /// Build a minimal, self-consistent Message 31 payload (Data Header
    /// Block + one REF Data Moment Block, no VOL/ELV/RAD blocks) to
    /// exercise `parse_message31` directly on a synthetic buffer.
    fn build_minimal_message31(gates: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"KTLX"); // Radar Identifier
        buf.extend_from_slice(&233_941u32.to_be_bytes()); // Collection Time
        buf.extend_from_slice(&19876u16.to_be_bytes()); // Modified Julian Date
        buf.extend_from_slice(&1u16.to_be_bytes()); // Azimuth Number
        buf.extend_from_slice(&0.0f32.to_be_bytes()); // Azimuth Angle
        buf.push(0); // Compression Indicator
        buf.push(0); // spare
        let radial_length_pos = buf.len();
        buf.extend_from_slice(&0u16.to_be_bytes()); // Radial Length (patched below)
        buf.push(1); // Azimuth Resolution Spacing (0.5 deg)
        buf.push(0x03); // Radial Status: start of volume
        buf.push(1); // Elevation Number
        buf.push(0); // Cut Sector Number
        buf.extend_from_slice(&0.5f32.to_be_bytes()); // Elevation Angle
        buf.push(0); // Radial Spot Blanking Status
        buf.push(0); // Azimuth Indexing Mode
        buf.extend_from_slice(&1u16.to_be_bytes()); // Data Block Count = 1

        let pointer_pos = buf.len();
        buf.extend_from_slice(&0u32.to_be_bytes()); // pointer to the REF block (patched below)

        let ref_block_start = buf.len();
        buf.extend_from_slice(b"DREF");
        buf.extend_from_slice(&0u32.to_be_bytes()); // reserved
        buf.extend_from_slice(&(gates.len() as u16).to_be_bytes()); // NG
        buf.extend_from_slice(&1000i16.to_be_bytes()); // first gate range (1.0 km)
        buf.extend_from_slice(&250i16.to_be_bytes()); // gate spacing (0.25 km)
        buf.extend_from_slice(&0u16.to_be_bytes()); // TOVER
        buf.extend_from_slice(&0i16.to_be_bytes()); // SNR threshold
        buf.push(0); // control flags
        buf.push(8); // data word size
        buf.extend_from_slice(&2.0f32.to_be_bytes()); // scale
        buf.extend_from_slice(&66.0f32.to_be_bytes()); // offset
        buf.extend_from_slice(gates);

        let radial_length = (buf.len()) as u16;
        buf[radial_length_pos..radial_length_pos + 2].copy_from_slice(&radial_length.to_be_bytes());
        let pointer = ref_block_start as u32;
        buf[pointer_pos..pointer_pos + 4].copy_from_slice(&pointer.to_be_bytes());

        buf
    }

    #[test]
    fn parse_message31_decodes_synthetic_ref_moment() {
        let gates = [0u8, 1, 2, 255];
        let buf = build_minimal_message31(&gates);

        let parsed = parse_message31(&buf, 0).expect("valid synthetic message 31");
        assert_eq!(parsed.elevation_number, 1);
        assert_eq!(parsed.radial.azimuth_number, 1);
        assert_eq!(parsed.payload_len, buf.len());

        let ref_moment = parsed
            .radial
            .moments
            .get(&MomentKind::Reflectivity)
            .expect("REF moment present");
        assert_eq!(ref_moment.gates.len(), 4);
        assert!(matches!(ref_moment.gates[0], GateValue::Missing));
        assert!(matches!(ref_moment.gates[1], GateValue::RangeFolded));
        match ref_moment.gates[2] {
            GateValue::Value(v) => assert!((v - (-32.0)).abs() < 1e-4),
            other => panic!("expected Value, got {other:?}"),
        }
        match ref_moment.gates[3] {
            GateValue::Value(v) => assert!((v - 94.5).abs() < 1e-4),
            other => panic!("expected Value, got {other:?}"),
        }
    }

    #[test]
    fn parse_message31_rejects_radial_length_past_buffer_end() {
        let gates = [0u8, 1, 2, 255];
        let mut buf = build_minimal_message31(&gates);
        // Patch the Radial Length field to claim far more bytes than the
        // buffer actually has.
        let inflated = (buf.len() as u16) + 1000;
        buf[18..20].copy_from_slice(&inflated.to_be_bytes());

        let err = parse_message31(&buf, 0).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::Message31RadialLengthOutOfBounds { .. }
        ));
    }

    #[test]
    fn parse_message31_rejects_data_moment_extending_past_buffer() {
        let gates = [0u8, 1, 2, 255];
        let mut buf = build_minimal_message31(&gates);
        // Find the REF block's NG field (right after "DREF" + 4 reserved
        // bytes) and inflate it so the declared gate data runs past the
        // buffer end, without changing Radial Length.
        let ref_block_start = buf.len() - DATA_MOMENT_HEADER_LEN - gates.len();
        let ng_pos = ref_block_start + 8;
        buf[ng_pos..ng_pos + 2].copy_from_slice(&2000u16.to_be_bytes());

        let err = parse_message31(&buf, 0).unwrap_err();
        assert!(matches!(err, DecodeError::DataMomentOutOfBounds { .. }));
    }

    #[test]
    fn parse_message31_rejects_unsupported_compression_indicator() {
        let gates = [0u8, 1, 2, 255];
        let mut buf = build_minimal_message31(&gates);
        buf[16] = 1; // claim BZIP2-compressed message 31 payload
        let err = parse_message31(&buf, 0).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::UnsupportedMessage31Compression { .. }
        ));
    }

    #[test]
    fn parse_message31_rejects_invalid_data_word_size() {
        let gates = [0u8, 1, 2, 255];
        let mut buf = build_minimal_message31(&gates);
        let ref_block_start = buf.len() - DATA_MOMENT_HEADER_LEN - gates.len();
        buf[ref_block_start + 19] = 12; // neither 8 nor 16
        let err = parse_message31(&buf, 0).unwrap_err();
        assert!(matches!(err, DecodeError::InvalidDataWordSize { .. }));
    }
}
