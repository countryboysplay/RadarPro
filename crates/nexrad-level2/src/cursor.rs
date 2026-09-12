//! Bounds-checked, big-endian byte cursor.
//!
//! Every NEXRAD Level II byte range in this crate — the file itself, a
//! decompressed LDM record, or a single message/data-block payload — is
//! untrusted, network-sourced data. This cursor is the single place that
//! turns "read N bytes at position P" into a checked operation that
//! returns a structured [`DecodeError`] instead of indexing/slicing in a
//! way that could panic.

use crate::DecodeError;

/// A bounds-checked cursor over a byte slice, tracking an `offset` used
/// only to make error messages report a position meaningful to the
/// caller (e.g. an absolute file offset, or an offset relative to the
/// start of a decompressed record or message).
pub(crate) struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
    /// Offset of `buf[0]` within whatever larger context the caller wants
    /// reflected in error messages (purely diagnostic; does not affect
    /// bounds checking, which is always relative to `buf`).
    origin: usize,
}

impl<'a> Cursor<'a> {
    /// Construct a cursor over `buf`, reporting positions relative to
    /// `origin` in error messages.
    pub(crate) fn new(buf: &'a [u8], origin: usize) -> Self {
        Self {
            buf,
            pos: 0,
            origin,
        }
    }

    /// Current read position, relative to the start of `buf`.
    pub(crate) fn position(&self) -> usize {
        self.pos
    }

    /// Current read position, translated into the caller's coordinate
    /// space (for error messages).
    pub(crate) fn absolute_position(&self) -> usize {
        self.origin.saturating_add(self.pos)
    }

    /// Number of unread bytes remaining.
    pub(crate) fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Move the read position to an absolute offset within `buf`.
    ///
    /// Returns [`DecodeError::UnexpectedEnd`] if `pos` is past the end of
    /// the buffer.
    pub(crate) fn seek_to(&mut self, pos: usize) -> Result<(), DecodeError> {
        if pos > self.buf.len() {
            return Err(DecodeError::UnexpectedEnd {
                offset: self.origin.saturating_add(pos),
                need: 0,
                available: 0,
            });
        }
        self.pos = pos;
        Ok(())
    }

    /// Borrow the next `n` bytes without consuming them, bounds-checked
    /// against the whole buffer starting at `pos`.
    pub(crate) fn peek(&self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Internal(
            "cursor position overflow".to_string(),
        ))?;
        if end > self.buf.len() {
            return Err(DecodeError::UnexpectedEnd {
                offset: self.absolute_position(),
                need: n,
                available: self.buf.len() - self.pos,
            });
        }
        Ok(&self.buf[self.pos..end])
    }

    /// Consume and return the next `n` bytes.
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let bytes = self.peek(n)?;
        self.pos += n;
        Ok(bytes)
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn read_i16(&mut self) -> Result<i16, DecodeError> {
        let b = self.take(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn read_f32(&mut self) -> Result<f32, DecodeError> {
        let b = self.take(4)?;
        Ok(f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Read `n` bytes and require they are ASCII, returning them as a
    /// `String`. NEXRAD text fields (site IDs, moment names, magic
    /// strings) are always fixed-width ASCII.
    pub(crate) fn read_ascii(&mut self, n: usize) -> Result<String, DecodeError> {
        let offset = self.absolute_position();
        let bytes = self.take(n)?;
        if !bytes.is_ascii() {
            return Err(DecodeError::NonAsciiField {
                offset,
                bytes: bytes.to_vec(),
            });
        }
        // `is_ascii()` guarantees this UTF-8 conversion cannot fail.
        Ok(String::from_utf8(bytes.to_vec()).expect("ASCII bytes are always valid UTF-8"))
    }
}
