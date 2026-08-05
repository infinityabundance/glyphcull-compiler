//! Property tests for the format crate (proptest).
//!
//! Properties:
//! - Round-trip: `decode(encode(model)) == model` for generated section models.
//! - Totality: the reader never panics on arbitrary bytes (it errors or succeeds).
//! - Determinism: equal inputs produce byte-equal packages.
//! - Valid generated chunk graphs pass semantic validation.

#![allow(missing_docs)]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use proptest::prelude::*;

use glyphcull_format::codec::chunk::{flags, ChunkKind, ChunkRecord, ChunkSection};
use glyphcull_format::codec::content::{ContentSection, Payload, PayloadKind};
use glyphcull_format::codec::glyph::{glyph_flags, Atlas, GlyphRecord, GlyphSection, KerningPair};
use glyphcull_format::codec::image::{Image, ImageFormat, ImageSection};
use glyphcull_format::codec::info::Info;
use glyphcull_format::codec::style::{
    PropertyTag, PropertyValue, StyleProperty, StyleRecord, StyleSection,
};
use glyphcull_format::reader::parse;
use glyphcull_format::section::SectionKind;
use glyphcull_format::table::Compression;
use glyphcull_format::validate::validate_package;
use glyphcull_format::writer::PackageBuilder;

/// A small valid atlas (8×8 page, 2 glyphs) for generated packages.
fn make_atlas(seed: u8) -> Atlas {
    let page_w = 8_u32;
    let page_h = 8_u32;
    let mut page = vec![0_u8; (page_w * page_h * 4) as usize];
    page[0] = seed;
    Atlas {
        font_id: 0,
        format: 0,
        padding: 2,
        texels_per_em: 16384,
        ascent: 0.75,
        descent: 0.25,
        line_gap: 0.0,
        cap_height: 0.7,
        x_height: 0.5,
        units_per_em: 1000.0,
        family: "Prop Sans".to_string(),
        weight: 400,
        italic: false,
        page_width: page_w,
        page_height: page_h,
        glyphs: vec![
            GlyphRecord {
                codepoint: 'A' as u32,
                advance: 0.6,
                bearing_x: 0.05,
                bearing_y: 0.7,
                box_x: 1,
                box_y: 1,
                box_w: 4,
                box_h: 4,
                page_index: 0,
                flags: 0,
            },
            GlyphRecord {
                codepoint: ' ' as u32,
                advance: 0.25,
                bearing_x: 0.0,
                bearing_y: 0.0,
                box_x: 1,
                box_y: 1,
                box_w: 1,
                box_h: 1,
                page_index: 0,
                flags: glyph_flags::NO_OUTLINE,
            },
        ],
        kerning: vec![KerningPair {
            left: 'A' as u32,
            right: 'V' as u32,
            adjust: -0.05,
        }],
        pages: vec![page],
    }
}

proptest! {
    /// Chunk graphs that satisfy the tree invariants must pass validation.
    #[test]
    fn valid_chunk_graphs_pass_validation(seed in any::<u16>()) {
        let (chunks, chunk_count) = build_tree(seed);
        let info = Info {
            format_version: 1,
            generator: "prop".to_string(),
            generator_version: "0.0.0".to_string(),
            source_digest: "ab".repeat(32),
            document_id: "cd".repeat(16),
            title: None,
            lang: None,
            chunk_count,
            style_count: 1,
            content_count: 1,
            atlas_count: 1,
            image_count: 0,
        };
        let styles = StyleSection {
            styles: vec![StyleRecord {
                id: 0,
                properties: vec![
                    StyleProperty { tag: PropertyTag::FontId, value: PropertyValue::U32(0) },
                    StyleProperty { tag: PropertyTag::FontSizePx, value: PropertyValue::F32(16.0) },
                ],
            }],
        };
        let content = ContentSection {
            payloads: vec![Payload { kind: PayloadKind::TextUtf8, data: b"x".to_vec() }],
        };
        let mut builder = PackageBuilder::new();
        builder.add(SectionKind::Info, info.encode(), Compression::Zlib).unwrap();
        builder.add(SectionKind::Chunk, chunks.encode(), Compression::Zlib).unwrap();
        builder.add(SectionKind::Style, styles.encode(), Compression::Zlib).unwrap();
        builder.add(SectionKind::Content, content.encode(), Compression::Zlib).unwrap();
        builder
            .add(
                SectionKind::Glyph,
                GlyphSection { atlases: vec![make_atlas(seed as u8)] }.encode(),
                Compression::None,
            )
            .unwrap();
        let pkg = parse(&builder.build().unwrap()).unwrap();
        let issues = validate_package(&pkg);
        prop_assert!(issues.is_empty(), "unexpected issues: {issues:?}");
    }

    /// decode(encode(x)) == x for generated style sections.
    #[test]
    fn style_round_trip(records in 0..8usize) {
        let styles = StyleSection {
            styles: (0..records)
                .map(|id| StyleRecord {
                    id: id as u32,
                    properties: vec![
                        StyleProperty { tag: PropertyTag::FontId, value: PropertyValue::U32(0) },
                        StyleProperty { tag: PropertyTag::FontSizePx, value: PropertyValue::F32(16.0) },
                        StyleProperty { tag: PropertyTag::Color, value: PropertyValue::U32(0x00_0000FF) },
                    ],
                })
                .collect(),
        };
        let bytes = styles.encode();
        let decoded = StyleSection::decode(&bytes).unwrap();
        prop_assert_eq!(decoded, styles);
    }

    /// decode(encode(x)) == x for generated content sections.
    #[test]
    fn content_round_trip(payloads in 0..8usize) {
        let content = ContentSection {
            payloads: (0..payloads)
                .map(|i| {
                    if i % 3 == 2 {
                        Payload {
                            kind: PayloadKind::ImageRef,
                            data: 0_u32.to_le_bytes().to_vec(),
                        }
                    } else {
                        Payload {
                            kind: PayloadKind::TextUtf8,
                            data: format!("payload {i} text \u{4e2d}").into_bytes(),
                        }
                    }
                })
                .collect(),
        };
        let bytes = content.encode();
        let decoded = ContentSection::decode(&bytes).unwrap();
        prop_assert_eq!(decoded, content);
    }

    /// decode(encode(x)) == x for generated image sections.
    #[test]
    fn image_round_trip(images in 0..4usize) {
        let image_section = ImageSection {
            images: (0..images)
                .map(|i| Image {
                    width: 2,
                    height: 2,
                    format: ImageFormat::Rgba8,
                    data: vec![i as u8; 16],
                })
                .collect(),
        };
        let bytes = image_section.encode();
        let decoded = ImageSection::decode(&bytes).unwrap();
        prop_assert_eq!(decoded, image_section);
    }

    /// The reader is total: arbitrary bytes never panic; they parse or error.
    #[test]
    fn reader_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let _ = parse(&bytes);
    }

    /// A corrupting mutation either errors or parses; never panics.
    #[test]
    fn mutations_never_panic(bytes in prop::collection::vec(any::<u8>(), 1..2048)) {
        let mut mutated = bytes.clone();
        if let Some(last) = mutated.last_mut() {
            *last ^= 0xFF;
        }
        let _ = parse(&mutated);
    }

    /// Writer determinism: equal section sets produce byte-equal packages.
    #[test]
    fn writer_deterministic(payload in prop::collection::vec(any::<u8>(), 1..512)) {
        let build = || {
            let mut builder = PackageBuilder::new().with_seal(true);
            builder
                .add(SectionKind::Content, payload.clone(), Compression::Zlib)
                .unwrap();
            builder.build().unwrap()
        };
        prop_assert_eq!(build(), build());
    }
}

/// Generate a small valid chunk tree (document + paragraphs) with a deterministic
/// pseudo-random shape. Returns the section and its chunk count.
fn build_tree(seed: u16) -> (ChunkSection, u32) {
    let para_count = 1 + (seed % 6) as usize;
    let mut chunks = vec![ChunkRecord {
        kind: ChunkKind::Document,
        flags: flags::STRUCTURAL,
        style_id: 0,
        parent_id: 0,
        prev_id: 0,
        next_id: 0,
        first_child_id: if para_count > 0 { 2 } else { 0 },
        last_child_id: if para_count > 0 {
            (para_count + 1) as u32
        } else {
            0
        },
        content_index: 0,
        ordinal: 0,
        depth: 0,
    }];
    for i in 0..para_count {
        let id = (i + 2) as u32; // 1-based: doc is 1, paras are 2..=n+1
        let prev = if i == 0 { 0 } else { id - 1 };
        let next = if i + 1 == para_count { 0 } else { id + 1 };
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
    let count = (para_count + 1) as u32;
    (
        ChunkSection {
            chunks,
            extras: vec![],
        },
        count,
    )
}
