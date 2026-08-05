//! Bounds-checked little-endian cursor and writer primitives.
//!
//! Every read checks remaining length and can only fail with a typed error; indexing
//! is never used on untrusted lengths. All integers are little-endian per SPEC.md §1.
//!
//! The cursor is the single choke point for untrusted bytes: all accesses go through
//! [`Cursor::take`], which verifies bounds before returning a slice. The direct
//! indexing inside this module is therefore provably safe (the exclusive purpose of
//! the module) and is the documented exception to the workspace's indexing policy.
#![allow(clippy::indexing_slicing)]

use crate::error::{Error, Result};

/// A bounds-checked cursor over a byte slice.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Create a cursor over `data`.
    pub(crate) const fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// The number of bytes not yet consumed.
    pub(crate) const fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Peek the next byte without consuming it.
    pub(crate) fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    /// Read exactly `n` bytes, advancing the cursor.
    pub(crate) fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(Error::UnexpectedEof { what })?;
        if end > self.data.len() {
            return Err(Error::UnexpectedEof { what });
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Read a `u8`.
    pub(crate) fn u8(&mut self, what: &'static str) -> Result<u8> {
        Ok(self.take(1, what)?[0])
    }

    /// Read a little-endian `u16`.
    pub(crate) fn u16(&mut self, what: &'static str) -> Result<u16> {
        let b = self.take(2, what)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    /// Read a little-endian `u32`.
    pub(crate) fn u32(&mut self, what: &'static str) -> Result<u32> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Read a little-endian `u64`.
    pub(crate) fn u64(&mut self, what: &'static str) -> Result<u64> {
        let b = self.take(8, what)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    /// Read a little-endian `f32` (bit-exact via `to_bits`/`from_bits`).
    pub(crate) fn f32(&mut self, what: &'static str) -> Result<f32> {
        Ok(f32::from_bits(self.u32(what)?))
    }

    /// Read `n` bytes and require them to be valid UTF-8.
    pub(crate) fn utf8(&mut self, n: usize, what: &'static str) -> Result<&'a str> {
        let bytes = self.take(n, what)?;
        core::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)
    }

    /// Require that all bytes have been consumed.
    pub(crate) fn finish(self, what: &'static str) -> Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(Error::TrailingBytes { what })
        }
    }
}

/// A growable little-endian byte writer.
#[derive(Debug, Default, Clone)]
pub(crate) struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    /// Create an empty writer.
    pub(crate) const fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Append a `u8`.
    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    /// Append a little-endian `u16`.
    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Append a little-endian `u32`.
    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Append a little-endian `u64`.
    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Append a little-endian `f32` (bit-exact via `to_bits`).
    pub(crate) fn f32(&mut self, value: f32) {
        self.bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }

    /// Append raw bytes.
    pub(crate) fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    /// Return the written bytes.
    #[must_use]
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::{Cursor, Writer};

    #[test]
    fn round_trip_all_widths() {
        let mut w = Writer::new();
        w.u8(0xAB);
        w.u16(0xCDEF);
        w.u32(0x0123_4567);
        w.u64(0x89AB_CDEF_0123_4567);
        w.f32(3.5);
        w.bytes(b"tail");
        let bytes = w.into_bytes();

        let mut c = Cursor::new(&bytes);
        assert_eq!(c.u8("u8").expect("u8"), 0xAB);
        assert_eq!(c.u16("u16").expect("u16"), 0xCDEF);
        assert_eq!(c.u32("u32").expect("u32"), 0x0123_4567);
        assert_eq!(c.u64("u64").expect("u64"), 0x89AB_CDEF_0123_4567);
        assert_eq!(c.f32("f32").expect("f32"), 3.5);
        assert_eq!(c.take(4, "tail").expect("tail"), b"tail");
        assert_eq!(c.remaining(), 0);
        c.finish("all").expect("no trailing");
    }

    #[test]
    fn reads_fail_at_end() {
        let mut c = Cursor::new(b"\x01");
        assert_eq!(c.u8("x").expect("x"), 1);
        assert!(c.u16("y").is_err());
        assert!(c.take(1, "z").is_err());
    }

    #[test]
    fn utf8_validation() {
        let mut c = Cursor::new("héllo".as_bytes());
        assert_eq!(c.utf8(6, "text").expect("utf8"), "héllo");
        let mut c2 = Cursor::new("héllo".as_bytes());
        assert_eq!(c2.utf8(4, "prefix").expect("utf8"), "hél");
        let mut bad = Cursor::new(b"\xFF\xFE");
        assert!(bad.utf8(2, "text").is_err());
    }
}
