//! CHNK section codec (SPEC.md §2.2): the chunk graph.
//!
//! Records are fixed 44-byte rows for random access; extras (link targets, cell
//! spans, list values, alt text) are sparse and follow the record table. The codec
//! enforces dense ids (`record[i].id == i`), valid kinds/flags, and that extras
//! reference existing chunks. Tree-shape semantics are validated by
//! [`crate::validate`].

use crate::error::{Error, Result};
use crate::util::{Cursor, Writer};

/// A chunk record is exactly 44 bytes.
pub const CHUNK_RECORD_LEN: usize = 44;

/// Maximum chunk count (SPEC.md §1.3).
pub const MAX_CHUNK_COUNT: u32 = 1 << 28;

/// Maximum extras count (defensive; extras are sparse).
pub const MAX_EXTRA_COUNT: u32 = 1 << 26;

/// Maximum chunk depth (defensive; runaway nesting is a DoS vector).
pub const MAX_CHUNK_DEPTH: u32 = 1 << 16;

/// Chunk kinds (SPEC.md §2.2).
///
/// Variant names are self-descriptive; this is part of the documented exception to
/// the crate's `missing_docs` policy for domain enums.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ChunkKind {
    /// Root of the document.
    Document = 1,
    Heading1 = 2,
    Heading2 = 3,
    Heading3 = 4,
    Heading4 = 5,
    Heading5 = 6,
    Heading6 = 7,
    Paragraph = 8,
    Quote = 9,
    /// Structural wrapper for list items.
    List = 10,
    ListItem = 11,
    CodeBlock = 12,
    /// Structural wrapper for rows.
    Table = 13,
    TableRow = 14,
    TableCell = 15,
    Image = 16,
    Caption = 17,
    /// Inline text run.
    Run = 18,
    /// Inline link (text children + link_target extra).
    Link = 19,
    Br = 20,
    Hr = 21,
}

impl ChunkKind {
    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Parse a wire value; unknown values are errors in v1.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Document),
            2 => Some(Self::Heading1),
            3 => Some(Self::Heading2),
            4 => Some(Self::Heading3),
            5 => Some(Self::Heading4),
            6 => Some(Self::Heading5),
            7 => Some(Self::Heading6),
            8 => Some(Self::Paragraph),
            9 => Some(Self::Quote),
            10 => Some(Self::List),
            11 => Some(Self::ListItem),
            12 => Some(Self::CodeBlock),
            13 => Some(Self::Table),
            14 => Some(Self::TableRow),
            15 => Some(Self::TableCell),
            16 => Some(Self::Image),
            17 => Some(Self::Caption),
            18 => Some(Self::Run),
            19 => Some(Self::Link),
            20 => Some(Self::Br),
            21 => Some(Self::Hr),
            _ => None,
        }
    }

    /// Human-readable name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Heading1 => "heading1",
            Self::Heading2 => "heading2",
            Self::Heading3 => "heading3",
            Self::Heading4 => "heading4",
            Self::Heading5 => "heading5",
            Self::Heading6 => "heading6",
            Self::Paragraph => "paragraph",
            Self::Quote => "quote",
            Self::List => "list",
            Self::ListItem => "list_item",
            Self::CodeBlock => "code_block",
            Self::Table => "table",
            Self::TableRow => "table_row",
            Self::TableCell => "table_cell",
            Self::Image => "image",
            Self::Caption => "caption",
            Self::Run => "run",
            Self::Link => "link",
            Self::Br => "br",
            Self::Hr => "hr",
        }
    }

    /// True for wrappers that produce no geometry of their own.
    #[must_use]
    pub const fn is_structural(self) -> bool {
        matches!(
            self,
            Self::Document | Self::List | Self::Table | Self::TableRow
        )
    }

    /// True for inline kinds (nested inside block chunks).
    #[must_use]
    pub const fn is_inline(self) -> bool {
        matches!(self, Self::Run | Self::Link | Self::Br)
    }

    /// True for block-level renderable kinds.
    #[must_use]
    pub const fn is_block(self) -> bool {
        matches!(
            self,
            Self::Heading1
                | Self::Heading2
                | Self::Heading3
                | Self::Heading4
                | Self::Heading5
                | Self::Heading6
                | Self::Paragraph
                | Self::Quote
                | Self::ListItem
                | Self::CodeBlock
                | Self::TableCell
                | Self::Image
                | Self::Caption
                | Self::Hr
        )
    }
}

/// Chunk flag bits (SPEC.md §2.2).
pub mod flags {
    /// Excluded by semantic culling.
    pub const HIDDEN: u8 = 1 << 0;
    /// Layout hint: avoid a break between this chunk and the next.
    pub const KEEP_WITH_NEXT: u8 = 1 << 1;
    /// Layout hint: force a break before this chunk.
    pub const BREAK_BEFORE: u8 = 1 << 2;
    /// Suppress line wrapping (code).
    pub const NO_WRAP: u8 = 1 << 3;
    /// Structural: no direct geometry.
    pub const STRUCTURAL: u8 = 1 << 4;

    /// All defined flag bits (unknown bits are rejected).
    pub const ALL: u8 = 0x1F;
}

/// One chunk record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkRecord {
    /// Chunk kind.
    pub kind: ChunkKind,
    /// Flag bits (see [`flags`]).
    pub flags: u8,
    /// Resolved style id (0 = document default).
    pub style_id: u32,
    /// Parent chunk id (0 = none; only the root).
    pub parent_id: u32,
    /// Previous sibling id (0 = none).
    pub prev_id: u32,
    /// Next sibling id (0 = none).
    pub next_id: u32,
    /// First child id (0 = none).
    pub first_child_id: u32,
    /// Last child id (0 = none).
    pub last_child_id: u32,
    /// Index into CONT (0 = none).
    pub content_index: u32,
    /// Dense document-order ordinal.
    pub ordinal: u32,
    /// Depth (root = 0).
    pub depth: u32,
}

impl ChunkRecord {
    /// Encode as exactly [`CHUNK_RECORD_LEN`] bytes. `id` is the record's dense id.
    #[must_use]
    pub fn encode(&self, id: u32) -> [u8; CHUNK_RECORD_LEN] {
        let mut w = Writer::new();
        w.u32(id);
        w.u8(self.kind.to_u8());
        w.u8(self.flags);
        w.u16(0); // reserved
        w.u32(self.style_id);
        w.u32(self.parent_id);
        w.u32(self.prev_id);
        w.u32(self.next_id);
        w.u32(self.first_child_id);
        w.u32(self.last_child_id);
        w.u32(self.content_index);
        w.u32(self.ordinal);
        w.u32(self.depth);
        let bytes = w.into_bytes();
        // Fixed write sequence of exactly CHUNK_RECORD_LEN bytes; cannot fail.
        #[allow(clippy::expect_used)]
        {
            bytes.try_into().expect("chunk record is exactly 44 bytes")
        }
    }

    /// Decode a record; `expected_id` is the record's dense id (1-based).
    pub fn decode(bytes: &[u8; CHUNK_RECORD_LEN], expected_id: u32) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let id = c.u32("chunk id")?;
        if id != expected_id {
            return Err(Error::UnknownValue {
                what: "chunk id order",
                value: u64::from(id),
            });
        }
        let kind = ChunkKind::from_u8(c.u8("chunk kind")?).ok_or(Error::UnknownValue {
            what: "chunk kind",
            value: 0,
        })?;
        let flags = c.u8("chunk flags")?;
        let reserved = c.u16("chunk reserved")?;
        let style_id = c.u32("chunk style_id")?;
        let parent_id = c.u32("chunk parent_id")?;
        let prev_id = c.u32("chunk prev_id")?;
        let next_id = c.u32("chunk next_id")?;
        let first_child_id = c.u32("chunk first_child_id")?;
        let last_child_id = c.u32("chunk last_child_id")?;
        let content_index = c.u32("chunk content_index")?;
        let ordinal = c.u32("chunk ordinal")?;
        let depth = c.u32("chunk depth")?;
        c.finish("chunk record")?;
        if reserved != 0 {
            return Err(Error::ReservedBitsSet);
        }
        if flags & !flags::ALL != 0 {
            return Err(Error::UnknownValue {
                what: "chunk flags",
                value: u64::from(flags),
            });
        }
        if depth > MAX_CHUNK_DEPTH {
            return Err(Error::LimitExceeded {
                what: "chunk depth",
                value: u64::from(depth),
                limit: u64::from(MAX_CHUNK_DEPTH),
            });
        }
        // Structural flag must match kind.
        if kind.is_structural() != (flags & flags::STRUCTURAL != 0) {
            return Err(Error::UnknownValue {
                what: "chunk structural flag",
                value: u64::from(flags),
            });
        }
        Ok(Self {
            kind,
            flags,
            style_id,
            parent_id,
            prev_id,
            next_id,
            first_child_id,
            last_child_id,
            content_index,
            ordinal,
            depth,
        })
    }
}

/// Extra payload kinds (SPEC.md §2.2).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ChunkExtraKind {
    /// Link target: `u16 url_len` + UTF-8 URL.
    LinkTarget = 1,
    /// Table cell span: `u16 colspan` + `u16 rowspan`.
    CellSpan = 2,
    /// Explicit ordered-list value: `u32` (0 = auto).
    ListItemValue = 3,
    /// Image alt text: UTF-8.
    ImageAlt = 4,
}

impl ChunkExtraKind {
    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Parse a wire value.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::LinkTarget),
            2 => Some(Self::CellSpan),
            3 => Some(Self::ListItemValue),
            4 => Some(Self::ImageAlt),
            _ => None,
        }
    }
}

/// A sparse per-chunk extra payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkExtra {
    /// The owning chunk id.
    pub chunk_id: u32,
    /// The extra kind.
    pub kind: ChunkExtraKind,
    /// Kind-specific bytes (see [`ChunkExtraKind`]).
    pub data: Vec<u8>,
}

/// The decoded CHNK section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSection {
    /// Records in dense 1-based id order (`chunks[i].id == i + 1`; 0 is the "none"
    /// sentinel used by reference fields).
    pub chunks: Vec<ChunkRecord>,
    /// Extras sorted by (chunk_id, kind).
    pub extras: Vec<ChunkExtra>,
}

impl ChunkSection {
    /// Encode to the CHNK payload.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.chunks.len() as u32);
        for (i, chunk) in self.chunks.iter().enumerate() {
            // Chunk ids are 1-based dense (0 = none in reference fields).
            w.bytes(&chunk.encode((i + 1) as u32));
        }
        w.u32(self.extras.len() as u32);
        let mut extras = self.extras.clone();
        extras.sort_by_key(|e| (e.chunk_id, e.kind));
        for extra in extras {
            w.u32(extra.chunk_id);
            w.u8(extra.kind.to_u8());
            w.u8(0); // flags
            w.u16(extra.data.len() as u16);
            w.bytes(&extra.data);
        }
        w.into_bytes()
    }

    /// Decode and structurally validate the CHNK payload.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let chunk_count = c.u32("chunk count")?;
        if chunk_count > MAX_CHUNK_COUNT {
            return Err(Error::LimitExceeded {
                what: "chunk count",
                value: u64::from(chunk_count),
                limit: u64::from(MAX_CHUNK_COUNT),
            });
        }
        let mut chunks = Vec::with_capacity(chunk_count as usize);
        for i in 0..chunk_count {
            let record_bytes = c.take(CHUNK_RECORD_LEN, "chunk record")?;
            let record: &[u8; CHUNK_RECORD_LEN] =
                record_bytes.try_into().map_err(|_| Error::UnexpectedEof {
                    what: "chunk record",
                })?;
            // Chunk ids are 1-based dense (0 = none in reference fields).
            chunks.push(ChunkRecord::decode(record, i + 1)?);
        }
        let extra_count = c.u32("extra count")?;
        if extra_count > MAX_EXTRA_COUNT {
            return Err(Error::LimitExceeded {
                what: "extra count",
                value: u64::from(extra_count),
                limit: u64::from(MAX_EXTRA_COUNT),
            });
        }
        let mut extras = Vec::with_capacity(extra_count as usize);
        for _ in 0..extra_count {
            let chunk_id = c.u32("extra chunk id")?;
            // Chunk ids are 1-based dense: valid ids are 1..=chunk_count.
            if chunk_id == 0 || chunk_id > chunk_count {
                return Err(Error::DanglingReference {
                    what: "extra chunk",
                    id: chunk_id,
                });
            }
            let kind = ChunkExtraKind::from_u8(c.u8("extra kind")?).ok_or(Error::UnknownValue {
                what: "extra kind",
                value: 0,
            })?;
            let flags = c.u8("extra flags")?;
            let data_len = usize::from(c.u16("extra data len")?);
            if flags != 0 {
                return Err(Error::ReservedBitsSet);
            }
            let data = c.take(data_len, "extra data")?.to_vec();
            extras.push(ChunkExtra {
                chunk_id,
                kind,
                data,
            });
        }
        c.finish("CHNK payload")?;
        Ok(Self { chunks, extras })
    }

    /// The number of chunks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Whether the section is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{flags, ChunkExtra, ChunkExtraKind, ChunkKind, ChunkRecord, ChunkSection};
    use crate::error::Error;

    fn sample_section() -> ChunkSection {
        ChunkSection {
            chunks: vec![
                ChunkRecord {
                    kind: ChunkKind::Document,
                    flags: flags::STRUCTURAL,
                    style_id: 0,
                    parent_id: 0,
                    prev_id: 0,
                    next_id: 0,
                    first_child_id: 1,
                    last_child_id: 1,
                    content_index: 0,
                    ordinal: 0,
                    depth: 0,
                },
                ChunkRecord {
                    kind: ChunkKind::Paragraph,
                    flags: 0,
                    style_id: 1,
                    parent_id: 0,
                    prev_id: 0,
                    next_id: 0,
                    first_child_id: 0,
                    last_child_id: 0,
                    content_index: 1,
                    ordinal: 1,
                    depth: 1,
                },
            ],
            extras: vec![ChunkExtra {
                chunk_id: 1,
                kind: ChunkExtraKind::ImageAlt,
                data: b"alt text".to_vec(),
            }],
        }
    }

    #[test]
    fn round_trip() {
        let section = sample_section();
        let bytes = section.encode();
        assert_eq!(ChunkSection::decode(&bytes).expect("decode"), section);
    }

    #[test]
    fn extras_sorted_on_encode() {
        let mut section = sample_section();
        section.extras.push(ChunkExtra {
            chunk_id: 1,
            kind: ChunkExtraKind::LinkTarget,
            data: b"https://example.com".to_vec(),
        });
        let bytes = section.encode();
        let decoded = ChunkSection::decode(&bytes).expect("decode");
        assert_eq!(
            decoded.extras.iter().map(|e| e.kind).collect::<Vec<_>>(),
            vec![ChunkExtraKind::LinkTarget, ChunkExtraKind::ImageAlt] // sorted by kind
        );
    }

    #[test]
    fn dense_id_enforced() {
        // Corrupt the implicit id of the second record.
        let section = sample_section();
        let bytes = section.encode();
        let id_field = 4 + 44; // record 1 starts at byte 4 + 44
        let mut corrupted = bytes;
        corrupted[id_field] = 5;
        assert!(ChunkSection::decode(&corrupted).is_err());
    }

    #[test]
    fn unknown_kind_rejected() {
        let section = sample_section();
        let bytes = section.encode();
        let mut corrupted = bytes;
        corrupted[4 + 4] = 99; // kind of record 0
        assert!(matches!(
            ChunkSection::decode(&corrupted),
            Err(Error::UnknownValue { .. })
        ));
    }

    #[test]
    fn unknown_flags_rejected() {
        let section = sample_section();
        let bytes = section.encode();
        let mut corrupted = bytes;
        corrupted[4 + 5] = 0x80; // unknown flag bit
        assert!(ChunkSection::decode(&corrupted).is_err());
    }

    #[test]
    fn dangling_extra_rejected() {
        let section = sample_section();
        let bytes = section.encode();
        // extra chunk_id is the last 8 bytes of the header + ... locate it:
        // 4 (count) + 2*44 (records) + 4 (extra count) = 96; chunk_id u32 at 96.
        let mut corrupted = bytes;
        corrupted[96] = 7;
        assert!(matches!(
            ChunkSection::decode(&corrupted),
            Err(Error::DanglingReference { .. })
        ));
    }

    #[test]
    fn structural_flag_must_match_kind() {
        let section = sample_section();
        let bytes = section.encode();
        let mut corrupted = bytes;
        corrupted[4 + 5] = 0; // clear STRUCTURAL on the document chunk
        assert!(ChunkSection::decode(&corrupted).is_err());
    }

    #[test]
    fn kind_classification() {
        assert!(ChunkKind::Document.is_structural());
        assert!(ChunkKind::List.is_structural());
        assert!(ChunkKind::Run.is_inline());
        assert!(ChunkKind::Link.is_inline());
        assert!(ChunkKind::Paragraph.is_block());
        assert!(!ChunkKind::Br.is_block());
    }
}
