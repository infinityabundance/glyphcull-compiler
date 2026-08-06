//! Multi-channel signed-distance field generation.
//!
//! For every texel of a glyph's box, the three MSDF channels hold the exact
//! signed distance to the nearest edge of each of three edge *colors*; the
//! runtime's median-of-three reconstruction resolves corners correctly because
//! the two edges meeting at a corner always carry different colors.
//!
//! Distances are exact (see [`crate::geometry`]); the sign comes from the
//! non-zero winding number of the texel center. Values saturate at the padding
//! margin (SPEC.md §2.5: distance in texels = channel − 0.5), which is sound:
//! beyond the margin the reconstruction only ever sees the saturated value.
//!
//! Direct indexing is used on the texel grid and on the fixed-size color
//! tables (the documented exception, as in `glyphcull-chunk::graph`): every
//! index is derived from bounded loop counters (`i < width`, `j < height`) or
//! from `c in 0..3`, so all accesses are provably in range.

#![allow(clippy::indexing_slicing)]

use crate::font::{Edge, GlyphOutline};
use crate::geometry::{distance_to_line, distance_to_quadratic, edge_winding, Point};

/// One rendered glyph: RGBA8 pixels plus the box dimensions in texels.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedGlyph {
    /// Box width in texels.
    pub width: u16,
    /// Box height in texels.
    pub height: u16,
    /// RGBA8 pixels, `width × height × 4` bytes.
    pub pixels: Vec<u8>,
}
/// Three MSDF edge colors. Edges are colored with *two-bit* masks (msdfgen's
/// CYAN/MAGENTA/YELLOW): every edge contributes its distance to two channels,
/// so at any texel near a single edge two channels carry the true distance and
/// the median reconstruction is exact. At a corner the two incident edges share
/// exactly one channel, and the median of the three channels equals the minimum
/// of the two edge distances — the correct pseudo-distance.
pub const COLOR_COUNT: u8 = 3;

/// Channel masks for the three edge colors (bit 0 = R, 1 = G, 2 = B).
/// CYAN = G|B, MAGENTA = R|B, YELLOW = R|G.
const COLOR_MASKS: [u8; 3] = [0b110, 0b101, 0b011];

/// Assign each edge of a contour one of the three two-bit colors so that
/// adjacent edges differ (a 3-coloring of the cycle). With three colors this is
/// always possible: alternate two colors, using the third for the closing edge
/// of an odd-length contour.
fn color_contour(count: usize) -> Vec<u8> {
    let mut colors = Vec::with_capacity(count);
    let first = COLOR_MASKS[0];
    let mut prev = first;
    for i in 0..count {
        if i == 0 {
            colors.push(COLOR_MASKS[0]);
            continue;
        }
        let is_last = i + 1 == count;
        let c = (0..COLOR_COUNT)
            .find(|&c| {
                COLOR_MASKS[usize::from(c)] != prev
                    && (!is_last || COLOR_MASKS[usize::from(c)] != first)
            })
            .unwrap_or(0);
        let color = COLOR_MASKS[usize::from(c)];
        colors.push(color);
        prev = color;
    }
    colors
}

/// The edge bounding box (control polygon for curves — the curve lies inside
/// its convex hull), expanded by `margin`.
fn edge_bounds(edge: Edge, margin: f64) -> (Point, Point) {
    let mut min = edge.start();
    let mut max = edge.start();
    for p in [edge.start(), edge.end()] {
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
    if let Edge::Quad(q) = edge {
        let p = q.c;
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
    (
        Point::new(min.x - margin, min.y - margin),
        Point::new(max.x + margin, max.y + margin),
    )
}

/// The exact unsigned distance from `p` to an edge (in the edge's coordinate
/// space).
fn edge_distance(p: Point, edge: Edge) -> f64 {
    match edge {
        Edge::Line(l) => distance_to_line(p, l),
        Edge::Quad(q) => distance_to_quadratic(p, q),
    }
}

/// Scale an outline from font units to texels.
fn scale_outline(outline: &GlyphOutline, scale: f64) -> Vec<Vec<Edge>> {
    let map = |p: Point| Point::new(p.x * scale, p.y * scale);
    outline
        .contours
        .iter()
        .map(|contour| {
            contour
                .iter()
                .map(|edge| match *edge {
                    Edge::Line(l) => Edge::Line(crate::geometry::Line {
                        a: map(l.a),
                        b: map(l.b),
                    }),
                    Edge::Quad(q) => Edge::Quad(crate::geometry::Quadratic {
                        a: map(q.a),
                        c: map(q.c),
                        b: map(q.b),
                    }),
                })
                .collect()
        })
        .collect()
}

/// Render a glyph's MSDF into a padded box.
///
/// `scale` is texels per font unit; `padding` the SDF margin in texels around
/// the outline (the outline's cubics were already converted to quadratics with
/// the caller's tolerance). Returns `None` for glyphs with no outline.
pub fn render_msdf(outline: &GlyphOutline, scale: f64, padding: f64) -> Option<RenderedGlyph> {
    render_msdf_impl(outline, scale, padding, true)
}

/// The shared implementation; `correct` enables the interpolation-artifact
/// error-correction pass.
fn render_msdf_impl(
    outline: &GlyphOutline,
    scale: f64,
    padding: f64,
    correct: bool,
) -> Option<RenderedGlyph> {
    if outline.contours.is_empty() {
        return None;
    }
    let (min, max) = outline.bounds()?;

    // Box in texels: the outline bbox expanded by padding on every side.
    // Page y grows downward, so the glyph's top (font +y) has the smallest
    // page texel index.
    let x_min_t = (min.x * scale).floor() - padding;
    let x_max_t = (max.x * scale).ceil() + padding;
    let y_min_t = (-max.y * scale).floor() - padding; // top edge (page space)
    let y_max_t = (-min.y * scale).ceil() + padding; // bottom edge (page space)
    let width = (x_max_t - x_min_t) as i64 + 1;
    let height = (y_max_t - y_min_t) as i64 + 1;
    if width <= 0 || height <= 0 {
        return None;
    }
    let width = width as u16;
    let height = height as u16;

    // Scaled outline: distances below are in texels.
    let contours = scale_outline(outline, scale);
    let colored: Vec<(Vec<Edge>, Vec<u8>)> = contours
        .iter()
        .map(|c| (c.clone(), color_contour(c.len())))
        .collect();

    let margin = padding + 1e-9;
    let mut values = vec![0.0_f32; usize::from(width) * usize::from(height) * 3];
    for j in 0..usize::from(height) {
        for i in 0..usize::from(width) {
            // Texel-center → outline-space point. The outline is scaled y-up;
            // page y grows downward, so the texel's y is negated.
            let px = x_min_t + i as f64 + 0.5;
            let py = y_min_t + j as f64 + 0.5;
            let p = Point::new(px, -py);

            // Sign: non-zero winding rule (TrueType).
            let mut winding = 0_i32;
            for contour in &contours {
                for edge in contour {
                    match *edge {
                        Edge::Line(l) => {
                            winding += edge_winding(p, l.a, l.b, None);
                        }
                        Edge::Quad(q) => {
                            winding += edge_winding(p, q.a, q.b, Some(q.c));
                        }
                    }
                }
            }
            let inside = winding != 0;

            let base = (j * usize::from(width) + i) * 3;
            for ci in 0..3 {
                let mask = 1_u8 << ci;
                let mut best = f64::INFINITY;
                for (contour, colors) in &colored {
                    for (edge, &ec) in contour.iter().zip(colors.iter()) {
                        if ec & mask == 0 {
                            continue;
                        }
                        let (lo, hi) = edge_bounds(*edge, margin);
                        if p.x < lo.x || p.x > hi.x || p.y < lo.y || p.y > hi.y {
                            continue;
                        }
                        let d = edge_distance(p, *edge);
                        if d < best {
                            best = d;
                        }
                    }
                }
                if !best.is_finite() {
                    // No edge within the padding margin: saturated.
                    best = margin;
                }
                let signed = if inside { -best } else { best };
                values[base + ci] = (0.5 + signed).clamp(0.0, 1.0) as f32;
            }
        }
    }

    // Error correction (msdfgen's interpolation-artifact pass), then quantize.
    if correct {
        crate::correction::correct_msdf(
            &mut values,
            usize::from(width),
            usize::from(height),
            &colored,
            x_min_t,
            y_min_t,
            &crate::correction::CorrectionConfig::default(),
        );
    }
    let mut pixels = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
    for texel in 0..usize::from(width) * usize::from(height) {
        for ci in 0..3 {
            let byte = (f64::from(values[texel * 3 + ci]) * 255.0).round() as u8;
            pixels.push(byte);
        }
        pixels.push(255); // A channel
    }

    Some(RenderedGlyph {
        width,
        height,
        pixels,
    })
}

/// Render an MSDF without the error-correction pass (used by validation to
/// measure the correction's effect; the corrected path is [`render_msdf`]).
#[doc(hidden)]
pub fn render_msdf_uncorrected(
    outline: &GlyphOutline,
    scale: f64,
    padding: f64,
) -> Option<RenderedGlyph> {
    render_msdf_impl(outline, scale, padding, false)
}

/// Instrumented correction entry point (validation only): builds the colored
/// outline and runs the correction, returning the base and shape-pass flag
/// counts.
#[doc(hidden)]
pub fn correct_msdf_for_test(
    values: &mut [f32],
    width: usize,
    height: usize,
    outline: &GlyphOutline,
    scale: f64,
    x_min_t: f64,
    y_min_t: f64,
) -> (usize, usize) {
    let contours = scale_outline(outline, scale);
    let colored: Vec<crate::correction::ColoredContour> = contours
        .iter()
        .map(|c| (c.clone(), color_contour(c.len())))
        .collect();
    crate::correction::correct_msdf(
        values,
        width,
        height,
        &colored,
        x_min_t,
        y_min_t,
        &crate::correction::CorrectionConfig::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Line;

    fn square_outline() -> GlyphOutline {
        // A unit square from (0,0) to (10,10) (font units).
        GlyphOutline {
            contours: vec![vec![
                Edge::Line(Line {
                    a: Point::new(0.0, 0.0),
                    b: Point::new(10.0, 0.0),
                }),
                Edge::Line(Line {
                    a: Point::new(10.0, 0.0),
                    b: Point::new(10.0, 10.0),
                }),
                Edge::Line(Line {
                    a: Point::new(10.0, 10.0),
                    b: Point::new(0.0, 10.0),
                }),
                Edge::Line(Line {
                    a: Point::new(0.0, 10.0),
                    b: Point::new(0.0, 0.0),
                }),
            ]],
        }
    }

    #[test]
    fn edge_coloring_is_valid() {
        // Every edge carries a two-bit color mask (two channels); adjacent
        // edges must differ.
        for n in 2..=12 {
            let colors = color_contour(n);
            assert_eq!(colors.len(), n);
            for i in 0..n {
                assert_ne!(colors[i], colors[(i + 1) % n], "contour of {n}");
                assert!(colors[i] & !0b111 == 0, "mask bits");
                assert_eq!(colors[i].count_ones(), 2, "two-bit color");
            }
        }
    }

    #[test]
    fn square_msdf_values() {
        // scale 1 texel per font unit, padding 2.
        let img = render_msdf(&square_outline(), 1.0, 2.0).expect("render");
        // Box: 10×10 square + 2 padding each side → 15×15 (texel centers at
        // half-integers cover [−1.5, 12.5]).
        assert_eq!(img.width, 15);
        assert_eq!(img.height, 15);
        // Center texel (inside, far from edges): channels ~ 0.0.
        let off = (7 * usize::from(img.width) + 7) * 4;
        assert!(img.pixels[off] < 10, "inside channel R");
        assert!(img.pixels[off + 1] < 10);
        assert!(img.pixels[off + 2] < 10);
        assert_eq!(img.pixels[off + 3], 255);
        // Corner texel: beyond the padding margin → saturated 1.0.
        assert!(img.pixels[0] > 240, "outside channel R");
        // Gradient across the left edge (font x=0 sits between texel 1
        // (center −0.5) and texel 2 (center +0.5)).
        let mid_y = 7_usize;
        let row = |x: usize| img.pixels[(mid_y * usize::from(img.width) + x) * 4];
        assert!(row(1) > 240, "one texel left of the edge");
        assert!(row(2) < 15, "one texel right of the edge");
    }

    #[test]
    fn msdf_sign_flips_across_edge() {
        let img = render_msdf(&square_outline(), 1.0, 2.0).expect("render");
        // Row through the middle: left of the left edge → outside (1.0);
        // between edges → inside (0.0); right of right edge → outside.
        let mid_y = 7_usize;
        let row = |x: usize| img.pixels[(mid_y * usize::from(img.width) + x) * 4];
        assert!(row(0) > 240); // padding region, outside (distance 1.5)
        assert!(row(6) < 40); // inside (distance 4.5 → saturated 0)
        assert!(row(13) > 240); // right padding, outside (distance 1.5)
    }

    #[test]
    fn empty_outline_is_none() {
        assert!(render_msdf(&GlyphOutline { contours: vec![] }, 1.0, 2.0).is_none());
    }

    #[test]
    fn circle_like_glyph_is_round() {
        // A diamond (rotated square): checks that diagonal distances behave.
        let diamond = GlyphOutline {
            contours: vec![vec![
                Edge::Line(Line {
                    a: Point::new(5.0, 0.0),
                    b: Point::new(10.0, 5.0),
                }),
                Edge::Line(Line {
                    a: Point::new(10.0, 5.0),
                    b: Point::new(5.0, 10.0),
                }),
                Edge::Line(Line {
                    a: Point::new(5.0, 10.0),
                    b: Point::new(0.0, 5.0),
                }),
                Edge::Line(Line {
                    a: Point::new(0.0, 5.0),
                    b: Point::new(5.0, 0.0),
                }),
            ]],
        };
        let img = render_msdf(&diamond, 1.0, 2.0).expect("render");
        assert_eq!(img.width, 15);
        assert_eq!(img.height, 15);
        // Center: inside.
        let cx = 7_usize;
        let off = (7 * usize::from(img.width) + cx) * 4;
        assert!(img.pixels[off] < 40);
    }
}
