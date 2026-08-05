//! Semantic package validation (beyond the structural reader checks).
//!
//! `validate_package` checks the cross-section invariants of SPEC.md: INFO/count
//! agreement, the chunk graph's tree shape (single root, sibling consistency, depth,
//! dense ordinals, reference resolution, parent/child kind shape), style/font/content
//! reference resolution, and SEAL verification. It returns a list of issues; an
//! empty list means the package is valid.

use crate::codec::chunk::{ChunkExtraKind, ChunkKind, ChunkSection};
use crate::codec::content::{ContentSection, PayloadKind};
use crate::codec::glyph::GlyphSection;
use crate::codec::image::ImageSection;
use crate::codec::info::Info;
use crate::codec::seal::{verify_seal, SealSection};
use crate::codec::style::{PropertyTag, PropertyValue, StyleSection};
use crate::reader::ParsedPackage;
use crate::section::SectionKind;

/// One validation issue, addressed at a section level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    /// The section (or cross-section concern) the issue belongs to.
    pub section: &'static str,
    /// A precise, human-readable description.
    pub message: String,
}

impl ValidationIssue {
    fn new(section: &'static str, message: impl Into<String>) -> Self {
        Self {
            section,
            message: message.into(),
        }
    }
}

/// Validate a parsed package semantically. Returns issues; `Vec::is_empty()` means valid.
#[must_use]
pub fn validate_package(pkg: &ParsedPackage) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let info = match decode_section::<Info>(pkg, SectionKind::Info, &mut issues, "INFO") {
        Some(info) => Some(info),
        None => {
            issues.push(ValidationIssue::new(
                "INFO",
                "required INFO section missing or undecodable",
            ));
            None
        }
    };

    let chunks = decode_section::<ChunkSection>(pkg, SectionKind::Chunk, &mut issues, "CHNK");
    let styles = decode_section::<StyleSection>(pkg, SectionKind::Style, &mut issues, "STYL");
    let content = decode_section::<ContentSection>(pkg, SectionKind::Content, &mut issues, "CONT");
    let glyphs = decode_section::<GlyphSection>(pkg, SectionKind::Glyph, &mut issues, "GLYF");
    let images = decode_section::<ImageSection>(pkg, SectionKind::Images, &mut issues, "IMGS");

    // INFO counts must agree with the actual sections.
    if let Some(info) = &info {
        let expected = [
            (
                "chunk_count",
                info.chunk_count,
                chunks.as_ref().map_or(0, |c| c.len() as u32),
            ),
            (
                "style_count",
                info.style_count,
                styles.as_ref().map_or(0, |s| s.styles.len() as u32),
            ),
            (
                "content_count",
                info.content_count,
                content.as_ref().map_or(0, |c| c.payloads.len() as u32),
            ),
            (
                "atlas_count",
                info.atlas_count,
                glyphs.as_ref().map_or(0, |g| g.atlases.len() as u32),
            ),
            (
                "image_count",
                info.image_count,
                images.as_ref().map_or(0, |i| i.images.len() as u32),
            ),
        ];
        for (key, declared, actual) in expected {
            if declared != actual {
                issues.push(ValidationIssue::new(
                    "INFO",
                    format!("{key} declares {declared} but the section contains {actual}"),
                ));
            }
        }
    }

    // Chunk graph semantics.
    if let Some(chunks) = &chunks {
        validate_chunk_graph(chunks, styles.as_ref(), content.as_ref(), &mut issues);
    }

    // Styles: font_id references must resolve to an atlas.
    if let Some(styles) = &styles {
        let atlas_count = glyphs.as_ref().map_or(0, |g| g.atlases.len() as u32);
        for style in &styles.styles {
            for prop in &style.properties {
                if prop.tag == PropertyTag::FontId {
                    if let PropertyValue::U32(font_id) = prop.value {
                        if font_id >= atlas_count {
                            issues.push(ValidationIssue::new(
                                "STYL",
                                format!("style {} references font_id {font_id} but there are {atlas_count} atlases", style.id),
                            ));
                        }
                    }
                }
            }
        }
        // Text requires at least the default atlas.
        let has_text = content
            .as_ref()
            .is_some_and(|c| c.payloads.iter().any(|p| p.kind == PayloadKind::TextUtf8));
        if has_text && atlas_count == 0 {
            issues.push(ValidationIssue::new(
                "GLYF",
                "document contains text but no glyph atlas",
            ));
        }
    }

    // Content: image refs must resolve.
    if let Some(content) = &content {
        let image_count = images.as_ref().map_or(0, |i| i.images.len() as u32);
        for (i, payload) in content.payloads.iter().enumerate() {
            if payload.kind == PayloadKind::ImageRef {
                let id = match <[u8; 4]>::try_from(payload.data.as_slice()) {
                    Ok(bytes4) => u32::from_le_bytes(bytes4),
                    Err(_) => {
                        issues.push(ValidationIssue::new(
                            "CONT",
                            format!("payload {i} image_ref has wrong length"),
                        ));
                        continue;
                    }
                };
                if id >= image_count {
                    issues.push(ValidationIssue::new(
                        "CONT",
                        format!(
                            "payload {i} references image {id} but there are {image_count} images"
                        ),
                    ));
                }
            }
        }
    }

    // SEAL verification (when present).
    if let Some(seal_section) = pkg.sections.get(&SectionKind::Seal) {
        match SealSection::decode(&seal_section.payload) {
            Ok(seal) => {
                let covered: Vec<(SectionKind, &[u8])> = pkg
                    .sections
                    .iter()
                    .filter(|(kind, _)| **kind != SectionKind::Seal)
                    .map(|(kind, section)| (*kind, section.payload.as_slice()))
                    .collect();
                if let Err(e) = verify_seal(&seal, &pkg.header, &covered) {
                    issues.push(ValidationIssue::new(
                        "SEAL",
                        format!("verification failed: {e}"),
                    ));
                }
            }
            Err(e) => issues.push(ValidationIssue::new("SEAL", format!("undecodable: {e}"))),
        }
    }

    issues
}

/// True when the package is valid.
#[must_use]
pub fn is_valid(pkg: &ParsedPackage) -> bool {
    validate_package(pkg).is_empty()
}

/// Decode a section into its semantic model, recording issues on failure.
fn decode_section<T>(
    pkg: &ParsedPackage,
    kind: SectionKind,
    issues: &mut Vec<ValidationIssue>,
    name: &'static str,
) -> Option<T>
where
    T: DecodePayload,
{
    let payload = pkg.section(kind)?;
    match T::decode_payload(payload) {
        Ok(value) => Some(value),
        Err(e) => {
            issues.push(ValidationIssue::new(name, format!("undecodable: {e}")));
            None
        }
    }
}

/// Adapter so the codecs' `decode` signatures are uniform for validation.
trait DecodePayload: Sized {
    fn decode_payload(bytes: &[u8]) -> crate::error::Result<Self>;
}
impl DecodePayload for Info {
    fn decode_payload(bytes: &[u8]) -> crate::error::Result<Self> {
        Info::decode(bytes)
    }
}
impl DecodePayload for ChunkSection {
    fn decode_payload(bytes: &[u8]) -> crate::error::Result<Self> {
        ChunkSection::decode(bytes)
    }
}
impl DecodePayload for StyleSection {
    fn decode_payload(bytes: &[u8]) -> crate::error::Result<Self> {
        StyleSection::decode(bytes)
    }
}
impl DecodePayload for ContentSection {
    fn decode_payload(bytes: &[u8]) -> crate::error::Result<Self> {
        ContentSection::decode(bytes)
    }
}
impl DecodePayload for GlyphSection {
    fn decode_payload(bytes: &[u8]) -> crate::error::Result<Self> {
        GlyphSection::decode(bytes)
    }
}
impl DecodePayload for ImageSection {
    fn decode_payload(bytes: &[u8]) -> crate::error::Result<Self> {
        ImageSection::decode(bytes)
    }
}

/// Validate the chunk graph invariants (SPEC.md §2.2). Chunk ids are 1-based
/// dense (id = record index + 1); `0` is the "none" sentinel in every reference
/// field.
///
/// Every id-based access is guarded by the dangling-reference checks above it;
/// the remaining direct indexing is provably in bounds.
#[allow(clippy::indexing_slicing)]
fn validate_chunk_graph(
    chunks: &ChunkSection,
    styles: Option<&StyleSection>,
    content: Option<&ContentSection>,
    issues: &mut Vec<ValidationIssue>,
) {
    let n = chunks.len();
    let style_count = styles.map_or(0, |s| s.styles.len() as u32);
    let content_count = content.map_or(0, |c| c.payloads.len() as u32);

    if n == 0 {
        issues.push(ValidationIssue::new(
            "CHNK",
            "chunk graph is empty (no document root)",
        ));
        return;
    }

    // Dense ordinals and style/content references.
    for (i, chunk) in chunks.chunks.iter().enumerate() {
        let id = (i + 1) as u32;
        if chunk.ordinal != i as u32 {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!("chunk {id} ordinal {} is not dense", chunk.ordinal),
            ));
        }
        if chunk.style_id != 0 && chunk.style_id >= style_count {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!(
                    "chunk {id} references style {} but there are {style_count} styles",
                    chunk.style_id
                ),
            ));
        }
        // `content_index` is a 1-based index into CONT (0 = none), so valid values
        // are 1..=content_count.
        if chunk.content_index != 0 && chunk.content_index > content_count {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!(
                    "chunk {id} references content {} but there are {content_count} payloads",
                    chunk.content_index
                ),
            ));
        }
        if chunk.kind == ChunkKind::Image && chunk.content_index == 0 {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!("image chunk {id} has no content reference"),
            ));
        }
    }

    // Roots: exactly one, kind Document, depth 0, parent 0.
    let roots: Vec<u32> = chunks
        .chunks
        .iter()
        .enumerate()
        .filter(|(_, c)| c.parent_id == 0)
        .map(|(i, _)| (i + 1) as u32)
        .collect();
    if roots.len() != 1 {
        issues.push(ValidationIssue::new(
            "CHNK",
            format!("expected exactly one root, found {}", roots.len()),
        ));
    } else {
        let root = roots[0];
        let root_chunk = &chunks.chunks[(root - 1) as usize];
        if root_chunk.kind != ChunkKind::Document {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!(
                    "root chunk {root} is {}, not document",
                    root_chunk.kind.name()
                ),
            ));
        }
        if root_chunk.depth != 0 {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!("root chunk {root} has depth {}", root_chunk.depth),
            ));
        }
    }

    // Traverse the tree via sibling/child links; every node must be visited exactly
    // once, and sibling chains must be consistent.
    let mut visited = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    for (i, c) in chunks.chunks.iter().enumerate() {
        if c.parent_id == 0 {
            stack.push((i + 1) as u32);
        }
    }
    while let Some(id) = stack.pop() {
        if visited[(id - 1) as usize] {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!("chunk {id} reachable via multiple paths"),
            ));
            continue;
        }
        visited[(id - 1) as usize] = true;
        let chunk = &chunks.chunks[(id - 1) as usize];
        // Depth consistency.
        if chunk.parent_id != 0 {
            let parent_index = (chunk.parent_id - 1) as usize;
            if parent_index >= n {
                issues.push(ValidationIssue::new(
                    "CHNK",
                    format!("chunk {id} has dangling parent {}", chunk.parent_id),
                ));
            } else {
                let parent = &chunks.chunks[parent_index];
                if chunk.depth != parent.depth + 1 {
                    issues.push(ValidationIssue::new(
                        "CHNK",
                        format!(
                            "chunk {id} depth {} != parent depth {} + 1",
                            chunk.depth, parent.depth
                        ),
                    ));
                }
            }
        }
        // Sibling chain: walk first_child via next.
        let mut child = chunk.first_child_id;
        let mut count = 0_u32;
        let mut last_seen = 0_u32;
        while child != 0 {
            if child as usize > n {
                issues.push(ValidationIssue::new(
                    "CHNK",
                    format!("chunk {id} has dangling child {child}"),
                ));
                break;
            }
            count += 1;
            let child_chunk = &chunks.chunks[(child - 1) as usize];
            if child_chunk.parent_id != id {
                issues.push(ValidationIssue::new(
                    "CHNK",
                    format!(
                        "chunk {child} has parent {} but is in the child chain of {id}",
                        child_chunk.parent_id
                    ),
                ));
            }
            if child_chunk.prev_id != last_seen {
                issues.push(ValidationIssue::new(
                    "CHNK",
                    format!(
                        "chunk {child} prev {} inconsistent in chain of {id}",
                        child_chunk.prev_id
                    ),
                ));
            }
            last_seen = child;
            if count > n as u32 {
                issues.push(ValidationIssue::new(
                    "CHNK",
                    format!("sibling chain cycle under chunk {id}"),
                ));
                break;
            }
            let next = chunks.chunks[(child - 1) as usize].next_id;
            if next != 0
                && (next as usize > n || chunks.chunks[(next - 1) as usize].parent_id != id)
            {
                issues.push(ValidationIssue::new(
                    "CHNK",
                    format!("chunk {next} in wrong chain"),
                ));
                break;
            }
            child = next;
        }
        if chunk.last_child_id != last_seen {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!(
                    "chunk {id} last_child {} != chain end {last_seen}",
                    chunk.last_child_id
                ),
            ));
        }
        // Push children (reverse order for document-order processing).
        let mut children: Vec<u32> = Vec::new();
        let mut c = chunk.first_child_id;
        while c != 0 {
            if c as usize > n {
                break; // already reported above
            }
            children.push(c);
            let next = chunks.chunks[(c - 1) as usize].next_id;
            if next == c {
                issues.push(ValidationIssue::new(
                    "CHNK",
                    format!("chunk {c} is its own next sibling"),
                ));
                break;
            }
            c = next;
        }
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    for (i, seen) in visited.iter().enumerate() {
        if !seen {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!("chunk {} unreachable from the root", (i + 1)),
            ));
        }
    }

    // Parent/child kind shape.
    for (i, chunk) in chunks.chunks.iter().enumerate() {
        if chunk.parent_id == 0 {
            continue;
        }
        let parent_index = (chunk.parent_id - 1) as usize;
        if parent_index >= n {
            continue; // already reported as a dangling parent
        }
        let parent = &chunks.chunks[parent_index];
        if !allowed_child(parent.kind, chunk.kind) {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!(
                    "chunk {} ({}) cannot be a child of {} ({})",
                    i + 1,
                    chunk.kind.name(),
                    chunk.parent_id,
                    parent.kind.name()
                ),
            ));
        }
    }

    // Image chunks must reference an image_ref payload.
    if let Some(content) = content {
        for (i, chunk) in chunks.chunks.iter().enumerate() {
            if chunk.kind == ChunkKind::Image && chunk.content_index != 0 {
                let payload = &content.payloads[(chunk.content_index - 1) as usize];
                if payload.kind != PayloadKind::ImageRef {
                    issues.push(ValidationIssue::new(
                        "CHNK",
                        format!(
                            "image chunk {} content {} is not an image_ref",
                            i + 1,
                            chunk.content_index
                        ),
                    ));
                }
            }
        }
    }

    // Extras must be well-formed for their kind.
    for extra in &chunks.extras {
        let valid = match extra.kind {
            ChunkExtraKind::LinkTarget => {
                extra.data.len() >= 2
                    && u16::from_le_bytes([extra.data[0], extra.data[1]]) as usize + 2
                        == extra.data.len()
            }
            ChunkExtraKind::CellSpan => extra.data.len() == 4,
            ChunkExtraKind::ListItemValue => extra.data.len() == 4,
            ChunkExtraKind::ImageAlt => true,
        };
        if !valid {
            issues.push(ValidationIssue::new(
                "CHNK",
                format!(
                    "extra {:?} on chunk {} has malformed data ({} bytes)",
                    extra.kind,
                    extra.chunk_id,
                    extra.data.len()
                ),
            ));
        }
    }
}

/// The parent/child kind shape table (SPEC.md §2.2 invariants).
fn allowed_child(parent: ChunkKind, child: ChunkKind) -> bool {
    use ChunkKind::*;
    match parent {
        Document => matches!(
            child,
            Heading1
                | Heading2
                | Heading3
                | Heading4
                | Heading5
                | Heading6
                | Paragraph
                | Quote
                | List
                | CodeBlock
                | Table
                | Image
                | Hr
                | Caption
        ),
        Heading1 | Heading2 | Heading3 | Heading4 | Heading5 | Heading6 | Paragraph | Quote
        | Caption => child.is_inline(),
        List => child == ListItem,
        ListItem => matches!(
            child,
            List | Paragraph
                | Quote
                | CodeBlock
                | Table
                | Image
                | Hr
                | Heading1
                | Heading2
                | Heading3
                | Heading4
                | Heading5
                | Heading6
        ),
        CodeBlock | Hr | Image => false,
        Table => child == TableRow,
        TableRow => child == TableCell,
        TableCell => matches!(
            child,
            List | Paragraph | Quote | CodeBlock | Table | Image | Hr
        ),
        Run => matches!(child, Run | Link | Br),
        Link => matches!(child, Run | Br),
        Br => false,
    }
}

#[cfg(test)]
mod tests {
    use super::validate_package;
    use crate::codec::chunk::{
        flags, ChunkExtra, ChunkExtraKind, ChunkKind, ChunkRecord, ChunkSection,
    };
    use crate::codec::content::{ContentSection, Payload, PayloadKind};
    use crate::codec::glyph::{Atlas, GlyphRecord, GlyphSection};
    use crate::codec::info::Info;
    use crate::codec::style::{
        PropertyTag, PropertyValue, StyleProperty, StyleRecord, StyleSection,
    };
    use crate::reader::parse;
    use crate::section::SectionKind;
    use crate::table::Compression;
    use crate::writer::PackageBuilder;

    fn info(chunk_count: u32) -> Info {
        Info {
            format_version: 1,
            generator: "test".to_string(),
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
        }
    }

    fn styles() -> StyleSection {
        StyleSection {
            styles: vec![StyleRecord {
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
                ],
            }],
        }
    }

    fn content() -> ContentSection {
        ContentSection {
            payloads: vec![Payload {
                kind: PayloadKind::TextUtf8,
                data: b"Hello".to_vec(),
            }],
        }
    }

    fn doc_chunk(first_child: u32) -> ChunkRecord {
        ChunkRecord {
            kind: ChunkKind::Document,
            flags: flags::STRUCTURAL,
            style_id: 0,
            parent_id: 0,
            prev_id: 0,
            next_id: 0,
            first_child_id: first_child,
            last_child_id: first_child,
            content_index: 0,
            ordinal: 0,
            depth: 0,
        }
    }

    fn para_chunk(id: u32, parent: u32, prev: u32, next: u32) -> ChunkRecord {
        ChunkRecord {
            kind: ChunkKind::Paragraph,
            flags: 0,
            style_id: 0,
            parent_id: parent,
            prev_id: prev,
            next_id: next,
            first_child_id: 0,
            last_child_id: 0,
            content_index: 1,
            ordinal: id - 1, // ordinals are 0-based dense; ids are 1-based dense
            depth: 1,
        }
    }

    fn build_package(chunks: ChunkSection) -> Vec<u8> {
        let n = chunks.len() as u32;
        build_package_declared(chunks, n)
    }

    fn build_package_declared(chunks: ChunkSection, declared_chunk_count: u32) -> Vec<u8> {
        let mut builder = PackageBuilder::new();
        builder
            .add(
                SectionKind::Info,
                info(declared_chunk_count).encode(),
                Compression::Zlib,
            )
            .expect("add");
        builder
            .add(SectionKind::Chunk, chunks.encode(), Compression::Zlib)
            .expect("add");
        builder
            .add(SectionKind::Style, styles().encode(), Compression::Zlib)
            .expect("add");
        builder
            .add(SectionKind::Content, content().encode(), Compression::Zlib)
            .expect("add");
        // A minimal atlas so style/content font references resolve.
        let atlas = GlyphSection {
            atlases: vec![Atlas {
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
                family: "Test Sans".to_string(),
                weight: 400,
                italic: false,
                page_width: 8,
                page_height: 8,
                glyphs: vec![GlyphRecord {
                    codepoint: 'A' as u32,
                    advance: 0.6,
                    bearing_x: 0.0,
                    bearing_y: 0.7,
                    box_x: 1,
                    box_y: 1,
                    box_w: 4,
                    box_h: 4,
                    page_index: 0,
                    flags: 0,
                }],
                kerning: vec![],
                pages: vec![vec![0_u8; 8 * 8 * 4]],
            }],
        };
        builder
            .add(SectionKind::Glyph, atlas.encode(), Compression::None)
            .expect("add glyph");
        builder.build().expect("build")
    }

    #[test]
    fn valid_document() {
        let chunks = ChunkSection {
            chunks: vec![doc_chunk(2), para_chunk(2, 1, 0, 0)],
            extras: vec![],
        };
        let pkg = parse(&build_package(chunks)).expect("parse");
        let issues = validate_package(&pkg);
        assert!(issues.is_empty(), "issues: {issues:?}");
    }

    #[test]
    fn missing_root() {
        // Two chunks, each with a parent: no root exists.
        let chunks = ChunkSection {
            chunks: vec![para_chunk(1, 2, 0, 0), para_chunk(2, 1, 0, 0)],
            extras: vec![],
        };
        let pkg = parse(&build_package(chunks)).expect("parse");
        let issues = validate_package(&pkg);
        assert!(issues.iter().any(|i| i.message.contains("root")));
    }

    #[test]
    fn unreachable_chunk() {
        // Document with no children; a paragraph with a dangling parent.
        let chunks = ChunkSection {
            chunks: vec![doc_chunk(0), para_chunk(2, 3, 0, 0)],
            extras: vec![],
        };
        let pkg = parse(&build_package(chunks)).expect("parse");
        let issues = validate_package(&pkg);
        assert!(issues.iter().any(|i| i.message.contains("unreachable")));
    }

    #[test]
    fn sibling_chain_inconsistency() {
        // doc -> [p1, p2] but p2.prev points to itself.
        let chunks = ChunkSection {
            chunks: vec![
                ChunkRecord {
                    first_child_id: 2,
                    last_child_id: 3,
                    ..doc_chunk(0)
                },
                para_chunk(2, 1, 0, 3),
                ChunkRecord {
                    prev_id: 3,
                    next_id: 0,
                    ..para_chunk(3, 1, 0, 0)
                },
            ],
            extras: vec![],
        };
        let pkg = parse(&build_package(chunks)).expect("parse");
        let issues = validate_package(&pkg);
        assert!(issues.iter().any(|i| i.message.contains("prev")));
    }

    #[test]
    fn bad_child_kind() {
        // A document whose child is a run: not allowed at the top level.
        let run = ChunkRecord {
            kind: ChunkKind::Run,
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
        };
        let chunks = ChunkSection {
            chunks: vec![doc_chunk(2), run],
            extras: vec![],
        };
        let pkg = parse(&build_package(chunks)).expect("parse");
        let issues = validate_package(&pkg);
        assert!(issues
            .iter()
            .any(|i| i.message.contains("cannot be a child")));
    }

    #[test]
    fn depth_mismatch() {
        let chunks = ChunkSection {
            chunks: vec![
                doc_chunk(2),
                ChunkRecord {
                    depth: 5,
                    ..para_chunk(2, 1, 0, 0)
                },
            ],
            extras: vec![],
        };
        let pkg = parse(&build_package(chunks)).expect("parse");
        let issues = validate_package(&pkg);
        assert!(issues.iter().any(|i| i.message.contains("depth")));
    }

    #[test]
    fn dangling_style_reference() {
        // style_id 5 with only 1 style record.
        let chunks = ChunkSection {
            chunks: vec![
                doc_chunk(2),
                ChunkRecord {
                    style_id: 5,
                    ..para_chunk(2, 1, 0, 0)
                },
            ],
            extras: vec![],
        };
        let pkg = parse(&build_package(chunks)).expect("parse");
        let issues = validate_package(&pkg);
        assert!(issues.iter().any(|i| i.message.contains("style")));
    }

    #[test]
    fn info_count_mismatch() {
        // INFO declares 2 chunks; the section contains 1.
        let chunks = ChunkSection {
            chunks: vec![doc_chunk(0)],
            extras: vec![],
        };
        let pkg = parse(&build_package_declared(chunks, 2)).expect("parse");
        let issues = validate_package(&pkg);
        assert!(issues.iter().any(|i| i.message.contains("chunk_count")));
    }

    #[test]
    fn malformed_extra() {
        let chunks = ChunkSection {
            chunks: vec![doc_chunk(2), para_chunk(2, 1, 0, 0)],
            extras: vec![ChunkExtra {
                chunk_id: 2,
                kind: ChunkExtraKind::LinkTarget,
                data: vec![0, 5, 1, 2], // url_len=5 but only 2 url bytes
            }],
        };
        let pkg = parse(&build_package(chunks)).expect("parse");
        let issues = validate_package(&pkg);
        assert!(issues.iter().any(|i| i.message.contains("malformed")));
    }
}
