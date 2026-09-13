//! Parsing an HRRR `.idx` sidecar file and computing the byte range of one
//! message within the GRIB2 file it describes.
//!
//! # Empirically verified format (2026-09-12, live bucket)
//!
//! The exact same plain-ASCII, one-line-per-message, 1-indexed shape
//! GEFS's `.idx` files use (NOAA's own idx convention is shared tooling
//! across their model products, confirming the S08 stage brief's guess):
//! `N:byte_offset:d=YYYYMMDDHH:VARNAME:LEVEL:step_desc:` -- e.g.:
//!
//! ```text
//! 71:34722112:d=2026091212:TMP:2 m above ground:anl:
//! ```
//!
//! Unlike GEFS's ensemble members, HRRR's real lines carry **no** `ENS=...`
//! annotation at all after `step_desc` (HRRR is deterministic) -- confirmed
//! live: every field's trailing segment is just `anl:` with nothing after
//! the final colon. This crate never depends on that trailing text for
//! anything beyond diagnostics, matching `provider-gefs::idx`'s same
//! stance.
//!
//! The `VAR:LEVEL` naming for 2m temperature (`TMP` / `2 m above ground`)
//! is spelled **identically** to GEFS's -- confirmed live, correcting the
//! S08 stage brief's flagged possibility that it "may not be spelled
//! identically."
//!
//! # Byte range convention
//!
//! Identical to GEFS's: `[this_line's offset, next_line's offset)`, and
//! `[last_offset, content_length)` for the last message in the file.

use crate::error::HrrrError;

/// One parsed line of a `.idx` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdxEntry {
    /// 1-indexed message number within the GRIB2 file.
    pub message_number: u32,
    /// Byte offset of this message's `GRIB` magic within the file.
    pub offset: u64,
    /// The `d=YYYYMMDDHH` reference-time field, verbatim (this crate reads
    /// the authoritative reference time from the decoded GRIB2 Section 1
    /// instead -- see [`crate::decode`] -- so this is kept only for
    /// diagnostics/display).
    pub reference_time_raw: String,
    pub variable: String,
    pub level: String,
    /// Everything after `level` (step description), verbatim, for
    /// diagnostics/display only.
    pub rest: String,
}

/// Parse a `.idx` file's full text into its entries, in file order.
/// `url` is used only to attribute a [`HrrrError::MalformedIdxLine`] to the
/// file it came from.
pub fn parse_idx(url: &str, text: &str) -> Result<Vec<IdxEntry>, HrrrError> {
    let mut entries = Vec::new();
    for (zero_based_line_number, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let entry = parse_idx_line(url, zero_based_line_number + 1, line)?;
        entries.push(entry);
    }
    Ok(entries)
}

fn parse_idx_line(url: &str, line_number: usize, line: &str) -> Result<IdxEntry, HrrrError> {
    let malformed = || HrrrError::MalformedIdxLine {
        url: url.to_string(),
        line_number,
        line: line.to_string(),
    };

    let mut parts = line.splitn(6, ':');
    let message_number: u32 = parts
        .next()
        .ok_or_else(malformed)?
        .parse()
        .map_err(|_| malformed())?;
    let offset: u64 = parts
        .next()
        .ok_or_else(malformed)?
        .parse()
        .map_err(|_| malformed())?;
    let reference_time_raw = parts.next().ok_or_else(malformed)?.to_string();
    let variable = parts.next().ok_or_else(malformed)?.to_string();
    let level = parts.next().ok_or_else(malformed)?.to_string();
    let rest = parts.next().unwrap_or_default().to_string();

    if variable.is_empty() || level.is_empty() {
        return Err(malformed());
    }

    Ok(IdxEntry {
        message_number,
        offset,
        reference_time_raw,
        variable,
        level,
        rest,
    })
}

/// Find the (first) entry matching `variable`/`level` exactly.
pub fn find_message<'a>(
    entries: &'a [IdxEntry],
    variable: &str,
    level: &str,
) -> Option<&'a IdxEntry> {
    entries
        .iter()
        .find(|e| e.variable == variable && e.level == level)
}

/// Compute the half-open byte range `[start, end)` for the message at
/// `entries[position]`. `content_length` is required only when this is the
/// last message in the file.
pub fn byte_range(
    url: &str,
    entries: &[IdxEntry],
    position: usize,
    content_length: Option<u64>,
) -> Result<(u64, u64), HrrrError> {
    let start = entries[position].offset;
    let end = match entries.get(position + 1) {
        Some(next) => next.offset,
        None => content_length.ok_or_else(|| HrrrError::MissingContentLength {
            url: url.to_string(),
        })?,
    };
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real, live-fetched HRRR `.idx` excerpt (2026-09-12 12Z run,
    // `hrrr.t12z.wrfsfcf00.grib2.idx`, lines 64-73) -- note the total
    // absence of any `ENS=`-style trailing annotation, unlike GEFS.
    const REAL_HRRR_IDX_EXCERPT: &str = "64:31850743:d=2026091212:TMP:surface:anl:\n\
65:32605283:d=2026091212:SPFH:surface:anl:\n\
66:32916423:d=2026091212:DPT:surface:anl:\n\
67:33509886:d=2026091212:RH:surface:anl:\n\
68:33940274:d=2026091212:UGRD:10 m above ground:anl:\n\
69:34320715:d=2026091212:VGRD:10 m above ground:anl:\n\
70:34720313:d=2026091212:SNOD:surface:anl:\n\
71:34722112:d=2026091212:TMP:2 m above ground:anl:\n\
72:35918739:d=2026091212:POT:2 m above ground:anl:\n\
73:37094201:d=2026091212:SPFH:2 m above ground:anl:\n";

    #[test]
    fn parses_a_real_captured_idx_excerpt() {
        let entries = parse_idx("https://example.com/x.idx", REAL_HRRR_IDX_EXCERPT).unwrap();
        assert_eq!(entries.len(), 10);
        assert_eq!(entries[7].message_number, 71);
        assert_eq!(entries[7].offset, 34_722_112);
        assert_eq!(entries[7].variable, "TMP");
        assert_eq!(entries[7].level, "2 m above ground");
        assert_eq!(entries[7].rest, "anl:");
    }

    #[test]
    fn find_message_locates_tmp_2m() {
        let entries = parse_idx("u", REAL_HRRR_IDX_EXCERPT).unwrap();
        let found = find_message(&entries, "TMP", "2 m above ground").unwrap();
        assert_eq!(found.message_number, 71);
    }

    #[test]
    fn find_message_returns_none_for_absent_field() {
        let entries = parse_idx("u", REAL_HRRR_IDX_EXCERPT).unwrap();
        assert!(find_message(&entries, "TMP", "surface").is_some()); // present at index 0
        assert!(find_message(&entries, "NOSUCHVAR", "2 m above ground").is_none());
    }

    #[test]
    fn byte_range_uses_next_entrys_offset_when_not_last() {
        let entries = parse_idx("u", REAL_HRRR_IDX_EXCERPT).unwrap();
        // Index 7 is the TMP/2m entry (message 71).
        let (start, end) = byte_range("u", &entries, 7, None).unwrap();
        assert_eq!(start, 34_722_112);
        assert_eq!(end, 35_918_739);
        // Matches the real, live-verified message length for this exact
        // captured offset pair (also the size of this crate's own
        // committed fixture, `fixtures/provider-hrrr/hrrr_t12z_f00_tmp2m.grib2`).
        assert_eq!(end - start, 1_196_627);
    }

    #[test]
    fn byte_range_for_the_last_message_needs_content_length() {
        let entries = parse_idx("u", REAL_HRRR_IDX_EXCERPT).unwrap();
        let last = entries.len() - 1;
        let err = byte_range("u", &entries, last, None).unwrap_err();
        assert!(matches!(err, HrrrError::MissingContentLength { .. }));

        let (start, end) = byte_range("u", &entries, last, Some(50_000_000)).unwrap();
        assert_eq!(start, 37_094_201);
        assert_eq!(end, 50_000_000);
    }

    #[test]
    fn malformed_line_is_a_structured_error_not_a_panic() {
        let err = parse_idx("https://example.com/x.idx", "not a valid idx line").unwrap_err();
        match err {
            HrrrError::MalformedIdxLine { line_number, .. } => assert_eq!(line_number, 1),
            other => panic!("expected MalformedIdxLine, got {other:?}"),
        }
    }

    #[test]
    fn blank_lines_are_skipped() {
        let text = "1:0:d=2026091212:VIS:surface:anl:\n\n\n";
        let entries = parse_idx("u", text).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn hrrr_lines_have_no_ens_annotation_unlike_gefs() {
        let entries = parse_idx("u", REAL_HRRR_IDX_EXCERPT).unwrap();
        assert!(entries.iter().all(|e| !e.rest.contains("ENS")));
    }
}
