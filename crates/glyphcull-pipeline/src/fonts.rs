//! The bundled font registry: maps a resolved face (family + weight + italic)
//! to font bytes for atlas generation.
//!
//! The compiler ships the four Noto Sans faces (Regular, Bold, Italic,
//! BoldItalic; SIL OFL — see `assets/fonts/OFL.txt`). Resolution is
//! deterministic: the family is matched case-insensitively (CSS semantics),
//! the generic families `sans-serif` and `serif` map to the default family,
//! and the nearest available weight is used when the exact weight is absent.
//! Any other family is an error — the compiler must never silently substitute
//! an unknown typeface.

use std::collections::BTreeMap;

/// The bundled default family name.
pub const DEFAULT_FAMILY: &str = "Noto Sans";

/// The bundled Noto Sans Regular face.
pub const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
/// The bundled Noto Sans Bold face.
pub const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");
/// The bundled Noto Sans Italic face.
pub const FONT_ITALIC: &[u8] = include_bytes!("../assets/fonts/NotoSans-Italic.ttf");
/// The bundled Noto Sans Bold Italic face.
pub const FONT_BOLD_ITALIC: &[u8] = include_bytes!("../assets/fonts/NotoSans-BoldItalic.ttf");
/// The supplementary glyph face (DejaVu Sans, Block Elements subset — see
/// `assets/fonts/DejaVu-LICENSE.txt`): shape glyphs such as U+2588 FULL BLOCK
/// that the text faces do not carry. Atlas generation falls back to this face
/// for codepoints the primary font lacks, so glyph-drawn bar charts render as
/// real filled bars. Deterministic and additive: atlases change only when a
/// document actually uses one of these codepoints.
pub const FONT_SUPPLEMENTARY_BLOCKS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Block.ttf");

/// A font-family resolution failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontError {
    /// No font registered under this family (and it is not a generic).
    UnknownFamily(String),
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::UnknownFamily(family) => {
                write!(f, "no bundled font for family {family:?}")
            }
        }
    }
}

impl std::error::Error for FontError {}

/// The registry of bundled faces, keyed by lowercase family name.
#[derive(Debug, Clone, Default)]
pub struct FontRegistry {
    /// family (lowercase) → (weight, italic) → bytes.
    faces: BTreeMap<String, BTreeMap<(u16, bool), &'static [u8]>>,
}

impl FontRegistry {
    /// The registry with the four bundled Noto Sans faces.
    #[must_use]
    pub fn bundled() -> Self {
        let mut registry = Self::default();
        registry.add(DEFAULT_FAMILY, 400, false, FONT_REGULAR);
        registry.add(DEFAULT_FAMILY, 700, false, FONT_BOLD);
        registry.add(DEFAULT_FAMILY, 400, true, FONT_ITALIC);
        registry.add(DEFAULT_FAMILY, 700, true, FONT_BOLD_ITALIC);
        registry
    }

    /// Register a face (later registrations of the same spec are ignored —
    /// first wins, keeping resolution deterministic).
    pub fn add(&mut self, family: &str, weight: u16, italic: bool, bytes: &'static [u8]) {
        let entry = self.faces.entry(family.to_ascii_lowercase()).or_default();
        entry
            .entry((weight.clamp(100, 900), italic))
            .or_insert(bytes);
    }

    /// The normalized registry key for a family name: the generic families map
    /// to the default family; everything else is matched case-insensitively.
    fn key(family: &str) -> String {
        match family.to_ascii_lowercase().as_str() {
            "sans-serif" | "serif" => DEFAULT_FAMILY.to_ascii_lowercase(),
            other => other.to_string(),
        }
    }

    /// Resolve a face to font bytes.
    pub fn resolve(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
    ) -> Result<&'static [u8], FontError> {
        let faces = self
            .faces
            .get(&Self::key(family))
            .ok_or_else(|| FontError::UnknownFamily(family.to_string()))?;
        let weight = weight.clamp(100, 900);
        if let Some(bytes) = faces.get(&(weight, italic)) {
            return Ok(bytes);
        }
        // Nearest available weight (deterministic: prefer heavier on ties).
        let nearest = faces
            .keys()
            .filter(|(_, i)| *i == italic)
            .min_by_key(|(w, _)| (weight.abs_diff(*w), std::cmp::Reverse(*w)));
        if let Some(&(w, _)) = nearest {
            if let Some(bytes) = faces.get(&(w, italic)) {
                return Ok(bytes);
            }
        }
        Err(FontError::UnknownFamily(family.to_string()))
    }

    /// The registered families (sorted, deterministic).
    #[must_use]
    pub fn families(&self) -> Vec<&str> {
        self.faces.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_resolves_defaults() {
        let registry = FontRegistry::bundled();
        assert!(registry.resolve("Noto Sans", 400, false).is_ok());
        assert!(registry.resolve("Noto Sans", 700, true).is_ok());
        // Case-insensitive matching (CSS semantics).
        assert!(registry.resolve("noto sans", 400, false).is_ok());
        // Generic families map to Noto Sans.
        assert!(registry.resolve("sans-serif", 400, false).is_ok());
        assert!(registry.resolve("SERIF", 400, false).is_ok());
        // Nearest weight: 500 → regular, 900 → bold.
        assert_eq!(registry.resolve("Noto Sans", 500, false), Ok(FONT_REGULAR));
        assert_eq!(registry.resolve("Noto Sans", 900, false), Ok(FONT_BOLD));
        // Unknown family is an error.
        assert!(matches!(
            registry.resolve("Comic Sans", 400, false),
            Err(FontError::UnknownFamily(_))
        ));
    }

    #[test]
    fn resolution_is_deterministic() {
        let registry = FontRegistry::bundled();
        for (family, weight, italic) in [
            ("Noto Sans", 400, false),
            ("Noto Sans", 700, true),
            ("sans-serif", 300, false),
        ] {
            let a = registry.resolve(family, weight, italic);
            let b = registry.resolve(family, weight, italic);
            assert_eq!(a, b);
        }
    }

    #[test]
    fn bundled_faces_parse() {
        for bytes in [FONT_REGULAR, FONT_BOLD, FONT_ITALIC, FONT_BOLD_ITALIC] {
            assert!(glyphcull_atlas::font::FontFace::parse(bytes).is_ok());
        }
    }
}
