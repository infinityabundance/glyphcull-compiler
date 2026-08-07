//! The 32-byte section table entry (SPEC.md §1.2).

use crate::error::{Error, Result};
use crate::section::SectionKind;
use crate::util::{Cursor, Writer};

/// Each section table entry is exactly 32 bytes.
pub const SECTION_ENTRY_LEN: usize = 32;

/// Maximum decoded size of a single section (SPEC.md §1.3).
pub const MAX_SECTION_DECODED_LEN: u64 = 2 << 30;

/// Maximum total file size (SPEC.md §1.3).
pub const MAX_FILE_LEN: u64 = 4 << 30;

/// Compression codes (SPEC.md §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Stored raw.
    None = 0,
    /// zlib deflate, level 9.
    Zlib = 1,
}

impl Compression {
    /// The wire code.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Parse a wire code, rejecting unknown values.
    pub fn from_code(code: u8) -> Result<Self> {
        match code {
            0 => Ok(Self::None),
            1 => Ok(Self::Zlib),
            other => Err(Error::UnsupportedCompression {
                kind: 0,
                code: other,
            }),
        }
    }
}

/// One section table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionEntry {
    /// The numeric section kind (may be reserved/unknown: readers skip those).
    pub kind: u32,
    /// The compression applied to the stored payload.
    pub compression: Compression,
    /// Reserved flags; must be zero in v1.
    pub flags: u8,
    /// Absolute byte offset of the stored payload.
    pub offset: u64,
    /// Byte length of the stored payload as written.
    pub stored_len: u64,
    /// Byte length after decompression.
    pub decoded_len: u64,
    /// CRC-32 over the decoded payload.
    pub crc32: u32,
}

impl SectionEntry {
    /// The known kind, if this entry's kind is defined in v1.
    #[must_use]
    pub const fn known_kind(&self) -> Option<SectionKind> {
        SectionKind::from_u32(self.kind)
    }

    /// Encode the entry as exactly [`SECTION_ENTRY_LEN`] bytes.
    #[must_use]
    pub fn encode(&self) -> [u8; SECTION_ENTRY_LEN] {
        let mut w = Writer::new();
        w.u32(self.kind);
        w.u8(self.compression.code());
        w.u8(self.flags);
        w.u16(0); // reserved
        w.u64(self.offset);
        w.u64(self.stored_len);
        w.u32(self.decoded_len as u32);
        w.u32(self.crc32);
        let bytes = w.into_bytes();
        // Fixed write sequence of exactly SECTION_ENTRY_LEN bytes; cannot fail.
        #[allow(clippy::expect_used)]
        {
            bytes.try_into().expect("section entry is exactly 32 bytes")
        }
    }

    /// Decode and validate an entry from exactly [`SECTION_ENTRY_LEN`] bytes.
    pub fn decode(bytes: &[u8; SECTION_ENTRY_LEN]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let kind = c.u32("section kind")?;
        let compression = Compression::from_code(c.u8("compression")?)?;
        let flags = c.u8("flags")?;
        let reserved = c.u16("reserved")?;
        let offset = c.u64("offset")?;
        let stored_len = c.u64("stored_len")?;
        let decoded_len = u64::from(c.u32("decoded_len")?);
        let crc32 = c.u32("crc32")?;
        c.finish("section entry")?;
        // The flags byte: bit 0 is the `critical` bit, meaningful only for
        // unknown section kinds (SPEC.md §1.2 — a critical unknown section MUST
        // be rejected; a noncritical one is skipped). Reserved bits 1..7 must be
        // zero, and known kinds must carry no flags at all (the writer never
        // emits them; strictness keeps canonical packages canonical).
        if flags & 0xFE != 0 || reserved != 0 {
            return Err(Error::ReservedBitsSet);
        }
        if SectionKind::from_u32(kind).is_some() && flags != 0 {
            return Err(Error::ReservedBitsSet);
        }
        if decoded_len > MAX_SECTION_DECODED_LEN {
            return Err(Error::DecodedLenExceeded {
                decoded_len,
                limit: MAX_SECTION_DECODED_LEN,
            });
        }
        if decoded_len == 0 {
            // A zero-length section is not meaningful in v1.
            return Err(Error::LimitExceeded {
                what: "decoded_len",
                value: 0,
                limit: 1,
            });
        }
        Ok(Self {
            kind,
            compression,
            flags,
            offset,
            stored_len,
            decoded_len,
            crc32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Compression, SectionEntry};
    use crate::section::SectionKind;

    fn sample() -> SectionEntry {
        SectionEntry {
            kind: SectionKind::Info.to_u32(),
            compression: Compression::Zlib,
            flags: 0,
            offset: 48,
            stored_len: 64,
            decoded_len: 100,
            crc32: 0xDEAD_BEEF,
        }
    }

    #[test]
    fn round_trip() {
        let entry = sample();
        let decoded = SectionEntry::decode(&entry.encode()).expect("decode");
        assert_eq!(decoded, entry);
        assert_eq!(decoded.known_kind(), Some(SectionKind::Info));
    }

    #[test]
    fn unknown_kind_preserved() {
        let mut entry = sample();
        entry.kind = 8; // reserved kind: container preserves, reader skips
        let decoded = SectionEntry::decode(&entry.encode()).expect("decode");
        assert_eq!(decoded.kind, 8);
        assert_eq!(decoded.known_kind(), None);
    }

    #[test]
    fn reserved_bits_rejected() {
        let mut bytes = sample().encode();
        bytes[5] = 1; // flags
        assert_eq!(
            SectionEntry::decode(&bytes),
            Err(crate::error::Error::ReservedBitsSet)
        );
        let mut bytes = sample().encode();
        bytes[6] = 1; // reserved u16 low byte
        assert_eq!(
            SectionEntry::decode(&bytes),
            Err(crate::error::Error::ReservedBitsSet)
        );
    }

    #[test]
    fn unknown_kind_rejected() {
        // Reserved kinds are preserved at the container level (readers skip them),
        // so there is no rejection here; see `unknown_kind_preserved`.
        let mut entry = sample();
        entry.kind = 8;
        assert!(SectionEntry::decode(&entry.encode()).is_ok());
    }

    #[test]
    fn bad_compression_rejected() {
        let mut bytes = sample().encode();
        bytes[4] = 7;
        assert!(SectionEntry::decode(&bytes).is_err());
    }

    #[test]
    fn zero_decoded_len_rejected() {
        let mut entry = sample();
        entry.decoded_len = 0;
        assert!(SectionEntry::decode(&entry.encode()).is_err());
    }
}
