//! `glyphcull-atlas` — the GlyphCull compiler's MSDF glyph atlas generator.
//!
//! Produces the resolution-independent multi-channel signed-distance-field
//! atlases (SPEC.md GLYF) that runtimes sample at any size: exact signed
//! distance to line and quadratic Bézier edges, bounded-error cubic→quadratic
//! conversion, MSDF edge coloring with median-of-three corner reconstruction,
//! deterministic skyline packing, and a reference rasterizer for rendering
//! validation (PERFORMANCE.md).
//!
//! # Pipeline
//!
//! ```text
//! font bytes + codepoints
//!   │  FontFace::parse ── metrics, outlines (font units)
//!   │  render_msdf ── 3-channel exact SDF per glyph (texels)
//!   │  pack_rects ── skyline packing into pages
//!   │  kerning_units ── GPOS PairPos + kern → sorted pairs
//!   ▼
//! Atlas (format-0 MSDF RGBA8, glyph records sorted by codepoint)
//! ```
//!
//! Determinism is a contract: same font bytes + same codepoint set + same
//! options ⇒ byte-identical atlas. Every iteration is over sorted structures;
//! no hash maps, no timestamps, no randomness.
//!
//! # Validation
//!
//! [`raster::msdf_coverage`] vs [`raster::rasterize_coverage`] compare the
//! reconstructed MSDF output against direct supersampled rasterization; the
//! committed tolerances live in the integration tests (PERFORMANCE.md §Rendering
//! validation).
//!
//! Direct indexing in the build path is the documented exception (as in
//! `glyphcull-chunk::graph`): every index is derived from the pack output, whose
//! bounds are established by the packer, or from `c in 0..3`.

#![allow(clippy::indexing_slicing)]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

pub mod correction;
pub mod error;
pub mod font;
pub mod geometry;
pub mod gpos;
pub mod pack;
pub mod raster;
pub mod sdf;

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use glyphcull_format::codec::glyph::{glyph_flags, Atlas, GlyphRecord, KerningPair};

use error::Error;
use font::{FontFace, GlyphOutline};

/// Default texels per em, fixed-point ×1024 (32 texels per em).
pub const DEFAULT_TEXELS_PER_EM: u32 = 32 * 1024;
/// Default SDF padding (texels) around each glyph box.
pub const DEFAULT_PADDING: u16 = 4;
/// Default atlas page size (texels).
pub const DEFAULT_PAGE_SIZE: u32 = 1024;
/// Default cubic→quadratic tolerance in texels (bounded-error conversion).
pub const DEFAULT_CUBIC_TOLERANCE_TEXELS: f64 = 0.1;

/// Atlas generation options. Values are validated by [`build_atlas`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasOptions {
    /// Atlas density, fixed-point ×1024 texels per em (SPEC.md §2.5).
    pub texels_per_em: u32,
    /// SDF margin in texels around every glyph box.
    pub padding: u16,
    /// Page width in texels.
    pub page_width: u32,
    /// Page height in texels.
    pub page_height: u32,
    /// Cubic→quadratic conversion tolerance in texels.
    pub cubic_tolerance_texels: f64,
}

impl Default for AtlasOptions {
    fn default() -> Self {
        Self {
            texels_per_em: DEFAULT_TEXELS_PER_EM,
            padding: DEFAULT_PADDING,
            page_width: DEFAULT_PAGE_SIZE,
            page_height: DEFAULT_PAGE_SIZE,
            cubic_tolerance_texels: DEFAULT_CUBIC_TOLERANCE_TEXELS,
        }
    }
}

/// The result of an atlas build: the atlas plus the codepoints that could not
/// be mapped to glyphs in the font (absent from the atlas).
#[derive(Debug, Clone, PartialEq)]
pub struct AtlasResult {
    /// The built atlas (records sorted by codepoint).
    pub atlas: Atlas,
    /// Codepoints with no glyph in the font (sorted).
    pub missing: Vec<u32>,
}

/// Build an MSDF atlas for `codepoints` (sorted, deduplicated) from raw font
/// bytes. `font_id` is stamped onto the atlas (the pipeline assigns it).
pub fn build_atlas(
    font_bytes: &[u8],
    codepoints: &BTreeSet<u32>,
    font_id: u32,
    options: &AtlasOptions,
) -> Result<AtlasResult, Error> {
    if options.texels_per_em == 0 {
        return Err(Error::InvalidOption("texels_per_em must be > 0"));
    }
    if options.padding == 0 {
        return Err(Error::InvalidOption("padding must be > 0"));
    }
    if options.page_width == 0 || options.page_height == 0 {
        return Err(Error::InvalidOption("page dimensions must be > 0"));
    }
    if !options.cubic_tolerance_texels.is_finite() || options.cubic_tolerance_texels <= 0.0 {
        return Err(Error::InvalidOption(
            "cubic_tolerance_texels must be finite and > 0",
        ));
    }

    let face = FontFace::parse(font_bytes)?;
    let info = face.info();
    let upm = f64::from(info.units_per_em);
    let texels_per_em_f = f64::from(options.texels_per_em) / 1024.0;
    let scale = texels_per_em_f / upm;
    let tolerance_units = options.cubic_tolerance_texels / scale;

    // 1. Collect per-glyph data in codepoint order (deterministic).
    struct GlyphWork {
        codepoint: u32,
        advance_em: f32,
        bearing_x_em: f32,
        bearing_y_em: f32,
        rendered: Option<sdf::RenderedGlyph>,
        flags: u8,
        order: usize,
    }
    let mut work: Vec<GlyphWork> = Vec::new();
    let mut missing: Vec<u32> = Vec::new();
    for (order, &cp) in codepoints.iter().enumerate() {
        let Some(gid) = face.glyph_index(cp) else {
            missing.push(cp);
            continue;
        };
        let advance = face.advance_units(gid).unwrap_or(0);
        let lsb = face.lsb_units(gid).unwrap_or(0);
        let outline: Option<GlyphOutline> = face.outline(gid, tolerance_units);
        let (rendered, bounds, flags) = match &outline {
            Some(o) => {
                let rendered = sdf::render_msdf(o, scale, f64::from(options.padding));
                let bounds = o.bounds();
                let mut flags = 0_u8;
                if advance == 0 {
                    flags |= glyph_flags::COMBINING;
                }
                (rendered, bounds, flags)
            }
            None => {
                let mut flags = glyph_flags::NO_OUTLINE;
                if advance == 0 {
                    flags |= glyph_flags::COMBINING;
                }
                (None, None, flags)
            }
        };
        let bearing_y_em = bounds.map_or(0.0, |(_, max)| max.y / upm) as f32;
        work.push(GlyphWork {
            codepoint: cp,
            advance_em: f32::from(advance) / info.units_per_em,
            bearing_x_em: f32::from(lsb) / info.units_per_em,
            bearing_y_em,
            rendered,
            flags,
            order,
        });
    }
    missing.sort_unstable();

    // 2. Pack boxes: heights first (packing efficiency), codepoints as the
    //    deterministic tie-break. `order` restores the codepoint sequence.
    let mut pack_input: Vec<(u32, u32, usize)> = Vec::with_capacity(work.len());
    for (index, g) in work.iter().enumerate() {
        let (w, h) = match &g.rendered {
            Some(r) => (u32::from(r.width), u32::from(r.height)),
            None => (1, 1),
        };
        pack_input.push((w, h, index));
    }
    pack_input.sort_by(|a, b| {
        (b.1, a.2).cmp(&(a.1, b.2)) // height desc, then codepoint order asc
    });
    let rects: Vec<(u32, u32)> = pack_input.iter().map(|&(w, h, _)| (w, h)).collect();
    let placed = pack::pack_rects(&rects, options.page_width, options.page_height);
    let mut placed_by_order: BTreeMap<usize, pack::PlacedRect> = BTreeMap::new();
    for (slot, &p) in pack_input.iter().zip(placed.iter()) {
        placed_by_order.insert(slot.2, p);
    }

    // 3. Build pages and copy pixels.
    let page_count = placed
        .iter()
        .map(|p| usize::from(p.page))
        .max()
        .map_or(1, |m| m + 1);
    let page_bytes = usize::try_from(options.page_width)
        .ok()
        .and_then(|w| usize::try_from(options.page_height).ok().map(|h| w * h * 4))
        .ok_or(Error::InvalidOption("page dimensions too large"))?;
    let mut pages: Vec<Vec<u8>> = vec![vec![0_u8; page_bytes]; page_count];
    let mut records: Vec<GlyphRecord> = Vec::with_capacity(work.len());
    for g in &work {
        // Every work item is packed (the pack input covers all orders), so the
        // fallback never triggers; `unwrap_or` keeps the code panic-free.
        let p = placed_by_order
            .get(&g.order)
            .copied()
            .unwrap_or(pack::PlacedRect {
                x: 0,
                y: 0,
                page: 0,
            });
        match &g.rendered {
            Some(r) => {
                let w = usize::from(r.width);
                let h = usize::from(r.height);
                let dst_x = usize::from(p.x);
                let dst_y = usize::from(p.y);
                let page_w = options.page_width as usize;
                let page = &mut pages[usize::from(p.page)];
                for row in 0..h {
                    let src = &r.pixels[row * w * 4..(row + 1) * w * 4];
                    let dst = (dst_y + row) * page_w * 4 + dst_x * 4;
                    if let Some(slot) = page.get_mut(dst..dst + w * 4) {
                        slot.copy_from_slice(src);
                    }
                }
                records.push(GlyphRecord {
                    codepoint: g.codepoint,
                    advance: g.advance_em,
                    bearing_x: g.bearing_x_em,
                    bearing_y: g.bearing_y_em,
                    box_x: p.x,
                    box_y: p.y,
                    box_w: r.width,
                    box_h: r.height,
                    page_index: p.page,
                    flags: g.flags,
                });
            }
            None => {
                records.push(GlyphRecord {
                    codepoint: g.codepoint,
                    advance: g.advance_em,
                    bearing_x: g.bearing_x_em,
                    bearing_y: g.bearing_y_em,
                    box_x: p.x,
                    box_y: p.y,
                    box_w: 1,
                    box_h: 1,
                    page_index: p.page,
                    flags: g.flags,
                });
            }
        }
    }
    records.sort_by_key(|r| r.codepoint);

    // 4. Kerning (em), sorted by (left, right).
    let kerning_units = face.kerning_units(codepoints);
    let kerning: Vec<KerningPair> = kerning_units
        .iter()
        .map(|(&(left, right), &adjust)| KerningPair {
            left,
            right,
            adjust: adjust as f32 / info.units_per_em,
        })
        .collect();

    let atlas = Atlas {
        font_id,
        format: 0,
        padding: options.padding,
        texels_per_em: options.texels_per_em,
        ascent: info.ascent_em,
        descent: info.descent_em,
        line_gap: info.line_gap_em,
        cap_height: info.cap_height_em,
        x_height: info.x_height_em,
        units_per_em: info.units_per_em,
        family: info.family,
        weight: info.weight,
        italic: info.italic,
        page_width: options.page_width,
        page_height: options.page_height,
        glyphs: records,
        kerning,
        pages,
    };
    Ok(AtlasResult { atlas, missing })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled Noto Sans Regular.
    const NOTO: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");

    fn sample_codepoints() -> BTreeSet<u32> {
        "Hello, world! 123".chars().map(|c| c as u32).collect()
    }

    #[test]
    fn builds_valid_atlas() {
        let cps = sample_codepoints();
        let result = build_atlas(NOTO, &cps, 0, &AtlasOptions::default()).expect("atlas");
        assert!(result.missing.is_empty());
        let atlas = &result.atlas;
        assert_eq!(atlas.format, 0);
        assert_eq!(atlas.font_id, 0);
        assert!(!atlas.glyphs.is_empty());
        assert_eq!(atlas.glyphs.len(), cps.len());
        // Sorted by codepoint.
        assert!(atlas
            .glyphs
            .windows(2)
            .all(|w| w[0].codepoint < w[1].codepoint));
        // Space has no outline; letters do.
        let space = atlas
            .glyphs
            .iter()
            .find(|g| g.codepoint == ' ' as u32)
            .expect("space");
        assert_ne!(space.flags & glyph_flags::NO_OUTLINE, 0);
        let h = atlas
            .glyphs
            .iter()
            .find(|g| g.codepoint == 'H' as u32)
            .expect("H");
        assert_eq!(h.flags & glyph_flags::NO_OUTLINE, 0);
        assert!(h.box_w > 5 && h.box_h > 5);
        // Boxes inside the page.
        for g in &atlas.glyphs {
            assert!(u32::from(g.box_x) + u32::from(g.box_w) <= atlas.page_width);
            assert!(u32::from(g.box_y) + u32::from(g.box_h) <= atlas.page_height);
        }
        // Page count matches pages vector.
        let max_page = atlas
            .glyphs
            .iter()
            .map(|g| usize::from(g.page_index))
            .max()
            .expect("pages");
        assert_eq!(atlas.pages.len(), max_page + 1);
        for page in &atlas.pages {
            assert_eq!(page.len(), 1024 * 1024 * 4);
        }
    }

    #[test]
    fn determinism() {
        let cps = sample_codepoints();
        let a = build_atlas(NOTO, &cps, 0, &AtlasOptions::default()).expect("a");
        let b = build_atlas(NOTO, &cps, 0, &AtlasOptions::default()).expect("b");
        assert_eq!(a.atlas, b.atlas);
    }

    #[test]
    fn missing_codepoints_reported() {
        // U+10FFFF is unlikely to exist in Noto Sans.
        let mut cps = sample_codepoints();
        cps.insert(0x10FFFF);
        let result = build_atlas(NOTO, &cps, 0, &AtlasOptions::default()).expect("atlas");
        assert!(result.missing.contains(&0x10FFFF));
        assert_eq!(result.atlas.glyphs.len(), cps.len() - 1);
    }

    #[test]
    fn options_validated() {
        let cps = sample_codepoints();
        let opts = AtlasOptions {
            texels_per_em: 0,
            ..AtlasOptions::default()
        };
        assert!(build_atlas(NOTO, &cps, 0, &opts).is_err());
        let opts = AtlasOptions {
            padding: 0,
            ..AtlasOptions::default()
        };
        assert!(build_atlas(NOTO, &cps, 0, &opts).is_err());
    }

    #[test]
    fn invalid_font_rejected() {
        let cps = sample_codepoints();
        assert!(build_atlas(b"garbage", &cps, 0, &AtlasOptions::default()).is_err());
    }

    #[test]
    fn kerning_emitted_sorted() {
        let mut cps = BTreeSet::new();
        for c in "AVToWYa".chars() {
            cps.insert(c as u32);
        }
        let result = build_atlas(NOTO, &cps, 0, &AtlasOptions::default()).expect("atlas");
        assert!(
            result
                .atlas
                .kerning
                .iter()
                .any(|k| k.left == 'A' as u32 && k.right == 'V' as u32 && k.adjust < 0.0),
            "A/V kerning present and negative"
        );
        assert!(result
            .atlas
            .kerning
            .windows(2)
            .all(|w| (w[0].left, w[0].right) < (w[1].left, w[1].right)));
    }

    #[test]
    fn renders_and_reconstructs_matching_reference() {
        // End-to-end rendering validation on real glyphs: MSDF reconstruction
        // vs supersampled direct rasterization, with committed tolerances. Both
        // texel-center and sub-texel (bilinear, the runtime's actual sampling)
        // reconstruction are exercised — the error-correction pass exists
        // precisely for the interpolation case.
        let mut cps = BTreeSet::new();
        for c in "AHOVgpq".chars() {
            cps.insert(c as u32);
        }
        let opts = AtlasOptions {
            texels_per_em: 64 * 1024, // 64 texels/em
            padding: 6,
            ..AtlasOptions::default()
        };
        let result = build_atlas(NOTO, &cps, 0, &opts).expect("atlas");
        let atlas = &result.atlas;
        let face = FontFace::parse(NOTO).expect("font");
        let scale = 64.0 / f64::from(atlas.units_per_em);
        let mut worst_rmse = 0.0_f64;
        let mut worst_max = 0.0_f64;
        for g in &atlas.glyphs {
            if g.flags & glyph_flags::NO_OUTLINE != 0 {
                continue;
            }
            let gid = face.glyph_index(g.codepoint).expect("gid");
            let outline = face.outline(gid, 0.1 / scale).expect("outline");
            let (min, max) = outline.bounds().expect("bounds");
            let x_min_t = (min.x * scale).floor() - f64::from(opts.padding);
            let y_min_t = (-max.y * scale).floor() - f64::from(opts.padding);
            let width = usize::from(g.box_w);
            let height = usize::from(g.box_h);
            // Fetch the glyph's pixels from the page.
            let page = &atlas.pages[usize::from(g.page_index)];
            let mut pixels = vec![0_u8; width * height * 4];
            let page_w = atlas.page_width as usize;
            for row in 0..height {
                let src = (usize::from(g.box_y) + row) * page_w * 4 + usize::from(g.box_x) * 4;
                let dst = row * width * 4;
                if let Some(s) = page.get(src..src + width * 4) {
                    pixels[dst..dst + width * 4].copy_from_slice(s);
                }
            }
            // The reference is each pixel's true coverage; the reconstruction
            // samples the MSDF at the pixel center plus a within-pixel offset
            // (the runtime's bilinear texture sampling).
            let offsets = [
                (0.0, 0.0),
                (0.25, 0.25),
                (-0.25, 0.25),
                (0.25, -0.25),
                (-0.25, -0.25),
            ];
            let reference =
                raster::rasterize_coverage(&outline, scale, x_min_t, y_min_t, width, height, 8);
            for (ox, oy) in offsets {
                let reconstructed =
                    raster::msdf_coverage_offset(&pixels, width, height, 1.0, ox, oy);
                let (rmse, max_err) = raster::coverage_error(&reference, &reconstructed);
                worst_rmse = worst_rmse.max(rmse);
                worst_max = worst_max.max(max_err);
            }
        }
        // Committed tolerances (measured on Noto Sans Regular at 64 t/e over
        // texel centers and within-pixel bilinear samples): the median
        // reconstruction reproduces supersampled coverage with sub-10% RMSE
        // and sub-60% worst-case single-texel error. The residual error
        // concentrates in the one-texel boundary band and at corners — the
        // inherent limit of 8-bit MSDF reconstruction at 1:1 (at rendering
        // scales of 2+ texels per pixel it is sub-pixel).
        assert!(
            worst_rmse < 0.1,
            "rendering validation RMSE {worst_rmse} exceeds committed tolerance"
        );
        assert!(
            worst_max < 0.6,
            "rendering validation max error {worst_max} exceeds committed tolerance"
        );
    }
}
