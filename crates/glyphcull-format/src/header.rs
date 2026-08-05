//! The 16-byte package header (SPEC.md §1.1).

use crate::crc32::crc32;
use crate::error::{Error, Result};
use crate::util::{Cursor, Writer};

/// The ASCII magic bytes: `CULL`.
pub const MAGIC: [u8; 4] = *b"CULL";

/// The current format version (SPEC.md §1).
pub const VERSION: u16 = 1;

/// The header is 16 bytes: magic(4) + version(2) + flags(2) + section_count(4) + crc(4).
pub const HEADER_LEN: usize = 16;

/// The declared upper bound on sections in a v1 package (SPEC.md §1.3).
pub const MAX_SECTION_COUNT: u32 = 64;

/// The header CRC-32 covers bytes `0..12` (everything except the CRC field itself).
const CRC_COVERED_LEN: usize = 12;

/// The package header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Format version; must equal [`VERSION`] for v1 readers.
    pub version: u16,
    /// Reserved flags; v1 writes zero.
    pub flags: u16,
    /// Number of section table entries.
    pub section_count: u32,
    /// CRC-32 over bytes `0..12` of the encoded header.
    pub header_crc32: u32,
}

impl Header {
    /// Encode the header as exactly [`HEADER_LEN`] bytes.
    ///
    /// The fixed write sequence produces exactly 16 bytes; the direct byte accesses
    /// below are provably in bounds on local, fixed-size buffers.
    #[allow(clippy::indexing_slicing)]
    #[must_use]
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut w = Writer::new();
        w.bytes(&MAGIC);
        w.u16(self.version);
        w.u16(self.flags);
        w.u32(self.section_count);
        w.u32(0); // CRC placeholder
        let mut bytes = w.into_bytes();
        // The write sequence above produces exactly HEADER_LEN bytes; the pinned
        // `layout_is_pinned` test verifies the size. The conversion cannot fail.
        #[allow(clippy::expect_used)]
        {
            let crc = crc32(&bytes[..CRC_COVERED_LEN]);
            bytes[12..].copy_from_slice(&crc.to_le_bytes());
            bytes.try_into().expect("header is exactly 16 bytes")
        }
    }

    /// Decode and validate a header from exactly [`HEADER_LEN`] bytes.
    pub fn decode(bytes: &[u8; HEADER_LEN]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let magic = c.take(4, "magic")?;
        if magic != MAGIC {
            return Err(Error::BadMagic);
        }
        let version = c.u16("version")?;
        if version != VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let flags = c.u16("flags")?;
        let section_count = c.u32("section_count")?;
        let header_crc32 = c.u32("header_crc32")?;
        c.finish("header")?;
        let actual = crc32(&bytes[..CRC_COVERED_LEN]);
        if actual != header_crc32 {
            return Err(Error::HeaderCrcMismatch {
                expected: header_crc32,
                actual,
            });
        }
        if section_count > MAX_SECTION_COUNT {
            return Err(Error::TooManySections {
                count: section_count,
                limit: MAX_SECTION_COUNT,
            });
        }
        if section_count == 0 {
            return Err(Error::LimitExceeded {
                what: "section_count",
                value: 0,
                limit: 1,
            });
        }
        Ok(Self {
            version,
            flags,
            section_count,
            header_crc32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{crc32, Header, VERSION};
    use crate::error::Error;

    fn sample() -> Header {
        Header {
            version: VERSION,
            flags: 0,
            section_count: 1,
            header_crc32: 0,
        }
    }

    #[test]
    fn round_trip() {
        let sample = sample();
        let encoded = sample.encode();
        let decoded = Header::decode(&encoded).expect("decode");
        assert_eq!(decoded.version, sample.version);
        assert_eq!(decoded.flags, sample.flags);
        assert_eq!(decoded.section_count, sample.section_count);
        // `header_crc32` is computed at encode time, not carried from the struct.
        assert_eq!(decoded.header_crc32, crc32(&encoded[..12]));
    }

    #[test]
    fn layout_is_pinned() {
        let encoded = sample().encode();
        assert_eq!(&encoded[0..4], b"CULL");
        assert_eq!(u16::from_le_bytes([encoded[4], encoded[5]]), VERSION);
        assert_eq!(u16::from_le_bytes([encoded[6], encoded[7]]), 0);
        assert_eq!(
            u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]),
            1
        );
        assert_eq!(
            u32::from_le_bytes([encoded[12], encoded[13], encoded[14], encoded[15]]),
            crc32(&encoded[..12])
        );
    }

    #[test]
    fn bad_magic_rejected() {
        let mut bytes = sample().encode();
        bytes[0] = b'X';
        assert_eq!(Header::decode(&bytes), Err(Error::BadMagic));
    }

    #[test]
    fn bad_version_rejected() {
        let mut bytes = sample().encode();
        bytes[4] = 2;
        bytes[5] = 0;
        assert_eq!(Header::decode(&bytes), Err(Error::UnsupportedVersion(2)));
    }

    #[test]
    fn bad_crc_rejected() {
        let mut bytes = sample().encode();
        bytes[8] = 99; // corrupt section_count after CRC was computed
        assert!(matches!(
            Header::decode(&bytes),
            Err(Error::HeaderCrcMismatch { .. })
        ));
    }

    #[test]
    fn too_many_sections_rejected() {
        let mut header = sample();
        header.section_count = 65;
        assert!(matches!(
            Header::decode(&header.encode()),
            Err(Error::TooManySections {
                count: 65,
                limit: 64
            })
        ));
    }

    #[test]
    fn zero_sections_rejected() {
        let mut header = sample();
        header.section_count = 0;
        assert!(matches!(
            Header::decode(&header.encode()),
            Err(Error::LimitExceeded { .. })
        ));
    }
}
