//! Malformed-input corpus: the reader must never panic and must reject corruption.
//!
//! Two disciplines are tested here:
//! - **Truncation**: every proper prefix of a valid package must fail to parse.
//! - **Bit flips**: single-byte mutations either fail to parse or produce a valid
//!   package (corruption is either detected or innocuous). Structural fields must be
//!   detected; content bytes may legitimately produce different valid content.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

mod roundtrip;

use glyphcull_format::reader::parse;

/// Every proper prefix of a valid package must be rejected (typed error, no panic).
#[test]
fn every_truncation_is_rejected() {
    let bytes = roundtrip::build_full_package();
    for len in 0..bytes.len() {
        let prefix = &bytes[..len];
        let result = parse(prefix);
        assert!(
            result.is_err(),
            "prefix of length {len} parsed successfully; this must not happen for a proper prefix"
        );
    }
    // The full package parses.
    parse(&bytes).expect("full package parses");
}

/// Mutating the magic must be caught.
#[test]
fn magic_flip_detected() {
    let bytes = roundtrip::build_full_package();
    for i in 0..4 {
        let mut mutated = bytes.clone();
        mutated[i] ^= 0x01;
        assert!(parse(&mutated).is_err(), "magic byte {i} flip not detected");
    }
}

/// Mutating the version must be caught.
#[test]
fn version_flip_detected() {
    let bytes = roundtrip::build_full_package();
    let mut mutated = bytes.clone();
    mutated[4] ^= 0x40;
    assert!(parse(&mutated).is_err());
}

/// Mutating the header section count must be caught (CRC or limits).
#[test]
fn section_count_flip_detected() {
    let bytes = roundtrip::build_full_package();
    let mut mutated = bytes.clone();
    mutated[8] ^= 0x01;
    assert!(parse(&mutated).is_err());
}

/// Mutating section table structural fields must be caught (bounds, CRC, lengths).
#[test]
fn table_field_flips_detected() {
    let bytes = roundtrip::build_full_package();
    for offset in [16, 24, 32, 40, 48, 56, 64, 72] {
        let mut mutated = bytes.clone();
        // Flip the low byte of the u64 offset field of the first entry.
        mutated[offset] ^= 0x01;
        assert!(
            parse(&mutated).is_err(),
            "offset field byte at {offset} flip not detected"
        );
    }
}

/// Flipping a content byte either errors (CRC) or yields different valid content:
/// both are acceptable, but parsing must never panic.
#[test]
fn content_flips_never_panic() {
    let bytes = roundtrip::build_full_package();
    for i in 0..bytes.len() {
        let mut mutated = bytes.clone();
        mutated[i] ^= 0x01;
        // No panic is the assertion; both outcomes are legitimate here.
        let _ = parse(&mutated);
    }
}

/// Truncation at every prefix combined with flips must never panic.
#[test]
fn truncation_plus_flips_never_panic() {
    let bytes = roundtrip::build_full_package();
    for len in (0..bytes.len()).step_by(7) {
        let mut mutated = bytes[..len].to_vec();
        if let Some(last) = mutated.last_mut() {
            *last ^= 0xFF;
        }
        let _ = parse(&mutated);
    }
}

/// A package with an unknown (reserved) section kind is skipped, not rejected.
#[test]
fn unknown_section_kind_is_skipped() {
    use glyphcull_format::reader::parse;
    use glyphcull_format::section::SectionKind;
    use glyphcull_format::table::Compression;
    use glyphcull_format::writer::PackageBuilder;

    let mut builder = PackageBuilder::new();
    builder
        .add(SectionKind::Info, b"{\"i\":1}".to_vec(), Compression::Zlib)
        .expect("add");
    let bytes = builder.build().expect("build");

    // Append an unknown-kind section by hand: entry (kind=8) + payload.
    // Rebuild: header count 2, table 64 bytes, then payloads.
    // Simplest: parse the built package and re-emit with an extra section via the
    // writer is not possible (writer rejects unknown kinds), so hand-assemble:
    let mut out = Vec::new();
    out.extend_from_slice(&bytes[..16]); // header: count=1 → fix to 2 and CRC
    let header = &mut out[..16];
    header[8] = 2; // section_count
                   // Recompute header CRC (bytes 0..12).
    let crc = glyphcull_format::crc32::crc32(&out[..12]);
    out[12..16].copy_from_slice(&crc.to_le_bytes());

    // Original table (1 entry at 16..48) + new entry (kind 8).
    out.extend_from_slice(&bytes[16..48]);
    // The INFO payload now sits at offset 16 + 64 = 80 (extended table); patch the
    // copied entry's offset field (entry bytes 8..16, i.e. out[24..32]).
    let info_offset = (16 + 64) as u64;
    out[24..32].copy_from_slice(&info_offset.to_le_bytes());
    let payload = b"reserved-future-section".to_vec();
    let mut entry = Vec::new();
    entry.extend_from_slice(&8_u32.to_le_bytes()); // kind
    entry.push(0); // compression
    entry.push(0); // flags
    entry.extend_from_slice(&0_u16.to_le_bytes()); // reserved
                                                   // The unknown payload follows the INFO payload (whose stored length is known
                                                   // from the original package).
    let orig = parse(&bytes).expect("parse original");
    let info_entry = &orig.entries[0];
    let unknown_offset = info_offset + info_entry.stored_len;
    entry.extend_from_slice(&unknown_offset.to_le_bytes());
    entry.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    entry.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    entry.extend_from_slice(&glyphcull_format::crc32::crc32(&payload).to_le_bytes());
    assert_eq!(entry.len(), 32);
    out.extend_from_slice(&entry);

    // Original payloads (after the original table): the original payload offset was
    // 48; with the extended table it moves to 80. Re-emit payload bytes directly.
    let orig_payload_start = info_entry.offset as usize;
    out.extend_from_slice(&bytes[orig_payload_start..]);
    out.extend_from_slice(&payload);

    let pkg = parse(&out).expect("parses with unknown section");
    assert_eq!(pkg.unknown.len(), 1);
    assert_eq!(pkg.unknown[0].0, 8);
    assert_eq!(pkg.unknown[0].1.payload, payload);
}
