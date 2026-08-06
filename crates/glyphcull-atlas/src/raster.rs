//! Reference rasterization: the ground truth that MSDF output is validated
//! against (rendering validation, PERFORMANCE.md).
//!
//! A glyph outline is rasterized directly by supersampling: every pixel is
//! subdivided into an `n × n` grid and each subsample's coverage is decided by
//! the non-zero winding rule. This produces an analytic-quality coverage mask
//! with error `O(1/n)` — the reference. The MSDF is then sampled at the same
//! resolution through the normative reconstruction (SPEC.md §2.5: median of the
//! three channels, mapped through a smoothstep with screen-space width), and
//! the two masks are compared (RMSE + max error).
//!
//! Direct grid indexing is used throughout (the documented exception, as in
//! `glyphcull-chunk::graph`): every index is `j * width + i` with `i < width`
//! and `j < height` by loop construction.

#![allow(clippy::indexing_slicing)]

use crate::font::{Edge, GlyphOutline};
use crate::geometry::{edge_winding, Point};

/// Supersampled direct coverage of a glyph at the given texel scale.
///
/// `width`/`height` are the pixel dimensions of the rendered area; `scale` is
/// texels per font unit (the outline is scaled from font units to texels);
/// `x_min_t`/`y_min_t` are the box's top-left texel coordinates in page space
/// (the same convention [`crate::sdf::render_msdf`] uses), so both renderers
/// see identical geometry.
#[must_use]
pub fn rasterize_coverage(
    outline: &GlyphOutline,
    scale: f64,
    x_min_t: f64,
    y_min_t: f64,
    width: usize,
    height: usize,
    supersample: u32,
) -> Vec<f32> {
    let map = |p: Point| Point::new(p.x * scale, p.y * scale);
    let scaled: Vec<Vec<Edge>> = outline
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
        .collect();
    let n = supersample.max(1);
    let step = 1.0 / f64::from(n);
    let mut coverage = vec![0.0_f32; width * height];
    for j in 0..height {
        for i in 0..width {
            let mut hits = 0_u32;
            for sy in 0..n {
                for sx in 0..n {
                    let px = x_min_t + i as f64 + (f64::from(sx) + 0.5) * step;
                    let py = y_min_t + j as f64 + (f64::from(sy) + 0.5) * step;
                    // Page y is down; the outline is y-up.
                    let p = Point::new(px, -py);
                    let mut winding = 0_i32;
                    for contour in &scaled {
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
                    if winding != 0 {
                        hits += 1;
                    }
                }
            }
            coverage[j * width + i] = hits as f32 / (n * n) as f32;
        }
    }
    coverage
}

/// Reconstruct coverage from an MSDF RGBA8 image (SPEC.md §2.5), sampled at a
/// sub-texel offset from the pixel center.
///
/// `pixels` are the raw RGBA8 texels (`width × height`); `width_texels` is the
/// screen-space width of the reconstruction window (1.0 at 1:1). The sample
/// position is `(i + 0.5 + ox, j + 0.5 + oy)` in box-local texel coordinates
/// (the pixel center plus the offset), evaluated by bilinear interpolation of
/// the channels (the runtime's texture sampling), followed by the median-of-
/// three reconstruction.
#[must_use]
pub fn msdf_coverage_offset(
    pixels: &[u8],
    width: usize,
    height: usize,
    width_texels: f64,
    ox: f64,
    oy: f64,
) -> Vec<f32> {
    let half = width_texels / 2.0;
    let mut out = vec![0.0_f32; width * height];
    for j in 0..height {
        for i in 0..width {
            let mut ch = [0.0_f64; 3];
            for (k, slot) in ch.iter_mut().enumerate() {
                *slot = bilinear_channel(
                    pixels,
                    width,
                    height,
                    i as f64 + 0.5 + ox,
                    j as f64 + 0.5 + oy,
                    k,
                ) / 255.0;
            }
            ch.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median = ch[1];
            let t = (median - (0.5 - half)) / (2.0 * half);
            let t = t.clamp(0.0, 1.0);
            let smooth = t * t * (3.0 - 2.0 * t);
            out[j * width + i] = (1.0 - smooth) as f32;
        }
    }
    out
}

/// Bilinear interpolation of one channel at a texel-space position (the texel
/// (i, j) covers [i, i+1) × [j, j+1)).
fn bilinear_channel(
    pixels: &[u8],
    width: usize,
    height: usize,
    px: f64,
    py: f64,
    channel: usize,
) -> f64 {
    let fx = (px - 0.5).floor().max(0.0) as usize;
    let fy = (py - 0.5).floor().max(0.0) as usize;
    let u = (px - 0.5 - fx as f64).clamp(0.0, 1.0);
    let v = (py - 0.5 - fy as f64).clamp(0.0, 1.0);
    let x0 = fx.min(width.saturating_sub(1));
    let y0 = fy.min(height.saturating_sub(1));
    let x1 = (fx + 1).min(width.saturating_sub(1));
    let y1 = (fy + 1).min(height.saturating_sub(1));
    let at = |x: usize, y: usize| f64::from(pixels[(y * width + x) * 4 + channel]);
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * u;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * u;
    top + (bottom - top) * v
}

/// Reconstruct coverage from an MSDF RGBA8 image (SPEC.md §2.5).
///
/// `pixels` are the raw RGBA8 texels (`width × height`); the reconstruction is
/// `coverage = smoothstep(0.5 − w/2, 0.5 + w/2, m)` where `m` is the median of
/// the three channels and `w` is the screen-space width in texels (1.0 at 1:1,
/// the validation scale). Samples at texel centers (see
/// [`msdf_coverage_offset`] for sub-texel sampling).
#[must_use]
pub fn msdf_coverage(pixels: &[u8], width: usize, height: usize, width_texels: f64) -> Vec<f32> {
    msdf_coverage_offset(pixels, width, height, width_texels, 0.0, 0.0)
}

/// Compare two coverage masks; returns (rmse, max error).
#[must_use]
pub fn coverage_error(a: &[f32], b: &[f32]) -> (f64, f64) {
    assert_eq!(a.len(), b.len(), "masks must have equal length");
    let mut sum = 0.0_f64;
    let mut max = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let d = f64::from(x - y).abs();
        sum += d * d;
        if d > max {
            max = d;
        }
    }
    let n = a.len().max(1) as f64;
    ((sum / n).sqrt(), max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Line;
    use crate::sdf::render_msdf;

    fn square_outline() -> GlyphOutline {
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
    fn square_coverage_is_exact() {
        // A square (0,0)-(10,10) at scale 1: coverage is 1 inside, 0 outside.
        let cov = rasterize_coverage(&square_outline(), 1.0, -2.0, -12.0, 15, 15, 8);
        // Interior texel (5,5) center (3.5, -4.5) → inside.
        assert_eq!(cov[7 * 15 + 7], 1.0);
        // Far outside.
        assert_eq!(cov[0], 0.0);
    }

    #[test]
    fn msdf_reconstruction_close_to_reference() {
        let outline = square_outline();
        let scale: f64 = 2.0;
        let padding: f64 = 3.0;
        let x_min = (0.0 * scale).floor() - padding;
        let y_min = (-10.0 * scale).floor() - padding;
        let width = ((10.0 * scale).ceil() + padding - x_min) as usize + 1;
        let height = ((-0.0 * scale).ceil() + padding - y_min) as usize + 1;
        let img = render_msdf(&outline, scale, padding).expect("msdf");
        assert_eq!(usize::from(img.width), width);
        assert_eq!(usize::from(img.height), height);

        let reference = rasterize_coverage(&outline, scale, x_min, y_min, width, height, 8);
        let reconstructed = msdf_coverage(&img.pixels, width, height, 1.0);
        let (rmse, max_err) = coverage_error(&reference, &reconstructed);
        // A straight edge at 1:1 with the linear reconstruction is essentially
        // exact; the median corners contribute small error.
        assert!(rmse < 0.02, "rmse {rmse}");
        assert!(max_err < 0.25, "max err {max_err}");
    }
}
