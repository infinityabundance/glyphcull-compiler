//! Golden minimal package: an INFO-only package with fully pinned bytes.
//!
//! The pinned literal is the *contract on disk*: any change to the byte layout,
//! header fields, section table, CRC handling, or zlib output breaks this test with
//! a visible diff. The test also verifies the pinned bytes structurally, using only
//! the independent primitives (crc32, header decode), so the literal is checked
//! against the specification, not just against the writer.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

use glyphcull_format::codec::info::Info;
use glyphcull_format::crc32::crc32;
use glyphcull_format::header::HEADER_LEN;
use glyphcull_format::reader::parse;
use glyphcull_format::section::SectionKind;
use glyphcull_format::table::{Compression, SectionEntry, SECTION_ENTRY_LEN};
use glyphcull_format::writer::PackageBuilder;

/// The minimal package: a single INFO section, zlib-compressed.
fn build_minimal() -> Vec<u8> {
    let info = Info {
        format_version: 1,
        generator: "glyphcull-format".to_string(),
        generator_version: "0.1.0".to_string(),
        source_digest: "00".repeat(32),
        document_id: "0123456789abcdef0123456789abcdef".to_string(),
        title: None,
        lang: None,
        chunk_count: 0,
        style_count: 0,
        content_count: 0,
        atlas_count: 0,
        image_count: 0,
    };
    let mut builder = PackageBuilder::new();
    builder
        .add(SectionKind::Info, info.encode(), Compression::Zlib)
        .expect("add");
    builder.build().expect("build")
}

/// The pinned bytes of the minimal package (hex). Generated deliberately; see the
/// module docs. Regenerated only with a reviewed diff via `scripts/regenerate-golden.sh`.
const PINNED_HEX: &str = "43554c4c0100000001000000cfa2f7fd010000000100000030000000000000009300000000000000250100009859a04a78daa54ecb0ec22010fc973dd786fab63fd3206c291158038b4963fc77693c9478eddee6b133f306c94ea641510e0cbd68404d393c6a4c813170c56852d92f94d5d083e8f687e3e97cb9dee45d691cff31343052f4928717c6642940df35603060944cb12418373f27959ddbfd8c50c9eb1388b66b45d1ac9706ab3989725438686b30f1e2db78a522f1ecd68acf176a9058be";

#[test]
fn pinned_bytes_unchanged() {
    let expected: Vec<u8> = hex::decode(PINNED_HEX).expect("pinned hex decodes");
    assert_eq!(
        build_minimal(),
        expected,
        "package bytes drifted from the pinned golden; review the diff deliberately"
    );
}

#[test]
fn pinned_bytes_structurally_valid() {
    let bytes: Vec<u8> = hex::decode(PINNED_HEX).expect("pinned hex decodes");
    assert!(bytes.len() > HEADER_LEN + SECTION_ENTRY_LEN);

    // Header layout (independent verification).
    assert_eq!(&bytes[0..4], b"CULL");
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 1, "version");
    assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), 0, "flags");
    assert_eq!(
        u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        1,
        "section_count"
    );
    assert_eq!(
        u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
        crc32(&bytes[..12]),
        "header crc"
    );

    // Section table entry layout.
    let entry = SectionEntry::decode(
        &bytes[HEADER_LEN..HEADER_LEN + SECTION_ENTRY_LEN]
            .try_into()
            .expect("entry bytes"),
    )
    .expect("entry decodes");
    assert_eq!(entry.kind, SectionKind::Info.to_u32());
    assert_eq!(entry.compression, Compression::Zlib);
    assert_eq!(entry.offset, (HEADER_LEN + SECTION_ENTRY_LEN) as u64);
    assert_eq!(entry.offset + entry.stored_len, bytes.len() as u64);
    // decoded_len must equal the decoded INFO payload length.
    let decoded_payload = glyphcull_format::compress::zlib_decompress(
        &bytes[entry.offset as usize..bytes.len()],
        entry.decoded_len as usize,
    )
    .expect("pinned zlib decodes");
    assert_eq!(entry.decoded_len, decoded_payload.len() as u64);

    // The stored payload region decodes to the expected INFO payload.
    let pkg = parse(&bytes).expect("pinned bytes parse");
    let decoded =
        Info::decode(pkg.section(SectionKind::Info).expect("info")).expect("info decodes");
    assert_eq!(decoded.generator, "glyphcull-format");
    assert_eq!(decoded.chunk_count, 0);
    assert_eq!(decoded.source_digest, "00".repeat(32));
}

/// Regenerates `tests/fixtures/v1-minimal.cull` from the writer. Run deliberately
/// via `scripts/regenerate-golden.sh` (which refuses a dirty tree).
#[test]
#[ignore]
fn regenerate_fixture() {
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixtures dir");
    let path = out_dir.join("v1-minimal.cull");
    std::fs::write(&path, build_minimal()).expect("write fixture");
    eprintln!("wrote {}", path.display());
}

/// Prints the minimal package as hex (used once to establish the pin).
#[test]
#[ignore]
fn print_hex() {
    eprintln!("{}", hex::encode(&build_minimal()));
}

/// Minimal hex decoder for the pinned literal (avoids a dev-dependency).
mod hex {
    pub fn decode(s: &str) -> Result<Vec<u8>, String> {
        if s.len() % 2 != 0 {
            return Err("odd length".to_string());
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
            .collect()
    }

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }
}
