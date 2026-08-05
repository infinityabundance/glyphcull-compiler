//! The deterministic package writer (SPEC.md §1, §3).
//!
//! The writer assembles sections in canonical order, compresses per the requested
//! mode, computes per-section CRC-32 over decoded payloads, and (optionally) appends
//! a SEAL section. Output is a pure function of the added sections: identical input
//! produces identical bytes.

use std::collections::BTreeMap;

use crate::codec::seal::compute_seal;
use crate::compress::zlib_compress;
use crate::crc32::crc32;
use crate::error::{Error, Result};
use crate::header::{Header, HEADER_LEN, VERSION};
use crate::section::SectionKind;
use crate::table::{Compression, SectionEntry, MAX_SECTION_DECODED_LEN, SECTION_ENTRY_LEN};

/// The maximum total package size (SPEC.md §1.3).
pub const MAX_PACKAGE_LEN: u64 = 4 << 30;

/// Builds a package section by section.
///
/// ```
/// use glyphcull_format::section::SectionKind;
/// use glyphcull_format::table::Compression;
/// use glyphcull_format::writer::PackageBuilder;
///
/// let mut builder = PackageBuilder::new().with_seal(true);
/// builder
///     .add(SectionKind::Content, b"hello".to_vec(), Compression::Zlib)
///     .expect("add once");
/// let bytes = builder.build().expect("build");
/// assert!(bytes.len() > 16);
/// ```
#[derive(Debug, Default)]
pub struct PackageBuilder {
    sections: BTreeMap<SectionKind, (Compression, Vec<u8>)>,
    with_seal: bool,
}

impl PackageBuilder {
    /// Create an empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether to append a SEAL section covering all other sections.
    #[must_use]
    pub fn with_seal(mut self, on: bool) -> Self {
        self.with_seal = on;
        self
    }

    /// Add a section. Each kind may be added at most once.
    pub fn add(
        &mut self,
        kind: SectionKind,
        payload: Vec<u8>,
        compression: Compression,
    ) -> Result<()> {
        if self.sections.contains_key(&kind) {
            return Err(Error::DuplicateSection {
                kind: kind.to_u32(),
            });
        }
        if payload.len() as u64 > MAX_SECTION_DECODED_LEN {
            return Err(Error::DecodedLenExceeded {
                decoded_len: payload.len() as u64,
                limit: MAX_SECTION_DECODED_LEN,
            });
        }
        if payload.is_empty() {
            return Err(Error::LimitExceeded {
                what: "section payload",
                value: 0,
                limit: 1,
            });
        }
        self.sections.insert(kind, (compression, payload));
        Ok(())
    }

    /// Assemble the package bytes.
    pub fn build(&self) -> Result<Vec<u8>> {
        if self.sections.is_empty() {
            return Err(Error::MissingSection {
                what: "any section",
            });
        }

        // Determine the final section set (canonical order via BTreeMap: numeric
        // kind order equals canonical order).
        let mut kinds: Vec<SectionKind> = self.sections.keys().copied().collect();
        let mut payloads: Vec<(SectionKind, (Compression, Vec<u8>))> = Vec::new();
        for kind in &kinds {
            let (compression, payload) = self
                .sections
                .get(kind)
                .ok_or(Error::MissingSection { what: kind.name() })?;
            payloads.push((*kind, (*compression, payload.clone())));
        }

        // SEAL covers all non-SEAL sections; its payload is computed first.
        if self.with_seal && kinds.contains(&SectionKind::Seal) {
            return Err(Error::DuplicateSection {
                kind: SectionKind::Seal.to_u32(),
            });
        }
        if self.with_seal {
            // The overall hash covers header bytes 0..12, which include the final
            // section count (including SEAL itself), so the header is constructed
            // before the SEAL payload is computed. `header_crc32` lies outside the
            // covered range and is recomputed by `Header::encode`.
            let final_count = payloads.len() as u32 + 1;
            let hash_header = Header {
                version: VERSION,
                flags: 0,
                section_count: final_count,
                header_crc32: 0,
            };
            let covered: Vec<(SectionKind, &[u8])> = payloads
                .iter()
                .map(|(kind, (_, payload))| (*kind, payload.as_slice()))
                .collect();
            let seal = compute_seal(&hash_header, &covered);
            payloads.push((SectionKind::Seal, (Compression::None, seal.encode())));
            kinds.push(SectionKind::Seal);
        }

        // Canonical order: kinds must be strictly increasing (BTreeMap guarantees
        // ascending order; SEAL appended last preserves it).
        let monotonic = kinds.iter().zip(kinds.iter().skip(1)).all(|(a, b)| a < b);
        debug_assert!(monotonic);

        // Compute stored forms.
        let mut stored: Vec<(SectionKind, Compression, Vec<u8>, u64, u32)> =
            Vec::with_capacity(payloads.len());
        for (kind, (compression, payload)) in &payloads {
            let stored_bytes = match compression {
                Compression::None => payload.clone(),
                Compression::Zlib => zlib_compress(payload)?,
            };
            let decoded_len = payload.len() as u64;
            let crc = crc32(payload);
            stored.push((*kind, *compression, stored_bytes, decoded_len, crc));
        }

        // Layout: header + table + payloads.
        let table_len = (stored.len() as u64)
            .checked_mul(SECTION_ENTRY_LEN as u64)
            .ok_or(Error::OverflowGuard)?;
        let mut offset = (HEADER_LEN as u64)
            .checked_add(table_len)
            .ok_or(Error::OverflowGuard)?;
        let mut entries = Vec::with_capacity(stored.len());
        for (kind, compression, stored_bytes, decoded_len, crc) in &stored {
            let entry = SectionEntry {
                kind: kind.to_u32(),
                compression: *compression,
                flags: 0,
                offset,
                stored_len: stored_bytes.len() as u64,
                decoded_len: *decoded_len,
                crc32: *crc,
            };
            offset = offset
                .checked_add(stored_bytes.len() as u64)
                .ok_or(Error::OverflowGuard)?;
            entries.push(entry);
        }
        if offset > MAX_PACKAGE_LEN {
            return Err(Error::LimitExceeded {
                what: "package length",
                value: offset,
                limit: MAX_PACKAGE_LEN,
            });
        }

        // Assemble.
        let header = Header {
            version: VERSION,
            flags: 0,
            section_count: stored.len() as u32,
            header_crc32: 0,
        };
        let mut out = Vec::with_capacity(offset as usize);
        out.extend_from_slice(&header.encode());
        for entry in &entries {
            out.extend_from_slice(&entry.encode());
        }
        for (_, _, stored_bytes, _, _) in &stored {
            out.extend_from_slice(stored_bytes);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::PackageBuilder;
    use crate::section::SectionKind;
    use crate::table::Compression;

    #[test]
    fn duplicate_section_rejected() {
        let mut builder = PackageBuilder::new();
        builder
            .add(SectionKind::Info, b"x".to_vec(), Compression::None)
            .expect("add");
        assert!(builder
            .add(SectionKind::Info, b"y".to_vec(), Compression::None)
            .is_err());
    }

    #[test]
    fn empty_payload_rejected() {
        let mut builder = PackageBuilder::new();
        assert!(builder
            .add(SectionKind::Info, Vec::new(), Compression::None)
            .is_err());
    }

    #[test]
    fn empty_builder_rejected() {
        assert!(PackageBuilder::new().build().is_err());
    }

    #[test]
    fn deterministic_build() {
        let build = || {
            let mut builder = PackageBuilder::new().with_seal(true);
            builder
                .add(SectionKind::Info, b"info".to_vec(), Compression::Zlib)
                .expect("add");
            builder
                .add(
                    SectionKind::Content,
                    b"hello world hello world".to_vec(),
                    Compression::Zlib,
                )
                .expect("add");
            builder.build().expect("build")
        };
        assert_eq!(build(), build());
    }
}
