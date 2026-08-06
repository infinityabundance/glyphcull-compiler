//! Font parsing and outline extraction (ttf-parser).
//!
//! The atlas consumes raw font bytes and a codepoint set; this module owns the
//! font side of that contract: face metrics (in em), glyph outlines (contours of
//! line/quadratic edges, cubic curves converted with a provable error bound),
//! advance/bearing metrics, and kerning pair extraction (see [`crate::gpos`]).
//!
//! All coordinates leave this module in *font units* (f64, y-up); the SDF stage
//! applies the texel mapping.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::error::Error;
use crate::geometry::{cubic_to_quadratics, Line, Point, Quadratic};
use crate::gpos::kerning_pairs;

/// Typographic metrics of a face, in em (1.0 = one em).
#[derive(Debug, Clone, PartialEq)]
pub struct FontInfo {
    /// Font units per em.
    pub units_per_em: f32,
    /// Typographic ascent in em (positive, above baseline).
    pub ascent_em: f32,
    /// Typographic descent in em (positive, below baseline).
    pub descent_em: f32,
    /// Line gap in em.
    pub line_gap_em: f32,
    /// Cap height in em (0.0 if the font does not declare it).
    pub cap_height_em: f32,
    /// X height in em (0.0 if the font does not declare it).
    pub x_height_em: f32,
    /// Family name (name table id 1, first Unicode name found).
    pub family: String,
    /// Weight class 100..=900.
    pub weight: u16,
    /// Italic flag.
    pub italic: bool,
}

/// One edge of a glyph outline (font units, y-up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edge {
    /// A straight segment.
    Line(Line),
    /// A quadratic Bézier segment.
    Quad(Quadratic),
}

impl Edge {
    /// The start point.
    #[must_use]
    pub fn start(self) -> Point {
        match self {
            Edge::Line(l) => l.a,
            Edge::Quad(q) => q.a,
        }
    }

    /// The end point.
    #[must_use]
    pub fn end(self) -> Point {
        match self {
            Edge::Line(l) => l.b,
            Edge::Quad(q) => q.b,
        }
    }
}

/// A glyph outline: closed contours of edges.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphOutline {
    /// Contours (each implicitly closed back to its start).
    pub contours: Vec<Vec<Edge>>,
}

impl GlyphOutline {
    /// The tight bounding box of the outline (font units), or `None` for an
    /// empty outline (no contours or no points).
    #[must_use]
    pub fn bounds(&self) -> Option<(Point, Point)> {
        let mut min = Point::new(f64::INFINITY, f64::INFINITY);
        let mut max = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut any = false;
        for contour in &self.contours {
            for edge in contour {
                for p in [edge.start(), edge.end()] {
                    any = true;
                    if p.x < min.x {
                        min.x = p.x;
                    }
                    if p.y < min.y {
                        min.y = p.y;
                    }
                    if p.x > max.x {
                        max.x = p.x;
                    }
                    if p.y > max.y {
                        max.y = p.y;
                    }
                }
            }
        }
        if any {
            Some((min, max))
        } else {
            None
        }
    }
}

/// A parsed font face bound to its source bytes.
pub struct FontFace<'a> {
    face: Face<'a>,
}

/// The `OutlineBuilder` implementation that collects a [`GlyphOutline`].
struct OutlineCollector {
    contours: Vec<Vec<Edge>>,
    current: Vec<Edge>,
    pos: Point,
    start: Point,
    /// Cubic→quadratic tolerance in font units.
    tolerance_units: f64,
}

impl OutlineCollector {
    fn new(tolerance_units: f64) -> Self {
        Self {
            contours: Vec::new(),
            current: Vec::new(),
            pos: Point::new(0.0, 0.0),
            start: Point::new(0.0, 0.0),
            tolerance_units,
        }
    }

    fn finish(mut self) -> GlyphOutline {
        self.close_current();
        GlyphOutline {
            contours: self.contours,
        }
    }

    /// Close the in-progress contour with an implicit edge back to its start.
    fn close_current(&mut self) {
        if !self.current.is_empty() {
            if self.pos.dist_sq(self.start) > f64::EPSILON {
                self.current.push(Edge::Line(Line {
                    a: self.pos,
                    b: self.start,
                }));
            }
            self.contours.push(std::mem::take(&mut self.current));
        }
        self.pos = self.start;
    }
}

impl OutlineBuilder for OutlineCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close_current();
        self.start = Point::new(f64::from(x), f64::from(y));
        self.pos = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let b = Point::new(f64::from(x), f64::from(y));
        if self.pos.dist_sq(b) > f64::EPSILON {
            self.current.push(Edge::Line(Line { a: self.pos, b }));
            self.pos = b;
        }
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let c = Point::new(f64::from(x1), f64::from(y1));
        let b = Point::new(f64::from(x), f64::from(y));
        if self.pos.dist_sq(b) > f64::EPSILON {
            self.current
                .push(Edge::Quad(Quadratic { a: self.pos, c, b }));
            self.pos = b;
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p1 = Point::new(f64::from(x1), f64::from(y1));
        let p2 = Point::new(f64::from(x2), f64::from(y2));
        let p3 = Point::new(f64::from(x), f64::from(y));
        if self.pos.dist_sq(p3) > f64::EPSILON {
            for q in cubic_to_quadratics(self.pos, p1, p2, p3, self.tolerance_units) {
                self.current.push(Edge::Quad(q));
            }
            self.pos = p3;
        }
    }

    fn close(&mut self) {
        self.close_current();
    }
}

impl<'a> FontFace<'a> {
    /// Parse a face from raw font bytes.
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        let face = Face::parse(data, 0).map_err(|_| Error::FontParseFailed)?;
        Ok(Self { face })
    }

    /// Typographic metrics in em.
    #[must_use]
    pub fn info(&self) -> FontInfo {
        let upm = f32::from(self.face.units_per_em()).max(1.0);
        let asc = self
            .face
            .typographic_ascender()
            .unwrap_or_else(|| self.face.ascender());
        let desc = self
            .face
            .typographic_descender()
            .unwrap_or_else(|| self.face.descender());
        let gap = self
            .face
            .typographic_line_gap()
            .unwrap_or_else(|| self.face.line_gap());
        let family = family_name(&self.face).unwrap_or_else(|| "Unknown".to_string());
        FontInfo {
            units_per_em: upm,
            ascent_em: f32::from(asc) / upm,
            descent_em: f32::from(desc).abs() / upm,
            line_gap_em: f32::from(gap) / upm,
            cap_height_em: self
                .face
                .capital_height()
                .map_or(0.0, |v| f32::from(v) / upm),
            x_height_em: self.face.x_height().map_or(0.0, |v| f32::from(v) / upm),
            family,
            weight: self.face.weight().to_number(),
            italic: self.face.is_italic() || self.face.is_oblique(),
        }
    }

    /// The glyph id for a codepoint, if the cmap maps it.
    #[must_use]
    pub fn glyph_index(&self, codepoint: u32) -> Option<u16> {
        let c = char::from_u32(codepoint)?;
        let gid = self.face.glyph_index(c)?;
        Some(gid.0)
    }

    /// Horizontal advance in font units.
    #[must_use]
    pub fn advance_units(&self, glyph: u16) -> Option<u16> {
        self.face.glyph_hor_advance(GlyphId(glyph))
    }

    /// Left side bearing in font units.
    #[must_use]
    pub fn lsb_units(&self, glyph: u16) -> Option<i16> {
        self.face.glyph_hor_side_bearing(GlyphId(glyph))
    }

    /// The glyph outline (font units), cubic curves converted with the given
    /// tolerance (font units). `None` when the glyph has no outline data.
    #[must_use]
    pub fn outline(&self, glyph: u16, tolerance_units: f64) -> Option<GlyphOutline> {
        let mut collector = OutlineCollector::new(tolerance_units);
        self.face.outline_glyph(GlyphId(glyph), &mut collector)?;
        let outline = collector.finish();
        if outline.contours.is_empty() {
            return None;
        }
        Some(outline)
    }

    /// Kerning adjustments in font units for every pair whose left and right
    /// glyphs both belong to `codepoints` (GPOS PairPos and `kern` table,
    /// accumulated per the OpenType model). Keyed by codepoint pair.
    #[must_use]
    pub fn kerning_units(&self, codepoints: &BTreeSet<u32>) -> BTreeMap<(u32, u32), i32> {
        kerning_pairs(&self.face, codepoints)
    }
}

/// The family name (name id 1): the first Unicode-platform name wins.
fn family_name(face: &Face<'_>) -> Option<String> {
    let mut fallback: Option<String> = None;
    for name in face.names() {
        if name.name_id != ttf_parser::name_id::FAMILY {
            continue;
        }
        if let Some(s) = name.to_string() {
            if name.is_unicode() {
                return Some(s);
            }
            if fallback.is_none() {
                fallback = Some(s);
            }
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    /// The bundled Noto Sans Regular (present in the compiler assets).
    const NOTO: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");

    #[test]
    fn parses_metrics() {
        let face = FontFace::parse(NOTO).expect("font");
        let info = face.info();
        assert_eq!(info.units_per_em, 1000.0);
        assert!(info.ascent_em > 0.7 && info.ascent_em < 1.2);
        assert!(info.descent_em > 0.1 && info.descent_em < 0.5);
        assert_eq!(info.weight, 400);
        assert!(!info.italic);
        assert!(info.family.contains("Noto Sans"));
        assert!(info.cap_height_em > 0.6);
        assert!(info.x_height_em > 0.4);
    }

    #[test]
    fn glyph_index_and_metrics() {
        let face = FontFace::parse(NOTO).expect("font");
        let a = face.glyph_index('A' as u32).expect("A");
        let space = face.glyph_index(' ' as u32).expect("space");
        assert!(a != space);
        assert!(face.advance_units(a).expect("advance") > 500);
        assert!(face.advance_units(space).expect("advance") > 0);
        let _ = face.lsb_units(a);
    }

    #[test]
    fn outline_of_a_has_contours() {
        let face = FontFace::parse(NOTO).expect("font");
        let a = face.glyph_index('A' as u32).expect("A");
        let outline = face.outline(a, 1e-3).expect("outline");
        assert!(!outline.contours.is_empty());
        let (min, max) = outline.bounds().expect("bounds");
        assert!(min.x >= 0.0); // A's left bearing is positive in Noto Sans
        assert!(max.y > 600.0 && max.y < 800.0);
        assert!(max.x > 500.0);
        // All edges must be line or quad (never bare points).
        for contour in &outline.contours {
            assert!(!contour.is_empty());
            // The contour must be closed: last edge ends at the first start.
            let first_start = contour.first().expect("first").start();
            let last_end = contour.last().expect("last").end();
            assert!(first_start.dist_sq(last_end) < 1e-6);
        }
    }

    #[test]
    fn space_has_no_outline() {
        let face = FontFace::parse(NOTO).expect("font");
        let space = face.glyph_index(' ' as u32).expect("space");
        assert!(face.outline(space, 1e-3).is_none());
    }

    #[test]
    fn invalid_font_rejected() {
        assert!(matches!(
            FontFace::parse(b"not a font"),
            Err(Error::FontParseFailed)
        ));
    }

    #[test]
    fn kerning_extraction_deterministic() {
        let face = FontFace::parse(NOTO).expect("font");
        let mut cps = BTreeSet::new();
        // Common kerning pairs: AV, To, etc.
        for c in "AVToWYa".chars() {
            cps.insert(c as u32);
        }
        let k1 = face.kerning_units(&cps);
        let k2 = face.kerning_units(&cps);
        assert_eq!(k1, k2);
        // AV is a classic negative kerning pair.
        let av = (('A' as u32), ('V' as u32));
        assert!(
            k1.get(&av).copied().unwrap_or(0) < 0,
            "A/V should kern negatively, got {k1:?}"
        );
    }
}
