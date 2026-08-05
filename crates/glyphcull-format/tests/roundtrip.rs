//! Writer → reader round-trip over a full-featured synthetic package, plus
//! determinism assertions.

// Integration tests are not public API; the workspace `missing_docs` policy does
// not apply to test-only helpers.
#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

use glyphcull_format::codec::chunk::{
    flags, ChunkExtra, ChunkExtraKind, ChunkKind, ChunkRecord, ChunkSection,
};
use glyphcull_format::codec::content::{ContentSection, Payload, PayloadKind};
use glyphcull_format::codec::glyph::{glyph_flags, Atlas, GlyphRecord, GlyphSection, KerningPair};
use glyphcull_format::codec::image::{Image, ImageFormat, ImageSection};
use glyphcull_format::codec::info::Info;
use glyphcull_format::codec::seal::SealSection;
use glyphcull_format::codec::style::{
    PropertyTag, PropertyValue, StyleProperty, StyleRecord, StyleSection,
};
use glyphcull_format::reader::parse;
use glyphcull_format::section::SectionKind;
use glyphcull_format::table::Compression;
use glyphcull_format::validate::validate_package;
use glyphcull_format::writer::PackageBuilder;

fn document() -> ChunkRecord {
    ChunkRecord {
        kind: ChunkKind::Document,
        flags: flags::STRUCTURAL,
        style_id: 0,
        parent_id: 0,
        prev_id: 0,
        next_id: 0,
        first_child_id: 2,
        last_child_id: 5,
        content_index: 0,
        ordinal: 0,
        depth: 0,
    }
}

fn paragraph(_id: u32, first_child: u32, last_child: u32) -> ChunkRecord {
    ChunkRecord {
        kind: ChunkKind::Paragraph,
        flags: 0,
        style_id: 1,
        parent_id: 1,
        prev_id: 0,
        next_id: 5,
        first_child_id: first_child,
        last_child_id: last_child,
        content_index: 1,
        ordinal: 1,
        depth: 1,
    }
}

fn run(id: u32, content: u32, style: u32, parent: u32, prev: u32, next: u32) -> ChunkRecord {
    ChunkRecord {
        kind: ChunkKind::Run,
        flags: 0,
        style_id: style,
        parent_id: parent,
        prev_id: prev,
        next_id: next,
        first_child_id: 0,
        last_child_id: 0,
        content_index: content,
        ordinal: id - 1,
        depth: 2,
    }
}

pub fn build_full_package() -> Vec<u8> {
    let info = Info {
        format_version: 1,
        generator: "glyphcull-format".to_string(),
        generator_version: "0.1.0".to_string(),
        source_digest: "ab".repeat(32),
        document_id: "cd".repeat(16),
        title: Some("Round-trip fixture".to_string()),
        lang: Some("en".to_string()),
        chunk_count: 5,
        style_count: 2,
        content_count: 3,
        atlas_count: 1,
        image_count: 1,
    };

    let chunks = ChunkSection {
        chunks: vec![
            document(),
            paragraph(2, 3, 4),
            run(3, 1, 1, 2, 0, 4),
            run(4, 2, 0, 2, 3, 0),
            ChunkRecord {
                kind: ChunkKind::Image,
                flags: 0,
                style_id: 0,
                parent_id: 1,
                prev_id: 2,
                next_id: 0,
                first_child_id: 0,
                last_child_id: 0,
                content_index: 3,
                ordinal: 4,
                depth: 1,
            },
        ],
        extras: vec![
            ChunkExtra {
                chunk_id: 5,
                kind: ChunkExtraKind::ImageAlt,
                data: b"a diagram".to_vec(),
            },
            ChunkExtra {
                chunk_id: 4,
                kind: ChunkExtraKind::LinkTarget,
                data: {
                    let url = b"https://example.com/a?b=c";
                    let mut v = Vec::new();
                    v.extend_from_slice(&(url.len() as u16).to_le_bytes());
                    v.extend_from_slice(url);
                    v
                },
            },
        ],
    };

    let styles = StyleSection {
        styles: vec![
            StyleRecord {
                id: 0,
                properties: vec![
                    StyleProperty {
                        tag: PropertyTag::FontId,
                        value: PropertyValue::U32(0),
                    },
                    StyleProperty {
                        tag: PropertyTag::FontSizePx,
                        value: PropertyValue::F32(16.0),
                    },
                    StyleProperty {
                        tag: PropertyTag::LineHeight,
                        value: PropertyValue::F32(1.5),
                    },
                    StyleProperty {
                        tag: PropertyTag::Color,
                        value: PropertyValue::U32(0x00_0000FF),
                    },
                ],
            },
            StyleRecord {
                id: 1,
                properties: vec![
                    StyleProperty {
                        tag: PropertyTag::Italic,
                        value: PropertyValue::U8(1),
                    },
                    StyleProperty {
                        tag: PropertyTag::FontWeight,
                        value: PropertyValue::U16(700),
                    },
                    StyleProperty {
                        tag: PropertyTag::TextAlign,
                        value: PropertyValue::U8(3),
                    },
                ],
            },
        ],
    };

    let content = ContentSection {
        payloads: vec![
            Payload {
                kind: PayloadKind::TextUtf8,
                data: "The quick brown fox.".as_bytes().to_vec(),
            },
            Payload {
                kind: PayloadKind::TextUtf8,
                data: "Some emphasized text \u{4e2d}\u{6587}.".as_bytes().to_vec(),
            },
            Payload {
                kind: PayloadKind::ImageRef,
                data: 0_u32.to_le_bytes().to_vec(),
            },
        ],
    };

    let page_w = 32_u32;
    let page_h = 32_u32;
    let page = vec![128_u8; (page_w * page_h * 4) as usize];
    let glyphs = GlyphSection {
        atlases: vec![Atlas {
            font_id: 0,
            format: 0,
            padding: 4,
            texels_per_em: 32768,
            ascent: 0.75,
            descent: 0.25,
            line_gap: 0.0,
            cap_height: 0.7,
            x_height: 0.5,
            units_per_em: 1000.0,
            family: "Fixture Sans".to_string(),
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
                    box_x: 2,
                    box_y: 2,
                    box_w: 28,
                    box_h: 28,
                    page_index: 0,
                    flags: 0,
                },
                GlyphRecord {
                    codepoint: ' ' as u32,
                    advance: 0.25,
                    bearing_x: 0.0,
                    bearing_y: 0.0,
                    box_x: 2,
                    box_y: 2,
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
        }],
    };

    let images = ImageSection {
        images: vec![Image {
            width: 2,
            height: 2,
            format: ImageFormat::Rgba8,
            data: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        }],
    };

    let mut builder = PackageBuilder::new().with_seal(true);
    builder
        .add(SectionKind::Info, info.encode(), Compression::Zlib)
        .expect("add info");
    builder
        .add(SectionKind::Chunk, chunks.encode(), Compression::Zlib)
        .expect("add chunk");
    builder
        .add(SectionKind::Style, styles.encode(), Compression::Zlib)
        .expect("add style");
    builder
        .add(SectionKind::Content, content.encode(), Compression::Zlib)
        .expect("add content");
    builder
        .add(SectionKind::Glyph, glyphs.encode(), Compression::None)
        .expect("add glyph");
    builder
        .add(SectionKind::Images, images.encode(), Compression::None)
        .expect("add images");
    builder.build().expect("build")
}

#[test]
fn full_round_trip() {
    let bytes = build_full_package();
    let pkg = parse(&bytes).expect("parse");

    // Header.
    assert_eq!(pkg.header.section_count, 7);
    assert_eq!(pkg.header.version, 1);

    // INFO.
    let info = Info::decode(pkg.section(SectionKind::Info).expect("info")).expect("decode info");
    assert_eq!(info.chunk_count, 5);
    assert_eq!(info.style_count, 2);
    assert_eq!(info.content_count, 3);
    assert_eq!(info.atlas_count, 1);
    assert_eq!(info.image_count, 1);
    assert_eq!(info.title.as_deref(), Some("Round-trip fixture"));

    // CHNK.
    let chunks = ChunkSection::decode(pkg.section(SectionKind::Chunk).expect("chunk"))
        .expect("decode chunks");
    assert_eq!(chunks.chunks.len(), 5);
    assert_eq!(chunks.chunks[0].kind, ChunkKind::Document);
    assert_eq!(chunks.extras.len(), 2);

    // STYL.
    let styles = StyleSection::decode(pkg.section(SectionKind::Style).expect("style"))
        .expect("decode styles");
    assert_eq!(styles.styles.len(), 2);
    assert_eq!(styles.styles[1].properties.len(), 3);

    // CONT.
    let content = ContentSection::decode(pkg.section(SectionKind::Content).expect("content"))
        .expect("decode content");
    assert_eq!(content.payloads[0].data, b"The quick brown fox.");

    // GLYF.
    let glyphs = GlyphSection::decode(pkg.section(SectionKind::Glyph).expect("glyph"))
        .expect("decode glyph");
    assert_eq!(glyphs.atlases[0].glyphs.len(), 2);
    assert_eq!(glyphs.atlases[0].pages.len(), 1);

    // IMGS.
    let images = ImageSection::decode(pkg.section(SectionKind::Images).expect("images"))
        .expect("decode images");
    assert_eq!(images.images[0].width, 2);

    // SEAL verifies.
    let seal =
        SealSection::decode(pkg.section(SectionKind::Seal).expect("seal")).expect("decode seal");
    assert_eq!(seal.entries.len(), 6);

    // Semantic validation passes.
    let issues = validate_package(&pkg);
    assert!(issues.is_empty(), "issues: {issues:?}");
}

#[test]
fn full_round_trip_deterministic() {
    assert_eq!(build_full_package(), build_full_package());
}

#[test]
fn canonical_section_order() {
    let bytes = build_full_package();
    let pkg = parse(&bytes).expect("parse");
    let kinds: Vec<u32> = pkg.entries.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, vec![1, 2, 3, 4, 5, 6, 7]);
    // Offsets are strictly increasing and contiguous.
    for pair in pkg.entries.windows(2) {
        assert_eq!(pair[0].offset + pair[0].stored_len, pair[1].offset);
    }
}
