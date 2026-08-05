//! Section kinds and the canonical emission order (SPEC.md §1.4).

/// A section kind. Unknown numeric kinds are preserved by the reader (forward
/// compatibility) but never interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum SectionKind {
    /// Metadata (deterministic JSON).
    Info = 1,
    /// Chunk graph.
    Chunk = 2,
    /// Resolved style table.
    Style = 3,
    /// Content payloads (text, image refs).
    Content = 4,
    /// MSDF glyph atlases.
    Glyph = 5,
    /// Raster images.
    Images = 6,
    /// Integrity hash tree.
    Seal = 7,
}

impl SectionKind {
    /// Convert to the numeric kind.
    #[must_use]
    pub const fn to_u32(self) -> u32 {
        self as u32
    }

    /// Convert from a numeric kind; returns `None` for reserved/unknown kinds.
    #[must_use]
    pub const fn from_u32(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Info),
            2 => Some(Self::Chunk),
            3 => Some(Self::Style),
            4 => Some(Self::Content),
            5 => Some(Self::Glyph),
            6 => Some(Self::Images),
            7 => Some(Self::Seal),
            _ => None,
        }
    }

    /// The stable name used in diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Chunk => "CHNK",
            Self::Style => "STYL",
            Self::Content => "CONT",
            Self::Glyph => "GLYF",
            Self::Images => "IMGS",
            Self::Seal => "SEAL",
        }
    }
}

/// The canonical section emission order (SPEC.md §1.4).
pub const CANONICAL_ORDER: [SectionKind; 7] = [
    SectionKind::Info,
    SectionKind::Chunk,
    SectionKind::Style,
    SectionKind::Content,
    SectionKind::Glyph,
    SectionKind::Images,
    SectionKind::Seal,
];

/// True if the writer compresses this kind by default (SPEC.md §1.4).
#[must_use]
pub const fn default_compressed(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::Info | SectionKind::Chunk | SectionKind::Style | SectionKind::Content
    )
}

#[cfg(test)]
mod tests {
    use super::{SectionKind, CANONICAL_ORDER};

    #[test]
    fn canonical_order_matches_enum_values() {
        let values: Vec<u32> = CANONICAL_ORDER.iter().map(|k| k.to_u32()).collect();
        assert_eq!(values, vec![1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn round_trip() {
        for kind in CANONICAL_ORDER {
            assert_eq!(SectionKind::from_u32(kind.to_u32()), Some(kind));
        }
        assert_eq!(SectionKind::from_u32(0), None);
        assert_eq!(SectionKind::from_u32(8), None);
        assert_eq!(SectionKind::from_u32(0xFFFF_FFFF), None);
    }
}
