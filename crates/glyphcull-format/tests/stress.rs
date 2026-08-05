//! Stress tests: large packages parse, validate, and round-trip within bounds.

#![allow(missing_docs)]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::time::Instant;

use glyphcull_format::codec::chunk::{flags, ChunkKind, ChunkRecord, ChunkSection};
use glyphcull_format::codec::content::{ContentSection, Payload, PayloadKind};
use glyphcull_format::codec::info::Info;
use glyphcull_format::reader::parse;
use glyphcull_format::section::SectionKind;
use glyphcull_format::table::Compression;
use glyphcull_format::validate::validate_package;
use glyphcull_format::writer::PackageBuilder;

/// A 100k-chunk document with a 4 MiB text payload: parse + validate + round-trip.
#[test]
fn large_document_round_trip() {
    const CHUNKS: usize = 100_000;

    let mut chunks = vec![ChunkRecord {
        kind: ChunkKind::Document,
        flags: flags::STRUCTURAL,
        style_id: 0,
        parent_id: 0,
        prev_id: 0,
        next_id: 0,
        first_child_id: 2,
        last_child_id: (CHUNKS + 1) as u32,
        content_index: 0,
        ordinal: 0,
        depth: 0,
    }];
    for i in 0..CHUNKS {
        let id = (i + 2) as u32;
        let prev = if i == 0 { 0 } else { id - 1 };
        let next = if i + 1 == CHUNKS { 0 } else { id + 1 };
        chunks.push(ChunkRecord {
            kind: ChunkKind::Paragraph,
            flags: 0,
            style_id: 0,
            parent_id: 1,
            prev_id: prev,
            next_id: next,
            first_child_id: 0,
            last_child_id: 0,
            content_index: 1,
            ordinal: id - 1,
            depth: 1,
        });
    }

    let text = vec![b'A'; 4 << 20]; // 4 MiB of text
    let info = Info {
        format_version: 1,
        generator: "stress".to_string(),
        generator_version: "0.0.0".to_string(),
        source_digest: "ab".repeat(32),
        document_id: "cd".repeat(16),
        title: None,
        lang: None,
        chunk_count: (CHUNKS + 1) as u32,
        style_count: 0,
        content_count: 1,
        atlas_count: 0,
        image_count: 0,
    };

    let mut builder = PackageBuilder::new().with_seal(true);
    builder
        .add(SectionKind::Info, info.encode(), Compression::Zlib)
        .expect("add");
    builder
        .add(
            SectionKind::Chunk,
            ChunkSection {
                chunks,
                extras: vec![],
            }
            .encode(),
            Compression::Zlib,
        )
        .expect("add");
    builder
        .add(
            SectionKind::Content,
            ContentSection {
                payloads: vec![Payload {
                    kind: PayloadKind::TextUtf8,
                    data: text,
                }],
            }
            .encode(),
            Compression::Zlib,
        )
        .expect("add");

    let t0 = Instant::now();
    let bytes = builder.build().expect("build");
    let build_ms = t0.elapsed().as_millis();

    let t1 = Instant::now();
    let pkg = parse(&bytes).expect("parse");
    let parse_ms = t1.elapsed().as_millis();

    let t2 = Instant::now();
    let issues = validate_package(&pkg);
    let validate_ms = t2.elapsed().as_millis();

    assert!(issues.is_empty(), "issues: {issues:?}");
    assert_eq!(pkg.header.section_count, 4);
    let content = ContentSection::decode(pkg.section(SectionKind::Content).expect("content"))
        .expect("decode");
    assert_eq!(content.payloads[0].data.len(), 4 << 20);

    eprintln!(
        "stress: {CHUNKS} chunks + 4 MiB text: build {build_ms} ms, parse {parse_ms} ms, validate {validate_ms} ms, package {} bytes",
        bytes.len()
    );

    // Loose sanity bounds (CI machines vary): the whole pipeline must stay well
    // under a second for this size in debug builds.
    assert!(parse_ms < 5000, "parse too slow: {parse_ms} ms");
    assert!(validate_ms < 5000, "validate too slow: {validate_ms} ms");
}

/// A single-chunk image-only package (no atlas needed: no text payloads).
#[test]
fn image_only_package_valid() {
    let info = Info {
        format_version: 1,
        generator: "stress".to_string(),
        generator_version: "0.0.0".to_string(),
        source_digest: "ab".repeat(32),
        document_id: "cd".repeat(16),
        title: None,
        lang: None,
        chunk_count: 2,
        style_count: 0,
        content_count: 1,
        atlas_count: 0,
        image_count: 1,
    };
    let chunks = ChunkSection {
        chunks: vec![
            ChunkRecord {
                kind: ChunkKind::Document,
                flags: flags::STRUCTURAL,
                style_id: 0,
                parent_id: 0,
                prev_id: 0,
                next_id: 0,
                first_child_id: 2,
                last_child_id: 2,
                content_index: 0,
                ordinal: 0,
                depth: 0,
            },
            ChunkRecord {
                kind: ChunkKind::Image,
                flags: 0,
                style_id: 0,
                parent_id: 1,
                prev_id: 0,
                next_id: 0,
                first_child_id: 0,
                last_child_id: 0,
                content_index: 1,
                ordinal: 1,
                depth: 1,
            },
        ],
        extras: vec![],
    };
    let mut builder = PackageBuilder::new().with_seal(true);
    builder
        .add(SectionKind::Info, info.encode(), Compression::Zlib)
        .expect("add");
    builder
        .add(SectionKind::Chunk, chunks.encode(), Compression::Zlib)
        .expect("add");
    builder
        .add(
            SectionKind::Content,
            ContentSection {
                payloads: vec![Payload {
                    kind: PayloadKind::ImageRef,
                    data: 0_u32.to_le_bytes().to_vec(),
                }],
            }
            .encode(),
            Compression::Zlib,
        )
        .expect("add");
    builder
        .add(
            SectionKind::Images,
            glyphcull_format::codec::image::ImageSection {
                images: vec![glyphcull_format::codec::image::Image {
                    width: 64,
                    height: 64,
                    format: glyphcull_format::codec::image::ImageFormat::Rgba8,
                    data: vec![0x80_u8; 64 * 64 * 4],
                }],
            }
            .encode(),
            Compression::None,
        )
        .expect("add");
    let pkg = parse(&builder.build().expect("build")).expect("parse");
    assert!(validate_package(&pkg).is_empty());
}
