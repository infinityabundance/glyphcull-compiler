//! The partition transform: Semantic Graph → Chunk Graph.
//!
//! Semantic nodes become chunks with flat resolved styles (SPEC.md CHNK), content
//! payloads (SPEC.md CONT), and tree links. Inline content becomes run chunks;
//! transparent containers are spliced; list markers, cell spans, link targets,
//! and image alt text become extras. Ordinals and ids are assigned in document
//! order (ids 1-based, ordinals 0-based), depths follow the chunk structure.
//!
//! Style resolution runs inline with the walk: every semantic node's flat style
//! is computed from its matched declarations and its parent's computed style, so
//! the partition and the cascade agree by construction. The default stylesheet
//! owns the browser-default inline treatments (em → italic, strong → 700,
//! a → underline), so author overrides cascade correctly.
//!
//! Structural decisions (documented in DESIGN.md):
//! - Ordered list item markers are baked as `list_item_value` extras (the format
//!   has no list-start field; the runtime must not compute markers).
//! - The HTML `hidden` attribute sets the chunk `hidden` flag (semantic culling).
//!
//! The direct indexing in this module is on the builder's own `Vec` arena with
//! ids assigned monotonically (`id = index + 1`), so it is provably in bounds —
//! the documented exception to the workspace indexing policy.

#![allow(clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};

use glyphcull_format::codec::chunk::{
    flags, ChunkExtra, ChunkExtraKind, ChunkKind, ChunkRecord, ChunkSection,
};
use glyphcull_format::codec::content::{ContentSection, Payload, PayloadKind};
use glyphcull_semantic::css::CssRule;
use glyphcull_semantic::model::{SemanticKind, SemanticNode};

use crate::styles::{FaceKey, ResolvedStyle};

/// The marker glyph charset needed for a list style (SPEC.md §2.5 — runtimes
/// never synthesize typefaces, so every marker codepoint must be in the
/// atlas). Ordered lists need the full counter alphabet plus the separator.
fn marker_charset(list_style: u8) -> &'static str {
    match list_style {
        1 => "\u{2022}",                    // disc
        2 => "\u{25e6}",                    // circle
        3 => "\u{25aa}",                    // square
        4 => "0123456789.",                 // decimal
        5 => "abcdefghijklmnopqrstuvwxyz.", // lower-alpha
        6 => "ABCDEFGHIJKLMNOPQRSTUVWXYZ.", // upper-alpha
        7 => "ivxlcdm.",                    // lower-roman
        8 => "IVXLCDM.",                    // upper-roman
        _ => "",
    }
}

/// An image reference in document order; the pipeline decodes images in this
/// exact order so IMGS ids match the image_ref payload indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSource {
    /// The source path/URL as written in the document.
    pub src: String,
    /// Alt text, if any.
    pub alt: Option<String>,
}

/// The compiled chunk model: everything the runtime needs except glyph atlases
/// (built by the pipeline from `used_codepoints`).
#[derive(Debug, Clone)]
pub struct ChunkModel {
    /// The chunk graph (SPEC.md CHNK).
    pub chunk_section: ChunkSection,
    /// Interned resolved styles (id = index; style 0 is the document default).
    pub resolved_styles: Vec<ResolvedStyle>,
    /// Content payloads (SPEC.md CONT).
    pub content_section: ContentSection,
    /// Image references in document order.
    pub images: Vec<ImageSource>,
    /// Codepoints used per face (drives glyph atlas generation).
    pub used_codepoints: BTreeMap<FaceKey, BTreeSet<u32>>,
}

/// Partition a semantic tree into the chunk model.
pub fn build_chunk_model(
    root: &SemanticNode,
    rules: &[CssRule],
    user_stylesheets: &[glyphcull_semantic::css::Stylesheet],
) -> ChunkModel {
    let rules = if rules.is_empty() {
        crate::styles::ruleset(user_stylesheets)
    } else {
        rules.to_vec()
    };
    let mut builder = Builder {
        chunks: Vec::new(),
        extras: Vec::new(),
        content: Vec::new(),
        payload_ids: BTreeMap::new(),
        images: Vec::new(),
        used: BTreeMap::new(),
        styles: vec![ResolvedStyle::default()], // style 0: document default
        rules,
        stack: Vec::new(),
    };
    // The root itself is the document chunk (SPEC.md CHNK: chunk 1 is always
    // the document). Emitting the node rather than only its children makes the
    // graph self-contained: parent links, depth, and ordinals all follow from
    // the same walk.
    let default_style = ResolvedStyle::default();
    builder.emit_node(root, &default_style, &[]);
    let chunk_section = ChunkSection {
        chunks: builder.chunks,
        extras: builder.extras,
    };
    ChunkModel {
        chunk_section,
        resolved_styles: builder.styles,
        content_section: ContentSection {
            payloads: builder.content,
        },
        images: builder.images,
        used_codepoints: builder.used,
    }
}

/// The chunk-building state machine. `stack` holds open chunk ids (parent
/// chain); `ancestors` holds the semantic ancestor chain (nearest first) for
/// descendant-selector matching; sibling links are maintained on push.
struct Builder {
    chunks: Vec<ChunkRecord>,
    extras: Vec<ChunkExtra>,
    content: Vec<Payload>,
    payload_ids: BTreeMap<Vec<u8>, u32>,
    images: Vec<ImageSource>,
    used: BTreeMap<FaceKey, BTreeSet<u32>>,
    styles: Vec<ResolvedStyle>,
    rules: Vec<CssRule>,
    stack: Vec<u32>,
}

impl Builder {
    /// The ancestor chain for a node: `[node, parent, grandparent, ...]`.
    fn chain<'a>(node: &'a SemanticNode, ancestors: &[&'a SemanticNode]) -> Vec<&'a SemanticNode> {
        let mut chain = Vec::with_capacity(ancestors.len() + 1);
        chain.push(node);
        chain.extend_from_slice(ancestors);
        chain
    }

    /// Emit chunks for a node's children (splicing transparent containers).
    fn emit_children(
        &mut self,
        node: &SemanticNode,
        parent_style: &ResolvedStyle,
        ancestors: &[&SemanticNode],
    ) {
        let chain = Self::chain(node, ancestors);
        for child in &node.children {
            self.emit_node(child, parent_style, &chain);
        }
    }

    /// Emit a node's children with block semantics (SPEC.md §2.2): block
    /// children emit directly; runs of inline children are wrapped in an
    /// implicit `paragraph` chunk because block containers (list items, table
    /// cells) hold blocks — runs nest only under paragraphs/headings/captions.
    fn emit_block_children(
        &mut self,
        node: &SemanticNode,
        parent_style: &ResolvedStyle,
        ancestors: &[&SemanticNode],
    ) {
        let chain = Self::chain(node, ancestors);
        let mut inline: Vec<&SemanticNode> = Vec::new();
        for child in &node.children {
            if child.kind.is_block() {
                if !inline.is_empty() {
                    self.emit_implicit_paragraph(&inline, parent_style, &chain);
                    inline.clear();
                }
                self.emit_node(child, parent_style, &chain);
            } else {
                inline.push(child);
            }
        }
        if !inline.is_empty() {
            self.emit_implicit_paragraph(&inline, parent_style, &chain);
        }
    }

    /// Emit an implicit paragraph chunk wrapping inline nodes (the block
    /// container's style; no semantic node exists for the wrapper).
    fn emit_implicit_paragraph(
        &mut self,
        inline: &[&SemanticNode],
        parent_style: &ResolvedStyle,
        ancestors: &[&SemanticNode],
    ) {
        // The wrapper carries the container's resolved style (the container's
        // own cascade); the inline nodes resolve their own styles underneath.
        let style_id = self.intern_style((*parent_style).clone());
        self.push(ChunkKind::Paragraph, inline[0], style_id, 0);
        for child in inline {
            self.emit_inline_node(parent_style, child, ancestors);
        }
        self.pop();
    }

    /// Emit chunks for one semantic node.
    fn emit_node(
        &mut self,
        node: &SemanticNode,
        parent_style: &ResolvedStyle,
        ancestors: &[&SemanticNode],
    ) {
        let style = crate::styles::resolve_node(node, ancestors, parent_style, &self.rules);
        let style_id = self.intern_style(style.clone());

        let chain = Self::chain(node, ancestors);
        match node.kind {
            SemanticKind::Document => {
                self.push(ChunkKind::Document, node, style_id, 0);
                self.emit_children(node, &style, &chain);
                self.pop();
            }
            SemanticKind::Heading(level) => {
                let kind = match level {
                    1 => ChunkKind::Heading1,
                    2 => ChunkKind::Heading2,
                    3 => ChunkKind::Heading3,
                    4 => ChunkKind::Heading4,
                    5 => ChunkKind::Heading5,
                    _ => ChunkKind::Heading6,
                };
                self.push(kind, node, style_id, 0);
                self.emit_inline_runs(&node.children, &style, &chain);
                self.pop();
            }
            SemanticKind::Paragraph | SemanticKind::Caption => {
                let kind = if node.kind == SemanticKind::Caption {
                    ChunkKind::Caption
                } else {
                    ChunkKind::Paragraph
                };
                self.push(kind, node, style_id, 0);
                self.emit_inline_runs(&node.children, &style, &chain);
                self.pop();
            }
            SemanticKind::Quote => {
                // A quote is a renderable block container: its block children
                // (paragraphs, lists, code) are emitted recursively.
                self.push(ChunkKind::Quote, node, style_id, 0);
                self.emit_children(node, &style, &chain);
                self.pop();
            }
            SemanticKind::CodeBlock => {
                // The verbatim code text is content (SPEC.md §2.4): code chunks
                // carry their text payload directly (no inline children).
                let face = FaceKey {
                    family: style.font_family.clone(),
                    weight: style.font_weight,
                    italic: style.italic,
                };
                let payload_index = if node.text.is_empty() {
                    0
                } else {
                    self.text_payload(&node.text, face)
                };
                self.push(ChunkKind::CodeBlock, node, style_id, payload_index);
                self.pop();
            }
            SemanticKind::OrderedList | SemanticKind::UnorderedList => {
                self.push(ChunkKind::List, node, style_id, 0);
                let ordered = node.kind == SemanticKind::OrderedList;
                let mut value = node.list_start.unwrap_or(1);
                for child in &node.children {
                    if child.kind == SemanticKind::ListItem {
                        self.emit_list_item(child, &style, ordered, value, &chain);
                        value = value.saturating_add(1);
                    } else {
                        self.emit_node(child, &style, &chain);
                    }
                }
                self.pop();
            }
            SemanticKind::ListItem => {
                // A bare item (not inside a list) emits with auto markers.
                self.emit_list_item(node, &style, false, 0, &chain);
            }
            SemanticKind::Table => {
                self.push(ChunkKind::Table, node, style_id, 0);
                self.emit_children(node, &style, &chain);
                self.pop();
            }
            SemanticKind::TableRow => {
                self.push(ChunkKind::TableRow, node, style_id, 0);
                self.emit_children(node, &style, &chain);
                self.pop();
            }
            SemanticKind::TableCell => {
                self.push(ChunkKind::TableCell, node, style_id, 0);
                if node.colspan > 1 || node.rowspan > 1 {
                    let mut data = Vec::with_capacity(4);
                    data.extend_from_slice(&(node.colspan as u16).to_le_bytes());
                    data.extend_from_slice(&(node.rowspan as u16).to_le_bytes());
                    let id = self.current_id();
                    self.extras.push(ChunkExtra {
                        chunk_id: id,
                        kind: ChunkExtraKind::CellSpan,
                        data,
                    });
                }
                // Cell content is block-level: inline content wraps in an
                // implicit paragraph (SPEC.md §2.2).
                self.emit_block_children(node, &style, &chain);
                self.pop();
            }
            SemanticKind::Image => {
                let payload_index = self.image_payload(node);
                self.push(ChunkKind::Image, node, style_id, payload_index);
                let id = self.current_id();
                if let Some(alt) = &node.alt {
                    self.extras.push(ChunkExtra {
                        chunk_id: id,
                        kind: ChunkExtraKind::ImageAlt,
                        data: alt.clone().into_bytes(),
                    });
                }
                // An image that was inside a link carries the hoisted target
                // (glyphcull-semantic hoists image links to block level).
                if let Some(href) = &node.href {
                    let mut data = Vec::with_capacity(href.len() + 2);
                    data.extend_from_slice(&(href.len() as u16).to_le_bytes());
                    data.extend_from_slice(href.as_bytes());
                    self.extras.push(ChunkExtra {
                        chunk_id: id,
                        kind: ChunkExtraKind::LinkTarget,
                        data,
                    });
                }
                self.emit_children(node, &style, &chain);
                self.pop();
            }
            SemanticKind::Rule => {
                self.push(ChunkKind::Hr, node, style_id, 0);
                self.pop();
            }
            SemanticKind::Transparent => {
                // Transparent nodes carry text or children (the unified inline
                // model). At block level (list items, table cells, quotes) a
                // text-bearing transparent node is inline content: it wraps in
                // an implicit paragraph (SPEC.md §2.2 — block containers hold
                // blocks). Containers recurse with block semantics.
                if !node.text.is_empty() {
                    let chain = Self::chain(node, ancestors);
                    self.push(ChunkKind::Paragraph, node, style_id, 0);
                    self.emit_inline_node(&style, node, &chain);
                    self.pop();
                } else {
                    self.emit_block_children(node, &style, ancestors);
                }
            }
            _ => {
                // A stray inline node at block level (emphasis, strong, code,
                // soft break, …): wrap it in an implicit paragraph so the chunk
                // graph obeys the block-container shape (SPEC.md §2.2).
                let chain = Self::chain(node, ancestors);
                self.push(ChunkKind::Paragraph, node, style_id, 0);
                self.emit_inline_node(&style, node, &chain);
                self.pop();
            }
        }
    }

    /// Emit a list item chunk (with its marker value when ordered).
    fn emit_list_item(
        &mut self,
        node: &SemanticNode,
        parent_style: &ResolvedStyle,
        ordered: bool,
        value: u64,
        ancestors: &[&SemanticNode],
    ) {
        let style = crate::styles::resolve_node(node, ancestors, parent_style, &self.rules);
        let style_id = self.intern_style(style.clone());
        let chain = Self::chain(node, ancestors);
        // Marker glyphs must be in the atlas (runtimes never synthesize
        // typefaces): register the charset of the item's list style against
        // the item's face so the atlas covers the markers.
        let face = FaceKey {
            family: style.font_family.clone(),
            weight: style.font_weight,
            italic: style.italic,
        };
        let set = self.used.entry(face).or_default();
        for c in marker_charset(style.list_style).chars() {
            set.insert(u32::from(c));
        }
        self.push(ChunkKind::ListItem, node, style_id, 0);
        if ordered {
            let id = self.current_id();
            self.extras.push(ChunkExtra {
                chunk_id: id,
                kind: ChunkExtraKind::ListItemValue,
                data: (value as u32).to_le_bytes().to_vec(),
            });
        }
        // Item content is block-level: inline text wraps in an implicit
        // paragraph (SPEC.md §2.2 — runs nest only under block chunks).
        self.emit_block_children(node, &style, &chain);
        self.pop();
    }

    /// Emit the inline content of a block as run chunks. `ancestors` is the
    /// block's ancestor chain including the block itself, so each inline node
    /// resolves its own cascade (inline classes, ids, and styles apply).
    fn emit_inline_runs(
        &mut self,
        children: &[SemanticNode],
        block_style: &ResolvedStyle,
        ancestors: &[&SemanticNode],
    ) {
        for child in children {
            self.emit_inline_node(block_style, child, ancestors);
        }
    }

    /// Emit one inline node as run chunks, resolving its own computed style from
    /// the cascade (so inline classes, ids, and inline styles apply; the default
    /// stylesheet owns the em/strong/a/code defaults). Returns the ids of the
    /// linkable chunks (run, image) emitted beneath this node.
    fn emit_inline_node(
        &mut self,
        parent_style: &ResolvedStyle,
        node: &SemanticNode,
        ancestors: &[&SemanticNode],
    ) -> Vec<u32> {
        let style = crate::styles::resolve_node(node, ancestors, parent_style, &self.rules);
        let chain = Self::chain(node, ancestors);
        match node.kind {
            SemanticKind::Transparent => {
                if !node.text.is_empty() {
                    vec![self.emit_run(&style, node, node.text.clone())]
                } else {
                    let mut ids = Vec::new();
                    for child in &node.children {
                        ids.extend(self.emit_inline_node(&style, child, &chain));
                    }
                    ids
                }
            }
            SemanticKind::Link => {
                let href = node.href.clone();
                let mut ids = Vec::new();
                if !node.children.is_empty() {
                    for child in &node.children {
                        ids.extend(self.emit_inline_node(&style, child, &chain));
                    }
                } else if !node.text.is_empty() {
                    ids.push(self.emit_run(&style, node, node.text.clone()));
                }
                // Extras reference a single chunk id, so a multi-run link carries
                // one link_target extra per run chunk (SPEC.md CHNK).
                if let Some(href) = href {
                    let mut data = Vec::with_capacity(href.len() + 2);
                    data.extend_from_slice(&(href.len() as u16).to_le_bytes());
                    data.extend_from_slice(href.as_bytes());
                    for id in &ids {
                        self.extras.push(ChunkExtra {
                            chunk_id: *id,
                            kind: ChunkExtraKind::LinkTarget,
                            data: data.clone(),
                        });
                    }
                }
                ids
            }
            SemanticKind::Image => {
                let payload_index = self.image_payload(node);
                let style_id = self.intern_style(style.clone());
                self.push(ChunkKind::Image, node, style_id, payload_index);
                let id = self.current_id();
                if let Some(alt) = &node.alt {
                    self.extras.push(ChunkExtra {
                        chunk_id: id,
                        kind: ChunkExtraKind::ImageAlt,
                        data: alt.clone().into_bytes(),
                    });
                }
                if let Some(href) = &node.href {
                    let mut data = Vec::with_capacity(href.len() + 2);
                    data.extend_from_slice(&(href.len() as u16).to_le_bytes());
                    data.extend_from_slice(href.as_bytes());
                    self.extras.push(ChunkExtra {
                        chunk_id: id,
                        kind: ChunkExtraKind::LinkTarget,
                        data,
                    });
                }
                for child in &node.children {
                    self.emit_inline_node(&style, child, &chain);
                }
                self.pop();
                vec![id]
            }
            SemanticKind::HardBreak => {
                self.push(ChunkKind::Br, node, 0, 0);
                self.pop();
                Vec::new()
            }
            _ => {
                if !node.children.is_empty() {
                    let mut ids = Vec::new();
                    for child in &node.children {
                        ids.extend(self.emit_inline_node(&style, child, &chain));
                    }
                    ids
                } else if !node.text.is_empty() {
                    vec![self.emit_run(&style, node, node.text.clone())]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Emit one run chunk with text and its resolved style; returns the run's
    /// chunk id.
    fn emit_run(&mut self, style: &ResolvedStyle, node: &SemanticNode, text: String) -> u32 {
        let face = FaceKey {
            family: style.font_family.clone(),
            weight: style.font_weight,
            italic: style.italic,
        };
        let style_id = self.intern_style(style.clone());
        let payload_index = self.text_payload(&text, face);
        self.push(ChunkKind::Run, node, style_id, payload_index);
        let id = self.current_id();
        self.pop();
        id
    }

    /// Intern a resolved style; returns its id.
    fn intern_style(&mut self, style: ResolvedStyle) -> u32 {
        if let Some(pos) = self.styles.iter().position(|s| *s == style) {
            return pos as u32;
        }
        self.styles.push(style);
        (self.styles.len() - 1) as u32
    }

    /// Intern a text payload (deduplicated); returns the 1-based content index.
    /// Records the text's codepoints against `face`.
    fn text_payload(&mut self, text: &str, face: FaceKey) -> u32 {
        let set = self.used.entry(face).or_default();
        for c in text.chars() {
            set.insert(c as u32);
        }
        let bytes = text.as_bytes().to_vec();
        if let Some(&id) = self.payload_ids.get(&bytes) {
            return id + 1;
        }
        let id = self.content.len() as u32;
        self.payload_ids.insert(bytes.clone(), id);
        self.content.push(Payload {
            kind: PayloadKind::TextUtf8,
            data: bytes,
        });
        id + 1
    }

    /// Intern an image reference payload; returns the 1-based content index.
    fn image_payload(&mut self, node: &SemanticNode) -> u32 {
        let id = self.images.len() as u32;
        self.images.push(ImageSource {
            src: node.image_src.clone().unwrap_or_default(),
            alt: node.alt.clone(),
        });
        let data = id.to_le_bytes().to_vec();
        self.content.push(Payload {
            kind: PayloadKind::ImageRef,
            data,
        });
        self.content.len() as u32
    }

    fn push(&mut self, kind: ChunkKind, node: &SemanticNode, style_id: u32, content: u32) {
        let id = (self.chunks.len() + 1) as u32;
        let parent = self.stack.last().copied().unwrap_or(0);
        let mut chunk_flags = if kind.is_structural() {
            flags::STRUCTURAL
        } else {
            0
        };
        if node.hints.hidden {
            chunk_flags |= flags::HIDDEN;
        }
        let ordinal = self.chunks.len() as u32;
        let depth = self.stack.len() as u32;

        self.chunks.push(ChunkRecord {
            kind,
            flags: chunk_flags,
            style_id,
            parent_id: parent,
            prev_id: 0,
            next_id: 0,
            first_child_id: 0,
            last_child_id: 0,
            content_index: content,
            ordinal,
            depth,
        });

        // Link into the parent's child chain.
        if parent != 0 {
            let parent_index = (parent - 1) as usize;
            let parent_last = self.chunks[parent_index].last_child_id;
            if parent_last == 0 {
                self.chunks[parent_index].first_child_id = id;
            } else {
                let last_index = (parent_last - 1) as usize;
                self.chunks[last_index].next_id = id;
                self.chunks[(id - 1) as usize].prev_id = parent_last;
            }
            self.chunks[parent_index].last_child_id = id;
        }

        self.stack.push(id);
    }

    fn pop(&mut self) {
        self.stack.pop();
    }

    fn current_id(&self) -> u32 {
        self.stack.last().copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glyphcull_format::codec::info::Info;
    use glyphcull_format::codec::style::{
        PropertyTag, PropertyValue, StyleProperty, StyleRecord, StyleSection,
    };
    use glyphcull_format::reader::parse;
    use glyphcull_format::section::SectionKind;
    use glyphcull_format::table::Compression;
    use glyphcull_format::validate::validate_package;
    use glyphcull_format::writer::PackageBuilder;
    use glyphcull_semantic::parse_markdown;

    fn model(md: &str) -> ChunkModel {
        let root = parse_markdown(md).expect("parse");
        build_chunk_model(&root, &[], &[])
    }

    /// Convert the resolved styles into a STYL section (font_id left at the
    /// default 0; the validation fixture provides a matching atlas).
    fn style_section(styles: &[ResolvedStyle]) -> StyleSection {
        StyleSection {
            styles: styles
                .iter()
                .enumerate()
                .map(|(id, s)| StyleRecord {
                    id: id as u32,
                    properties: vec![
                        StyleProperty {
                            tag: PropertyTag::FontSizePx,
                            value: PropertyValue::F32(s.font_size_px),
                        },
                        StyleProperty {
                            tag: PropertyTag::LineHeight,
                            value: PropertyValue::F32(s.line_height),
                        },
                        StyleProperty {
                            tag: PropertyTag::FontWeight,
                            value: PropertyValue::U16(s.font_weight),
                        },
                        StyleProperty {
                            tag: PropertyTag::Italic,
                            value: PropertyValue::U8(u8::from(s.italic)),
                        },
                        StyleProperty {
                            tag: PropertyTag::Color,
                            value: PropertyValue::U32(s.color),
                        },
                        StyleProperty {
                            tag: PropertyTag::MarginTop,
                            value: PropertyValue::F32(s.margin_top),
                        },
                        StyleProperty {
                            tag: PropertyTag::MarginBottom,
                            value: PropertyValue::F32(s.margin_bottom),
                        },
                        StyleProperty {
                            tag: PropertyTag::TextAlign,
                            value: PropertyValue::U8(s.text_align),
                        },
                        StyleProperty {
                            tag: PropertyTag::WhiteSpace,
                            value: PropertyValue::U8(s.white_space),
                        },
                    ],
                })
                .collect(),
        }
    }

    /// Build a full package from a model and assert the format validator accepts it.
    fn assert_package_valid(m: &ChunkModel) {
        let info = Info {
            format_version: 1,
            generator: "graph-test".to_string(),
            generator_version: "0.0.0".to_string(),
            source_digest: "ab".repeat(32),
            document_id: "cd".repeat(16),
            title: None,
            lang: None,
            chunk_count: m.chunk_section.len() as u32,
            style_count: m.resolved_styles.len() as u32,
            content_count: m.content_section.payloads.len() as u32,
            atlas_count: 1,
            image_count: m.images.len() as u32,
        };
        // A minimal atlas (one glyph) so style/content references resolve.
        let atlas = glyphcull_format::codec::glyph::GlyphSection {
            atlases: vec![glyphcull_format::codec::glyph::Atlas {
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
                glyphs: vec![glyphcull_format::codec::glyph::GlyphRecord {
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
        let mut builder = PackageBuilder::new().with_seal(true);
        builder
            .add(SectionKind::Info, info.encode(), Compression::Zlib)
            .expect("info");
        builder
            .add(
                SectionKind::Chunk,
                m.chunk_section.encode(),
                Compression::Zlib,
            )
            .expect("chunk");
        builder
            .add(
                SectionKind::Style,
                style_section(&m.resolved_styles).encode(),
                Compression::Zlib,
            )
            .expect("style");
        builder
            .add(
                SectionKind::Content,
                m.content_section.encode(),
                Compression::Zlib,
            )
            .expect("content");
        builder
            .add(SectionKind::Glyph, atlas.encode(), Compression::None)
            .expect("glyph");
        if !m.images.is_empty() {
            builder
                .add(
                    SectionKind::Images,
                    glyphcull_format::codec::image::ImageSection {
                        images: m
                            .images
                            .iter()
                            .map(|_| glyphcull_format::codec::image::Image {
                                width: 1,
                                height: 1,
                                format: glyphcull_format::codec::image::ImageFormat::Rgba8,
                                data: vec![0, 0, 0, 255],
                            })
                            .collect(),
                    }
                    .encode(),
                    Compression::None,
                )
                .expect("images");
        }
        let bytes = builder.build().expect("build");
        let pkg = parse(&bytes).expect("parse");
        let issues = validate_package(&pkg);
        assert!(issues.is_empty(), "validation issues: {issues:?}");
    }

    #[test]
    fn basic_structure() {
        let m = model("# Title\n\nHello *world*!\n");
        let chunks = &m.chunk_section.chunks;
        // document, heading, run(Title), paragraph, run(Hello), run(world), run(!)
        assert_eq!(chunks.len(), 7);
        assert_eq!(chunks[0].kind, ChunkKind::Document);
        assert_eq!(chunks[0].depth, 0);
        assert_eq!(chunks[0].flags, flags::STRUCTURAL);
        // Document children: heading (2) and paragraph (4).
        assert_eq!(chunks[0].first_child_id, 2);
        assert_eq!(chunks[0].last_child_id, 4);
        assert_eq!(chunks[1].kind, ChunkKind::Heading1);
        assert_eq!(chunks[1].depth, 1);
        assert_eq!(chunks[1].first_child_id, 3);
        assert_eq!(chunks[2].kind, ChunkKind::Run);
        assert_eq!(chunks[2].depth, 2);
        // Ordinals are dense and match creation order.
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.ordinal, i as u32);
        }
        // Text payloads contain the expected text.
        let payloads = &m.content_section.payloads;
        let text: Vec<&str> = payloads
            .iter()
            .filter(|p| p.kind == PayloadKind::TextUtf8)
            .map(|p| std::str::from_utf8(&p.data).expect("utf8"))
            .collect();
        assert!(text.contains(&"Title"));
        assert!(text.contains(&"world"));
        assert_package_valid(&m);
    }

    #[test]
    fn inline_styles() {
        let m = model("*em* **strong** `code`\n");
        let styles = &m.resolved_styles;
        let chunk_styles: Vec<u32> = m
            .chunk_section
            .chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Run)
            .map(|c| c.style_id)
            .collect();
        assert!(chunk_styles.iter().any(|&id| styles[id as usize].italic));
        assert!(chunk_styles
            .iter()
            .any(|&id| styles[id as usize].font_weight == 700));
        assert!(chunk_styles.iter().any(|&id| styles[id as usize].code));
        assert_package_valid(&m);
    }

    #[test]
    fn ordered_list_markers() {
        let m = model("3. third\n4. fourth\n");
        let item_values: Vec<u32> = m
            .chunk_section
            .extras
            .iter()
            .filter(|e| e.kind == ChunkExtraKind::ListItemValue)
            .map(|e| u32::from_le_bytes(e.data[..4].try_into().expect("len")))
            .collect();
        assert_eq!(item_values, vec![3, 4]);
        assert_package_valid(&m);
    }

    #[test]
    fn list_item_text_is_content() {
        // Regression: tight-list items (inline text directly under the item)
        // must emit content — their text was dropped. Per SPEC.md §2.2 the
        // item is a block container, so the text wraps in an implicit
        // paragraph with run children.
        let m = model("# T\n\n- one\n- two\n");
        let chunks = &m.chunk_section.chunks;
        let list = chunks
            .iter()
            .find(|c| c.kind == ChunkKind::List)
            .expect("list");
        let items: Vec<&ChunkRecord> = chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::ListItem)
            .collect();
        assert_eq!(items.len(), 2);
        for item in &items {
            assert!(item.first_child_id != 0, "list item has a paragraph child");
            let para = chunks
                .iter()
                .find(|c| c.ordinal + 1 == item.first_child_id)
                .expect("paragraph child");
            assert_eq!(para.kind, ChunkKind::Paragraph);
            assert!(para.first_child_id != 0, "paragraph has a run child");
            let run = chunks
                .iter()
                .find(|c| c.ordinal + 1 == para.first_child_id)
                .expect("run child");
            assert_eq!(run.kind, ChunkKind::Run);
            assert!(run.content_index != 0, "run has a text payload");
        }
        assert_eq!(list.first_child_id, items[0].ordinal + 1);
        assert_eq!(list.last_child_id, items[1].ordinal + 1);
        let text: Vec<&str> = m
            .content_section
            .payloads
            .iter()
            .filter(|p| p.kind == PayloadKind::TextUtf8)
            .map(|p| std::str::from_utf8(&p.data).expect("utf8"))
            .collect();
        assert!(text.contains(&"one"));
        assert!(text.contains(&"two"));
        assert_package_valid(&m);
    }

    #[test]
    fn code_block_text_is_content() {
        // Regression: the verbatim code text must be a content payload on the
        // code chunk — it was dropped.
        let m = model("```text\nfn main() {}\n```\n");
        let code = m
            .chunk_section
            .chunks
            .iter()
            .find(|c| c.kind == ChunkKind::CodeBlock)
            .expect("code chunk");
        assert!(code.content_index != 0, "code chunk has a text payload");
        let payload = &m.content_section.payloads[(code.content_index - 1) as usize];
        assert_eq!(payload.kind, PayloadKind::TextUtf8);
        assert_eq!(
            std::str::from_utf8(&payload.data).expect("utf8"),
            // Code content is preserved verbatim (including the trailing newline).
            "fn main() {}\n"
        );
        assert_package_valid(&m);
    }

    #[test]
    fn table_cell_text_is_content() {
        // Regression: cell text (transparent inline content under a cell)
        // must emit content — it was dropped. The cell is a block container,
        // so the text wraps in an implicit paragraph.
        let src = "<table><tr><td>alpha</td><td>beta</td></tr></table>";
        let root = glyphcull_semantic::parse_html(src).expect("parse").tree;
        let m = build_chunk_model(&root, &[], &[]);
        let cells: Vec<&ChunkRecord> = m
            .chunk_section
            .chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::TableCell)
            .collect();
        assert_eq!(cells.len(), 2);
        for cell in &cells {
            assert!(cell.first_child_id != 0, "cell has a paragraph child");
            let para = m
                .chunk_section
                .chunks
                .iter()
                .find(|c| c.ordinal + 1 == cell.first_child_id)
                .expect("paragraph child");
            assert_eq!(para.kind, ChunkKind::Paragraph);
        }
        let text: Vec<&str> = m
            .content_section
            .payloads
            .iter()
            .filter(|p| p.kind == PayloadKind::TextUtf8)
            .map(|p| std::str::from_utf8(&p.data).expect("utf8"))
            .collect();
        assert!(text.contains(&"alpha"));
        assert!(text.contains(&"beta"));
        assert_package_valid(&m);
    }

    #[test]
    fn list_marker_glyphs_registered() {
        // Regression: marker glyphs must be in the atlas (runtimes never
        // synthesize typefaces). The default stylesheet gives ul → disc and
        // ol → decimal; the used-codepoint sets must cover the markers.
        let m = model("# T\n\n- one\n- two\n\n3. third\n4. fourth\n");
        let all: std::collections::BTreeSet<u32> = m
            .used_codepoints
            .values()
            .flat_map(|set| set.iter().copied())
            .collect();
        assert!(all.contains(&0x2022), "disc marker glyph present");
        assert!(all.contains(&b'.' as u32), "decimal separator present");
        for digit in 0_u32..10 {
            let cp = u32::from(b'0') + digit;
            assert!(all.contains(&cp), "digit {cp} present for counters");
        }
    }

    #[test]
    fn links_and_underlines() {
        let m = model("[text](https://e.test)\n");
        let styles = &m.resolved_styles;
        let run = m
            .chunk_section
            .chunks
            .iter()
            .find(|c| c.kind == ChunkKind::Run)
            .expect("run");
        assert!(styles[run.style_id as usize].underline);
        let link_extra = m
            .chunk_section
            .extras
            .iter()
            .find(|e| e.kind == ChunkExtraKind::LinkTarget)
            .expect("link extra");
        assert_eq!(link_extra.chunk_id, run.ordinal + 1);
        assert_package_valid(&m);
    }

    #[test]
    fn table_spans() {
        let src = "<table><tr><td colspan=\"2\">x</td></tr></table>";
        let root = glyphcull_semantic::parse_html(src).expect("parse").tree;
        let m = build_chunk_model(&root, &[], &[]);
        let span_extra = m
            .chunk_section
            .extras
            .iter()
            .find(|e| e.kind == ChunkExtraKind::CellSpan)
            .expect("span extra");
        assert_eq!(span_extra.data.len(), 4);
        assert_package_valid(&m);
    }

    #[test]
    fn images_registered() {
        let m = model("![alt text](img.png)\n");
        assert_eq!(m.images.len(), 1);
        assert_eq!(m.images[0].src, "img.png");
        assert_eq!(m.images[0].alt.as_deref(), Some("alt text"));
        assert_package_valid(&m);
    }

    #[test]
    fn codepoint_usage_per_face() {
        let m = model("Hello *World*\n");
        // Emphasis produces an italic face, so two faces are used; each records
        // exactly the codepoints it renders, and the union covers all text.
        assert_eq!(m.used_codepoints.len(), 2);
        let regular = m
            .used_codepoints
            .iter()
            .find(|(f, _)| !f.italic)
            .expect("regular face");
        assert_eq!(regular.0.family, "Noto Sans");
        assert_eq!(regular.0.weight, 400);
        let italic = m
            .used_codepoints
            .iter()
            .find(|(f, _)| f.italic)
            .expect("italic face");
        assert_eq!(italic.0.family, "Noto Sans");
        assert!(regular.1.contains(&('H' as u32)));
        assert!(regular.1.contains(&('e' as u32)));
        assert!(italic.1.contains(&('W' as u32)));
        assert!(italic.1.contains(&('d' as u32)));
        let union: BTreeSet<u32> = m
            .used_codepoints
            .values()
            .flat_map(|s| s.iter().copied())
            .collect();
        for c in "Hello World".chars() {
            assert!(union.contains(&(c as u32)), "missing codepoint {c:?}");
        }
    }

    #[test]
    fn determinism() {
        let md = "# T\n\n- a\n- b\n\n> quote *em*\n";
        let a = model(md);
        let b = model(md);
        assert_eq!(a.chunk_section.chunks, b.chunk_section.chunks);
        assert_eq!(a.resolved_styles, b.resolved_styles);
        assert_eq!(a.content_section, b.content_section);
        assert_eq!(a.used_codepoints, b.used_codepoints);
    }

    #[test]
    fn inline_span_class_styles_runs() {
        let src = r#"<style>.red { color: #ff0000; }</style><p>a <span class="red">b</span> c</p>"#;
        let out = glyphcull_semantic::parse_html(src).expect("parse");
        let root = &out.tree;
        let sheets: Vec<glyphcull_semantic::css::Stylesheet> = out
            .stylesheets
            .iter()
            .map(|s| glyphcull_semantic::css::parse_stylesheet(s).expect("css"))
            .collect();
        let m = build_chunk_model(root, &[], &sheets);
        let styles = &m.resolved_styles;
        let runs: Vec<u32> = m
            .chunk_section
            .chunks
            .iter()
            .filter(|c| c.kind == ChunkKind::Run)
            .map(|c| c.style_id)
            .collect();
        assert_eq!(runs.len(), 3);
        // The middle run (the span content) is red; the others are the default.
        assert!(
            styles[runs[1] as usize].color == 0xFF00_00FF,
            "span run must be red"
        );
        assert!(styles[runs[2] as usize].color == 0x0000_00FF);
        assert_package_valid(&m);
    }

    #[test]
    fn image_link_gets_link_target_extra() {
        let root = glyphcull_semantic::parse_markdown("[![alt](img.png)](https://e.test)\n")
            .expect("parse");
        let m = build_chunk_model(&root, &[], &[]);
        assert_eq!(m.images.len(), 1);
        let img = m
            .chunk_section
            .chunks
            .iter()
            .find(|c| c.kind == ChunkKind::Image)
            .expect("image chunk");
        let link = m
            .chunk_section
            .extras
            .iter()
            .find(|e| e.kind == ChunkExtraKind::LinkTarget)
            .expect("link extra");
        assert_eq!(link.chunk_id, img.ordinal + 1);
        let url = String::from_utf8(link.data[2..].to_vec()).expect("utf8");
        assert_eq!(url, "https://e.test");
        let alt = m
            .chunk_section
            .extras
            .iter()
            .find(|e| e.kind == ChunkExtraKind::ImageAlt)
            .expect("alt extra");
        assert_eq!(alt.chunk_id, img.ordinal + 1);
        assert_package_valid(&m);
    }

    #[test]
    fn multi_run_link_has_extra_per_run() {
        let root =
            glyphcull_semantic::parse_markdown("[a *b* c](https://e.test)\n").expect("parse");
        let m = build_chunk_model(&root, &[], &[]);
        let link_extras: Vec<&ChunkExtra> = m
            .chunk_section
            .extras
            .iter()
            .filter(|e| e.kind == ChunkExtraKind::LinkTarget)
            .collect();
        // The link has three runs (a, italic b, c), each with its own extra.
        assert_eq!(link_extras.len(), 3);
        assert_eq!(link_extras[0].chunk_id, link_extras[1].chunk_id - 1);
        assert_eq!(link_extras[1].chunk_id, link_extras[2].chunk_id - 1);
        // The italic run is styled italic.
        let styles = &m.resolved_styles;
        let italic_run = m
            .chunk_section
            .chunks
            .iter()
            .find(|c| c.kind == ChunkKind::Run && styles[c.style_id as usize].italic)
            .expect("italic run");
        assert_eq!(italic_run.ordinal + 1, link_extras[1].chunk_id);
        assert_package_valid(&m);
    }
}
