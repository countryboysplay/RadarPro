//! Parsing a GEFS `.idx` sidecar file and computing the byte range of one
//! message within the GRIB2 file it describes.
//!
//! # Empirically verified format (2026-09-12, live bucket)
//!
//! Plain ASCII text, one line per GRIB2 message, 1-indexed:
//! `N:byte_offset:d=YYYYMMDDHH:VARNAME:LEVEL:step_desc:ENS=...` -- e.g.:
//!
//! ```text
//! 11:4169841:d=2026091212:TMP:2 m above ground:anl:ENS=low-res ctl
//! ```
//!
//! The `ENS=` suffix's exact text varies by member type -- confirmed all
//! three forms live: `ENS=low-res ctl` (the control member, `gec00`),
//! `ENS=+1` (a perturbed member, `gep01`), `ens mean` (no `ENS=` prefix at
//! all, just those two words -- the ensemble mean, `geavg`). This crate
//! never parses that suffix for ensemble identity -- see
//! [`crate::ensemble`], which reads the authoritative ensemble metadata out
//! of the decoded GRIB2 Product Definition Section instead, since a
//! human-readable `.idx` annotation is not something to build correctness
//! on top of.
//!
//! # Byte range convention
//!
//! A message's byte range is `[this_line's offset, next_line's offset)`.
//! The **last** message in a file has no "next line" in the `.idx`, so its
//! range needs the object's total size instead -- confirmed empirically:
//! `[last_offset, content_length)` is exactly the remaining GRIB2 message,
//! ending in the standard `"7777"` end marker at `content_length - 4`.

use crate::error::GefsError;

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
    /// Everything after `level` (step description and the `ENS=`/`ens
    /// mean` annotation), verbatim, for diagnostics/display only.
    pub rest: String,
}

/// Parse a `.idx` file's full text into its entries, in file order.
/// `url` is used only to attribute a [`GefsError::MalformedIdxLine`] to the
/// file it came from.
///
/// Blank lines are skipped (observed at least a trailing blank line in some
/// real `.idx` files); any non-blank line that does not match the expected
/// `N:offset:d=...:VAR:LEVEL:rest` shape is a hard error rather than a
/// silently-skipped line, since silently dropping a malformed line could
/// silently shift every later "next offset" lookup.
pub fn parse_idx(url: &str, text: &str) -> Result<Vec<IdxEntry>, GefsError> {
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

fn parse_idx_line(url: &str, line_number: usize, line: &str) -> Result<IdxEntry, GefsError> {
    let malformed = || GefsError::MalformedIdxLine {
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

/// Find the (first) entry matching `variable`/`level` exactly, e.g.
/// `("TMP", "2 m above ground")`.
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
/// `entries[position]` (its index into `entries`, *not* its 1-indexed
/// `message_number`). `content_length` is the fetched object's total size,
/// required only when this is the last message in the file (see the module
/// docs).
pub fn byte_range(
    url: &str,
    entries: &[IdxEntry],
    position: usize,
    content_length: Option<u64>,
) -> Result<(u64, u64), GefsError> {
    let start = entries[position].offset;
    let end = match entries.get(position + 1) {
        Some(next) => next.offset,
        None => content_length.ok_or_else(|| GefsError::MissingContentLength {
            url: url.to_string(),
        })?,
    };
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_GEC00_IDX: &str = "1:0:d=2026091212:VIS:surface:anl:ENS=low-res ctl\n\
2:329494:d=2026091212:GUST:surface:anl:ENS=low-res ctl\n\
3:855715:d=2026091212:MSLET:mean sea level:anl:ENS=low-res ctl\n\
4:1725195:d=2026091212:PRES:surface:anl:ENS=low-res ctl\n\
5:2467987:d=2026091212:HGT:surface:anl:ENS=low-res ctl\n\
6:2920562:d=2026091212:TSOIL:0-0.1 m below ground:anl:ENS=low-res ctl\n\
7:3286288:d=2026091212:SOILW:0-0.1 m below ground:anl:ENS=low-res ctl\n\
8:3637374:d=2026091212:WEASD:surface:anl:ENS=low-res ctl\n\
9:3874712:d=2026091212:SNOD:surface:anl:ENS=low-res ctl\n\
10:4095887:d=2026091212:ICETK:surface:anl:ENS=low-res ctl\n\
11:4169841:d=2026091212:TMP:2 m above ground:anl:ENS=low-res ctl\n\
12:4609803:d=2026091212:DPT:2 m above ground:anl:ENS=low-res ctl\n";

    #[test]
    fn parses_a_real_captured_idx_file() {
        let entries = parse_idx("https://example.com/x.idx", REAL_GEC00_IDX).unwrap();
        assert_eq!(entries.len(), 12);
        assert_eq!(entries[10].message_number, 11);
        assert_eq!(entries[10].offset, 4_169_841);
        assert_eq!(entries[10].variable, "TMP");
        assert_eq!(entries[10].level, "2 m above ground");
        assert_eq!(entries[10].rest, "anl:ENS=low-res ctl");
    }

    #[test]
    fn find_message_locates_tmp_2m() {
        let entries = parse_idx("u", REAL_GEC00_IDX).unwrap();
        let found = find_message(&entries, "TMP", "2 m above ground").unwrap();
        assert_eq!(found.message_number, 11);
    }

    #[test]
    fn find_message_returns_none_for_absent_field() {
        let entries = parse_idx("u", REAL_GEC00_IDX).unwrap();
        assert!(find_message(&entries, "TMP", "surface").is_none());
        assert!(find_message(&entries, "NOSUCHVAR", "2 m above ground").is_none());
    }

    #[test]
    fn byte_range_uses_next_entrys_offset_when_not_last() {
        let entries = parse_idx("u", REAL_GEC00_IDX).unwrap();
        let (start, end) = byte_range("u", &entries, 10, None).unwrap();
        assert_eq!(start, 4_169_841);
        assert_eq!(end, 4_609_803);
        // This is exactly the real, empirically-verified message length
        // (439962 bytes) for this real captured offset pair.
        assert_eq!(end - start, 439_962);
    }

    #[test]
    fn byte_range_for_the_last_message_needs_content_length() {
        let entries = parse_idx("u", REAL_GEC00_IDX).unwrap();
        let last = entries.len() - 1;
        let err = byte_range("u", &entries, last, None).unwrap_err();
        assert!(matches!(err, GefsError::MissingContentLength { .. }));

        let (start, end) = byte_range("u", &entries, last, Some(5_000_000)).unwrap();
        assert_eq!(start, 4_609_803);
        assert_eq!(end, 5_000_000);
    }

    #[test]
    fn malformed_line_is_a_structured_error_not_a_panic() {
        let err = parse_idx("https://example.com/x.idx", "not a valid idx line").unwrap_err();
        match err {
            GefsError::MalformedIdxLine { line_number, .. } => assert_eq!(line_number, 1),
            other => panic!("expected MalformedIdxLine, got {other:?}"),
        }
    }

    #[test]
    fn blank_lines_are_skipped() {
        let text = "1:0:d=2026091212:VIS:surface:anl:ENS=low-res ctl\n\n\n";
        let entries = parse_idx("u", text).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn ens_mean_annotation_has_no_ens_equals_prefix() {
        // Confirmed live: geavg's .idx lines end in "ens mean", not
        // "ENS=ens mean" or similar -- this crate must not assume the
        // "ENS=" prefix is always present.
        let text = "11:4461193:d=2026091212:TMP:2 m above ground:anl:ens mean\n";
        let entries = parse_idx("u", text).unwrap();
        assert_eq!(entries[0].rest, "anl:ens mean");
    }
}
