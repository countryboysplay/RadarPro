//! `nexrad-level2` — NEXRAD Archive II / Level II decoding.
//!
//! # Status: S00 placeholder
//!
//! This crate is a deliberately minimal, compilable skeleton created during
//! the S00 "foundation" stage. It does **not** yet parse Archive II files or
//! Message 31 (digital radar data) records.
//!
//! Real decoding — including handling of missing values, range folding,
//! compression, and message framing, backed by real fixtures under
//! `fixtures/` — arrives in stage S01. No NEXRAD parsing behavior should be
//! assumed or inferred from this crate yet.

/// Placeholder marker type for this crate's future decoder entry point.
///
/// This exists only so the crate has a public item and something for
/// `apps/radar-cli` (or tests) to reference. It carries no decoding
/// behavior. In S01 this will be replaced by a real Archive II reader.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Level2Decoder;

impl Level2Decoder {
    /// Construct a placeholder decoder.
    ///
    /// This does not open, read, or validate any file. Real file handling
    /// (`inspect <file>` in `radar-cli`, and Archive II parsing here)
    /// arrives in S01.
    pub fn new() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_decoder_is_constructible() {
        let decoder = Level2Decoder::new();
        assert_eq!(decoder, Level2Decoder);
    }
}
