//! SEAL section codec (SPEC.md §2.7): content hash tree.
//!
//! The SEAL section covers every other section with a SHA-256 per-section hash plus
//! an overall hash that binds header identity, section kinds/sizes, and content.
//! The definition is deliberately non-circular (the SEAL section does not cover
//! itself), so the hash is computable before assembly.

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::header::Header;
use crate::section::SectionKind;
use crate::util::{Cursor, Writer};

/// The hash tree mode (SPEC.md §2.7).
pub const SEAL_MODE_HASH_TREE: u8 = 1;

/// The hash algorithm (SPEC.md §2.7).
pub const SEAL_ALGO_SHA256: u8 = 0;

/// A per-section hash entry is 36 bytes: kind (4) + digest (32).
pub const SECTION_HASH_LEN: usize = 36;

/// The overall hash length.
pub const OVERALL_HASH_LEN: usize = 32;

/// Maximum covered sections (bounded by the container limit).
pub const MAX_COVERED_SECTIONS: u32 = 64;

/// The decoded SEAL section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealSection {
    /// Per-section hashes: (kind, SHA-256 of the decoded payload).
    pub entries: Vec<(u32, [u8; OVERALL_HASH_LEN])>,
    /// Overall content hash.
    pub overall: [u8; OVERALL_HASH_LEN],
}

impl SealSection {
    /// Encode to the SEAL payload.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(SEAL_MODE_HASH_TREE);
        w.u8(SEAL_ALGO_SHA256);
        w.u8(0); // flags
        w.u8(0); // reserved
        w.u32(self.entries.len() as u32);
        for (kind, digest) in &self.entries {
            w.u32(*kind);
            w.bytes(digest);
        }
        w.bytes(&self.overall);
        w.into_bytes()
    }

    /// Decode and structurally validate the SEAL payload.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let mode = c.u8("SEAL mode")?;
        let algo = c.u8("SEAL algo")?;
        let flags = c.u8("SEAL flags")?;
        let reserved = c.u8("SEAL reserved")?;
        let count = c.u32("SEAL count")?;
        if mode != SEAL_MODE_HASH_TREE {
            return Err(Error::UnknownValue {
                what: "SEAL mode",
                value: u64::from(mode),
            });
        }
        if algo != SEAL_ALGO_SHA256 {
            return Err(Error::UnknownValue {
                what: "SEAL algo",
                value: u64::from(algo),
            });
        }
        if flags != 0 || reserved != 0 {
            return Err(Error::ReservedBitsSet);
        }
        if count > MAX_COVERED_SECTIONS {
            return Err(Error::LimitExceeded {
                what: "SEAL count",
                value: u64::from(count),
                limit: u64::from(MAX_COVERED_SECTIONS),
            });
        }
        let mut entries = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let kind = c.u32("SEAL section kind")?;
            let digest_bytes = c.take(OVERALL_HASH_LEN, "SEAL digest")?;
            let mut digest = [0_u8; OVERALL_HASH_LEN];
            digest.copy_from_slice(digest_bytes);
            entries.push((kind, digest));
        }
        let overall_bytes = c.take(OVERALL_HASH_LEN, "SEAL overall")?;
        let mut overall = [0_u8; OVERALL_HASH_LEN];
        overall.copy_from_slice(overall_bytes);
        c.finish("SEAL payload")?;
        Ok(Self { entries, overall })
    }
}

/// Compute the SEAL section over the covered sections.
///
/// `covered` must be the non-SEAL sections in canonical order, as `(kind, decoded
/// payload)`. The per-section entries hash each payload; the overall hash covers the
/// header identity bytes (`header` encoded bytes 0..12), then for each covered
/// section: kind (u32 LE), decoded length (u32 LE), and the decoded payload.
pub fn compute_seal(header: &Header, covered: &[(SectionKind, &[u8])]) -> SealSection {
    let mut overall = Sha256::new();
    let header_bytes = header.encode();
    overall.update(&header_bytes[..12]);

    let mut entries = Vec::with_capacity(covered.len());
    for (kind, payload) in covered {
        let digest = Sha256::digest(payload);
        let mut entry = [0_u8; OVERALL_HASH_LEN];
        entry.copy_from_slice(&digest);
        entries.push((kind.to_u32(), entry));

        overall.update(kind.to_u32().to_le_bytes());
        overall.update((payload.len() as u32).to_le_bytes());
        overall.update(payload);
    }

    let mut overall_digest = [0_u8; OVERALL_HASH_LEN];
    overall_digest.copy_from_slice(&overall.finalize());
    SealSection {
        entries,
        overall: overall_digest,
    }
}

/// Verify the SEAL section against the covered sections.
///
/// Returns [`Error::SealMismatch`] on any disagreement.
pub fn verify_seal(
    seal: &SealSection,
    header: &Header,
    covered: &[(SectionKind, &[u8])],
) -> Result<()> {
    let expected = compute_seal(header, covered);
    if seal.entries != expected.entries {
        return Err(Error::SealMismatch {
            what: "section hashes",
        });
    }
    if seal.overall != expected.overall {
        return Err(Error::SealMismatch {
            what: "overall hash",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{compute_seal, verify_seal, SealSection};
    use crate::header::{Header, VERSION};
    use crate::section::SectionKind;

    fn header() -> Header {
        Header {
            version: VERSION,
            flags: 0,
            section_count: 2,
            header_crc32: 0,
        }
    }

    fn covered() -> Vec<(SectionKind, &'static [u8])> {
        vec![
            (SectionKind::Info, b"{\"info\":1}"),
            (SectionKind::Content, b"text payload"),
        ]
    }

    #[test]
    fn round_trip() {
        let seal = compute_seal(&header(), &covered());
        let bytes = seal.encode();
        assert_eq!(SealSection::decode(&bytes).expect("decode"), seal);
    }

    #[test]
    fn verify_passes() {
        let seal = compute_seal(&header(), &covered());
        verify_seal(&seal, &header(), &covered()).expect("verify");
    }

    #[test]
    fn verify_fails_on_tamper() {
        let seal = compute_seal(&header(), &covered());
        let mut tampered = covered();
        tampered[1].1 = b"tampered payload";
        assert!(verify_seal(&seal, &header(), &tampered).is_err());
    }

    #[test]
    fn verify_fails_on_reordering() {
        let seal = compute_seal(&header(), &covered());
        let mut reversed = covered();
        reversed.reverse();
        assert!(verify_seal(&seal, &header(), &reversed).is_err());
    }

    #[test]
    fn decode_rejects_bad_mode() {
        let seal = compute_seal(&header(), &covered());
        let mut bytes = seal.encode();
        bytes[0] = 9;
        assert!(SealSection::decode(&bytes).is_err());
    }
}
