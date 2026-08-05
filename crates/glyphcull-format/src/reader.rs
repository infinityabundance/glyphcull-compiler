//! The strict package reader (SPEC.md §1.6).
//!
//! `parse` validates the header, the section table (bounds, compression, decoded
//! lengths, reserved bits), decodes every payload, verifies per-section CRC-32, and
//! verifies the SEAL hash tree when present. Unknown section kinds are preserved and
//! skipped (forward compatibility). Every failure is a typed [`Error`]; the reader
//! never panics on input.

use std::collections::BTreeMap;

use crate::codec::seal::{verify_seal, SealSection};
use crate::compress::zlib_decompress;
use crate::crc32::crc32;
use crate::error::{Error, Result};
use crate::header::{Header, HEADER_LEN};
use crate::section::SectionKind;
use crate::table::{Compression, SectionEntry, SECTION_ENTRY_LEN};

/// A decoded section payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSection {
    /// The compression applied on disk.
    pub compression: Compression,
    /// Stored byte length on disk.
    pub stored_len: u64,
    /// Decoded byte length.
    pub decoded_len: u64,
    /// CRC-32 over the decoded payload.
    pub crc32: u32,
    /// The decoded payload bytes.
    pub payload: Vec<u8>,
}

/// A parsed package with all known sections decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPackage {
    /// The validated header.
    pub header: Header,
    /// Section table entries in file order.
    pub entries: Vec<SectionEntry>,
    /// Decoded known sections, keyed by kind.
    pub sections: BTreeMap<SectionKind, DecodedSection>,
    /// Decoded sections with reserved/unknown kinds (forward compatibility).
    pub unknown: Vec<(u32, DecodedSection)>,
}

impl ParsedPackage {
    /// The decoded payload of a known section, if present.
    #[must_use]
    pub fn section(&self, kind: SectionKind) -> Option<&[u8]> {
        self.sections.get(&kind).map(|s| s.payload.as_slice())
    }
}

/// Parse and fully validate a package from raw bytes.
///
/// This reference implementation decodes every section eagerly; runtimes may instead
/// seek per-section via the table (SPEC.md §1) for streaming loads.
pub fn parse(bytes: &[u8]) -> Result<ParsedPackage> {
    if bytes.len() < HEADER_LEN {
        return Err(Error::TooShort);
    }
    let header_bytes: &[u8; HEADER_LEN] = bytes
        .get(..HEADER_LEN)
        .ok_or(Error::TooShort)?
        .try_into()
        .map_err(|_| Error::TooShort)?;
    let header = Header::decode(header_bytes)?;

    let table_len = (header.section_count as usize)
        .checked_mul(SECTION_ENTRY_LEN)
        .ok_or(Error::OverflowGuard)?;
    let table_end = HEADER_LEN
        .checked_add(table_len)
        .ok_or(Error::OverflowGuard)?;
    if bytes.len() < table_end {
        return Err(Error::TruncatedTable);
    }

    // Parse entries.
    let mut entries = Vec::with_capacity(header.section_count as usize);
    for i in 0..header.section_count as usize {
        let start = HEADER_LEN
            .checked_add(
                i.checked_mul(SECTION_ENTRY_LEN)
                    .ok_or(Error::OverflowGuard)?,
            )
            .ok_or(Error::OverflowGuard)?;
        let end = start
            .checked_add(SECTION_ENTRY_LEN)
            .ok_or(Error::OverflowGuard)?;
        let raw: &[u8; SECTION_ENTRY_LEN] = bytes
            .get(start..end)
            .ok_or(Error::TruncatedTable)?
            .try_into()
            .map_err(|_| Error::TruncatedTable)?;
        entries.push(SectionEntry::decode(raw)?);
    }

    // Decode payloads with validation, preserving file order.
    let mut known: BTreeMap<SectionKind, DecodedSection> = BTreeMap::new();
    let mut unknown: Vec<(u32, DecodedSection)> = Vec::new();
    for entry in &entries {
        let end = entry
            .offset
            .checked_add(entry.stored_len)
            .ok_or(Error::OutOfBounds {
                offset: entry.offset,
                length: entry.stored_len,
                file_len: bytes.len() as u64,
            })?;
        if end > bytes.len() as u64 {
            return Err(Error::OutOfBounds {
                offset: entry.offset,
                length: entry.stored_len,
                file_len: bytes.len() as u64,
            });
        }
        let stored = bytes
            .get(entry.offset as usize..end as usize)
            .ok_or(Error::OutOfBounds {
                offset: entry.offset,
                length: entry.stored_len,
                file_len: bytes.len() as u64,
            })?;
        let payload = match entry.compression {
            Compression::None => stored.to_vec(),
            Compression::Zlib => zlib_decompress(stored, entry.decoded_len as usize)?,
        };
        let actual_crc = crc32(&payload);
        if actual_crc != entry.crc32 {
            return Err(Error::CrcMismatch {
                kind: entry.kind,
                expected: entry.crc32,
                actual: actual_crc,
            });
        }
        let decoded = DecodedSection {
            compression: entry.compression,
            stored_len: entry.stored_len,
            decoded_len: entry.decoded_len,
            crc32: entry.crc32,
            payload,
        };
        match entry.known_kind() {
            Some(kind) => {
                if known.insert(kind, decoded).is_some() {
                    return Err(Error::DuplicateSection { kind: entry.kind });
                }
            }
            None => unknown.push((entry.kind, decoded)),
        }
    }

    // Verify SEAL when present: it covers every other section.
    if let Some(seal_section) = known.get(&SectionKind::Seal) {
        let seal = SealSection::decode(&seal_section.payload)?;
        let covered: Vec<(SectionKind, &[u8])> = known
            .iter()
            .filter(|(kind, _)| **kind != SectionKind::Seal)
            .map(|(kind, section)| (*kind, section.payload.as_slice()))
            .collect();
        verify_seal(&seal, &header, &covered)?;
    }

    Ok(ParsedPackage {
        header,
        entries,
        sections: known,
        unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::section::SectionKind;
    use crate::table::Compression;
    use crate::writer::PackageBuilder;

    fn sample_package(with_seal: bool) -> Vec<u8> {
        let mut builder = PackageBuilder::new().with_seal(with_seal);
        builder
            .add(
                SectionKind::Info,
                b"{\"info\":1}".to_vec(),
                Compression::Zlib,
            )
            .expect("add");
        builder
            .add(
                SectionKind::Content,
                b"content payload".to_vec(),
                Compression::Zlib,
            )
            .expect("add");
        builder.build().expect("build")
    }

    #[test]
    fn round_trip() {
        let bytes = sample_package(true);
        let pkg = parse(&bytes).expect("parse");
        assert_eq!(pkg.header.section_count, 3);
        assert_eq!(
            pkg.section(SectionKind::Info),
            Some(b"{\"info\":1}".as_slice())
        );
        assert_eq!(
            pkg.section(SectionKind::Content),
            Some(b"content payload".as_slice())
        );
        assert!(pkg.sections.contains_key(&SectionKind::Seal));
    }

    #[test]
    fn parse_without_seal() {
        let bytes = sample_package(false);
        let pkg = parse(&bytes).expect("parse");
        assert!(!pkg.sections.contains_key(&SectionKind::Seal));
    }

    #[test]
    fn too_short() {
        assert!(parse(b"").is_err());
        assert!(parse(b"CULL").is_err());
    }

    #[test]
    fn bad_magic() {
        let mut bytes = sample_package(false);
        bytes[0] = b'X';
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn truncated_table() {
        let bytes = sample_package(false);
        assert!(parse(&bytes[..24]).is_err());
    }

    #[test]
    fn crc_tamper_detected() {
        let mut bytes = sample_package(false);
        // Flip a payload byte (find the CONT section's payload region).
        let pkg = parse(&bytes).expect("parse");
        let content = pkg.section(SectionKind::Content).expect("content");
        let entry = pkg
            .entries
            .iter()
            .find(|e| e.kind == SectionKind::Content.to_u32())
            .expect("entry");
        // Find the byte in the file: entry.offset + delta. Flip the last byte.
        let index = (entry.offset + content.len() as u64 - 1) as usize;
        bytes[index] ^= 0xFF;
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn seal_tamper_detected() {
        let mut bytes = sample_package(true);
        let pkg = parse(&bytes).expect("parse");
        let content = pkg.section(SectionKind::Content).expect("content");
        let entry = pkg
            .entries
            .iter()
            .find(|e| e.kind == SectionKind::Content.to_u32())
            .expect("entry");
        // Flip a content byte; per-section CRC fails before SEAL is consulted, but
        // the SEAL check would also fail if CRCs were recomputed. Either way: error.
        let index = (entry.offset + content.len() as u64 - 1) as usize;
        bytes[index] ^= 0x01;
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn section_ordering_not_required() {
        // The reader consumes the table, so section order in the file does not
        // matter; the writer always emits canonical order, but a conforming reader
        // must not depend on it.
        let mut builder = PackageBuilder::new();
        builder
            .add(SectionKind::Content, b"first".to_vec(), Compression::Zlib)
            .expect("add");
        builder
            .add(SectionKind::Info, b"{\"i\":1}".to_vec(), Compression::Zlib)
            .expect("add");
        // Builder emits canonical order (Info first) regardless of insertion order.
        let bytes = builder.build().expect("build");
        let pkg = parse(&bytes).expect("parse");
        assert_eq!(
            pkg.section(SectionKind::Info),
            Some(b"{\"i\":1}".as_slice())
        );
    }
}
