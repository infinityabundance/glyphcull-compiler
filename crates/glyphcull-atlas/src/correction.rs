//! MSDF error correction (a faithful translation of msdfgen's
//! `MSDFErrorCorrection`).
//!
//! The median reconstruction is exact at texel centers, but when the runtime
//! samples the atlas with bilinear interpolation, channel crossings between
//! texels can make the interpolated median spike, producing visible artifacts
//! (thin "holes" or "fringes" along edges). The correction pass:
//!
//! 1. **Protects** texels that must not be modified: the 2×2 texels around
//!    every color-changing corner, and texels whose extreme channels form an
//!    edge with a neighbor (equalizing them would move the rendered edge).
//! 2. **Detects** texels whose interpolation with any of their 8 neighbors
//!    produces an artifact (the interpolated median deviates from the range
//!    implied by the neighboring medians), verified against the exact shape
//!    distance when the improvement is real (improvement-ratio test).
//! 3. **Equalizes** flagged texels to a single channel (all three channels set
//!    to the median), which eliminates the interpolation spike locally.
//!
//! The algorithm is deterministic and operates on the float channel values
//! (in the \[0,1] domain with the edge at 0.5) before 8-bit quantization.
//!
//! Direct grid indexing is used throughout (the documented exception, as in
//! `glyphcull-chunk::graph`): every index is `y * width + x` with `x < width`
//! and `y < height` by loop construction, or a checked `base + k` derived from
//! the same bounds, so all accesses are provably in range.

#![allow(clippy::indexing_slicing)]

use crate::font::Edge;
use crate::geometry::{distance_to_line, distance_to_quadratic, Point};

/// Correction parameters (msdfgen's defaults).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CorrectionConfig {
    /// Minimum ratio between the actual and the maximum expected distance delta
    /// for a texel to be considered erroneous.
    pub min_deviation_ratio: f64,
    /// Minimum ratio between the pre- and post-correction distance error.
    pub min_improve_ratio: f64,
}

impl Default for CorrectionConfig {
    fn default() -> Self {
        Self {
            min_deviation_ratio: 10.0 / 9.0,
            min_improve_ratio: 10.0 / 9.0,
        }
    }
}

/// Stencil flags.
const ERROR: u8 = 1;
const PROTECTED: u8 = 2;

/// A small tolerance for the interpolation-ratio tests.
const T_EPSILON: f64 = 0.01;
/// Protection radius tolerance (matches msdfgen).
const PROTECTION_RADIUS_TOLERANCE: f64 = 1.001;

/// A colored contour: edges plus their two-bit color masks.
pub type ColoredContour = (Vec<Edge>, Vec<u8>);

/// The median of three values.
fn median3(a: f32, b: f32, c: f32) -> f32 {
    // max(min(a,b), min(max(a,b), c))
    let x = a.min(b);
    let y = a.max(b).min(c);
    x.max(y)
}

/// The exact signed distance (in texels) from `p` to the outline (non-zero
/// winding rule), mapped to the channel domain (0.5 at the edge).
fn shape_distance(p: Point, contours: &[ColoredContour]) -> f64 {
    let mut best = f64::INFINITY;
    let mut winding = 0_i32;
    for (contour, _) in contours {
        for edge in contour {
            match *edge {
                Edge::Line(l) => {
                    winding += crate::geometry::edge_winding(p, l.a, l.b, None);
                    let d = distance_to_line(p, l);
                    if d < best {
                        best = d;
                    }
                }
                Edge::Quad(q) => {
                    winding += crate::geometry::edge_winding(p, q.a, q.b, Some(q.c));
                    let d = distance_to_quadratic(p, q);
                    if d < best {
                        best = d;
                    }
                }
            }
        }
    }
    let signed = if winding != 0 { -best } else { best };
    (0.5 + signed).clamp(0.0, 1.0)
}

/// Run the correction pass on `values` (`width × height` texels, 3 channels
/// each, in the \[0,1] domain with the edge at 0.5). `contours` is the colored
/// outline scaled to texel units (y-up, global frame); `x_min_t`/`y_min_t` are
/// the box's top-left page-texel coordinates (see [`crate::sdf::render_msdf`]).
pub fn correct_msdf(
    values: &mut [f32],
    width: usize,
    height: usize,
    contours: &[ColoredContour],
    x_min_t: f64,
    y_min_t: f64,
    config: &CorrectionConfig,
) -> (usize, usize) {
    if width == 0 || height == 0 || values.len() != width * height * 3 {
        return (0, 0);
    }
    let mut stencil = vec![0_u8; width * height];

    // 1. Protection: corners (color-changing edge pairs) and edge texels.
    protect_corners(&mut stencil, contours, width, height, x_min_t, y_min_t);
    protect_edges(&mut stencil, values, width, height);

    // 2. Base artifact detection (SDF contents only).
    find_errors(&mut stencil, values, width, height, config, None);
    let base_count = stencil.iter().filter(|&&s| s & ERROR != 0).count();

    // With the shape check enabled, everything is now protected (only inversion
    // artifacts count) and the exact shape distance refines the flags.
    for s in &mut stencil {
        *s |= PROTECTED;
    }
    find_errors(
        &mut stencil,
        values,
        width,
        height,
        config,
        Some((contours, x_min_t, y_min_t)),
    );
    let shape_count = stencil.iter().filter(|&&s| s & ERROR != 0).count();

    // 3. Equalize flagged texels.
    for y in 0..height {
        for x in 0..width {
            if stencil[y * width + x] & ERROR != 0 {
                let base = (y * width + x) * 3;
                let m = median3(values[base], values[base + 1], values[base + 2]);
                values[base] = m;
                values[base + 1] = m;
                values[base + 2] = m;
            }
        }
    }
    (base_count, shape_count)
}

/// Mark the 2×2 texels enveloping every color-changing corner as protected.
/// A corner is a color change between adjacent edges (the shared channel count
/// is a power of two — same-colored edges share both bits).
fn protect_corners(
    stencil: &mut [u8],
    contours: &[ColoredContour],
    width: usize,
    height: usize,
    x_min_t: f64,
    y_min_t: f64,
) {
    for (contour, colors) in contours {
        let count = contour.len();
        if count == 0 {
            continue;
        }
        for i in 0..count {
            let common = colors[(i + count - 1) % count] & colors[i];
            if common & common.wrapping_sub(1) == 0 {
                let p = contour[i].start();
                // Box-local page-texel coordinates of the corner point.
                let px = p.x - x_min_t;
                let py = -p.y - y_min_t;
                let l = (px - 0.5).floor() as i64;
                let b = (py - 0.5).floor() as i64;
                let r = l + 1;
                let t = b + 1;
                for (tx, ty) in [(l, b), (r, b), (l, t), (r, t)] {
                    if tx >= 0 && ty >= 0 && tx < width as i64 && ty < height as i64 {
                        stencil[ty as usize * width + tx as usize] |= PROTECTED;
                    }
                }
            }
        }
    }
}

/// Determine which channels contribute to an edge between texels `a` and `b`.
fn edge_between_texels(a: &[f32], b: &[f32]) -> u8 {
    let mut mask = 0_u8;
    for channel in 0..3 {
        let denom = a[channel] - b[channel];
        if denom.abs() <= f32::EPSILON {
            continue;
        }
        let t = (a[channel] - 0.5) / denom;
        if t > 0.0 && t < 1.0 {
            let c = [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ];
            // Only an edge if the crossing channel is the median there.
            if median3(c[0], c[1], c[2]) == c[channel] {
                mask |= 1 << channel;
            }
        }
    }
    mask
}

/// Protect texels whose non-median channels contribute to an edge with a
/// neighbor.
fn protect_edges(stencil: &mut [u8], values: &[f32], width: usize, height: usize) {
    let radius = PROTECTION_RADIUS_TOLERANCE as f32;
    let at = |x: usize, y: usize| -> [f32; 3] {
        let base = (y * width + x) * 3;
        [values[base], values[base + 1], values[base + 2]]
    };
    let protect = |stencil: &mut [u8], x: usize, y: usize, texel: [f32; 3], m: f32, mask: u8| {
        if mask != 0
            && ((mask & 1 != 0 && texel[0] != m)
                || (mask & 2 != 0 && texel[1] != m)
                || (mask & 4 != 0 && texel[2] != m))
        {
            stencil[y * width + x] |= PROTECTED;
        }
    };

    // Horizontal pairs.
    for y in 0..height {
        for x in 0..width.saturating_sub(1) {
            let a = at(x, y);
            let b = at(x + 1, y);
            let am = median3(a[0], a[1], a[2]);
            let bm = median3(b[0], b[1], b[2]);
            if (am - 0.5).abs() + (bm - 0.5).abs() < radius {
                let mask = edge_between_texels(&a, &b);
                protect(stencil, x, y, a, am, mask);
                protect(stencil, x + 1, y, b, bm, mask);
            }
        }
    }
    // Vertical pairs.
    for y in 0..height.saturating_sub(1) {
        for x in 0..width {
            let a = at(x, y);
            let b = at(x, y + 1);
            let am = median3(a[0], a[1], a[2]);
            let bm = median3(b[0], b[1], b[2]);
            if (am - 0.5).abs() + (bm - 0.5).abs() < radius {
                let mask = edge_between_texels(&a, &b);
                protect(stencil, x, y, a, am, mask);
                protect(stencil, x, y + 1, b, bm, mask);
            }
        }
    }
    // Diagonal pairs.
    for y in 0..height.saturating_sub(1) {
        for x in 0..width.saturating_sub(1) {
            let lb = at(x, y);
            let rb = at(x + 1, y);
            let lt = at(x, y + 1);
            let rt = at(x + 1, y + 1);
            let mlb = median3(lb[0], lb[1], lb[2]);
            let mrb = median3(rb[0], rb[1], rb[2]);
            let mlt = median3(lt[0], lt[1], lt[2]);
            let mrt = median3(rt[0], rt[1], rt[2]);
            if (mlb - 0.5).abs() + (mrt - 0.5).abs() < radius {
                let mask = edge_between_texels(&lb, &rt);
                protect(stencil, x, y, lb, mlb, mask);
                protect(stencil, x + 1, y + 1, rt, mrt, mask);
            }
            if (mrb - 0.5).abs() + (mlt - 0.5).abs() < radius {
                let mask = edge_between_texels(&rb, &lt);
                protect(stencil, x + 1, y, rb, mrb, mask);
                protect(stencil, x, y + 1, lt, mlt, mask);
            }
        }
    }
}

/// The interpolated median between texels `a` and `b` at ratio `t`.
fn interpolated_median(a: &[f32; 3], b: &[f32; 3], t: f64) -> f32 {
    let c = [
        a[0] + (b[0] - a[0]) * t as f32,
        a[1] + (b[1] - a[1]) * t as f32,
        a[2] + (b[2] - a[2]) * t as f32,
    ];
    median3(c[0], c[1], c[2])
}

/// The interpolated median under a bilinear model with constant `a`, linear
/// `l`, and quadratic `q` channel terms at ratio `t` (diagonal case).
fn interpolated_median_quad(a: &[f32; 3], l: &[f32; 3], q: &[f32; 3], t: f64) -> f32 {
    let c = [
        t as f32 * (t as f32 * q[0] + l[0]) + a[0],
        t as f32 * (t as f32 * q[1] + l[1]) + a[1],
        t as f32 * (t as f32 * q[2] + l[2]) + a[2],
    ];
    median3(c[0], c[1], c[2])
}

/// The range-test parameters shared by the artifact classifiers (msdfgen's
/// `BaseArtifactClassifier`): the expected interpolation span and whether the
/// texel is protected.
#[derive(Clone, Copy)]
struct RangeParams {
    /// The expected value-domain span of one texel (minDeviationRatio × 1).
    span: f64,
    /// Protected texels only count inversion artifacts.
    protected: bool,
}

impl RangeParams {
    /// Flags a candidate whose interpolated median deviates from the range
    /// implied by the boundaries: 0 = none, 1 = candidate, 3 = artifact.
    fn range_test(&self, at: f64, bt: f64, xt: f64, am: f32, bm: f32, xm: f32) -> u8 {
        let inversion = (am > 0.5 && bm > 0.5 && xm <= 0.5) || (am < 0.5 && bm < 0.5 && xm >= 0.5);
        let outside_bounds = !self.protected && median3(am, bm, xm) != xm;
        if inversion || outside_bounds {
            let ax_span = ((xt - at) * self.span) as f32;
            let bx_span = ((bt - xt) * self.span) as f32;
            let in_range = xm >= am - ax_span
                && xm <= am + ax_span
                && xm >= bm - bx_span
                && xm <= bm + bx_span;
            if !in_range {
                return 3; // CANDIDATE | ARTIFACT
            }
            return 1; // CANDIDATE
        }
        0
    }
}

/// Detect a linear interpolation artifact between texels `a` and `b`.
///
/// `classify` is called with the interpolation ratio where a channel pair
/// meets and the interpolated median; it returns whether the artifact is real.
fn has_linear_artifact(
    am: f32,
    a: &[f32; 3],
    b: &[f32; 3],
    params: RangeParams,
    classify: &dyn Fn(f64, f32, u8) -> bool,
) -> bool {
    let bm = median3(b[0], b[1], b[2]);
    // Only report artifacts for the texel farther from the edge.
    if (am - 0.5).abs() < (bm - 0.5).abs() {
        return false;
    }
    for (i, j) in [(0usize, 1usize), (2usize, 1usize), (0usize, 2usize)] {
        let d_a = a[i] - a[j];
        let d_b = b[i] - b[j];
        let denom = d_a - d_b;
        if denom.abs() <= f32::EPSILON {
            continue;
        }
        let t = f64::from(d_a / denom);
        if t > T_EPSILON && t < 1.0 - T_EPSILON {
            let xm = interpolated_median(a, b, t);
            let flags = params.range_test(0.0, 1.0, t, am, bm, xm);
            if classify(t, xm, flags) {
                return true;
            }
        }
    }
    false
}

/// Detect a bilinear interpolation artifact between diagonally adjacent texels
/// `a` and `d` (with `b`, `c` forming the other diagonal).
fn has_diagonal_artifact(
    am: f32,
    texels: [[f32; 3]; 4],
    params: RangeParams,
    classify: &dyn Fn(f64, f32, u8) -> bool,
) -> bool {
    let a = &texels[0];
    let b = &texels[1];
    let c = &texels[2];
    let d = &texels[3];
    let dm = median3(d[0], d[1], d[2]);
    if (am - 0.5).abs() < (dm - 0.5).abs() {
        return false;
    }
    // Bilinear interpolation terms along the diagonal (msdfgen's convention):
    // value(s) = a + l·s + q·s² with s the ratio along the diagonal.
    let mut l = [0.0_f32; 3];
    let mut q = [0.0_f32; 3];
    let mut t_ex = [0.0_f64; 3];
    for ch in 0..3 {
        let abc = a[ch] - b[ch] - c[ch];
        l[ch] = -a[ch] - abc;
        q[ch] = d[ch] + abc;
        t_ex[ch] = -0.5 * f64::from(l[ch]) / f64::from(q[ch]);
    }
    for (i, j) in [(0usize, 1usize), (2usize, 1usize), (0usize, 2usize)] {
        let d_a = a[i] - a[j];
        let d_bc = b[i] - b[j] + c[i] - c[j];
        let d_d = d[i] - d[j];
        // Find the ratios where the two channels are equal:
        // (dD − dBC + dA)·s² + (dBC − 2dA)·s + dA = 0.
        let mut roots = [0.0_f64; 2];
        let solutions = solve_quadratic(
            &mut roots,
            f64::from(d_d - d_bc + d_a),
            f64::from(d_bc - 2.0 * d_a),
            f64::from(d_a),
        );
        for &t in roots.iter().take(solutions) {
            if t > T_EPSILON && t < 1.0 - T_EPSILON {
                let xm = interpolated_median_quad(a, &l, &q, t);
                let mut flags = params.range_test(0.0, 1.0, t, am, dm, xm);
                // Check against the interpolated medians at the local extremes.
                for t_end in [t_ex[i], t_ex[j]] {
                    if t_end > 0.0 && t_end < 1.0 {
                        if t_end > t {
                            let hi_m = interpolated_median_quad(a, &l, &q, t_end);
                            flags |= params.range_test(0.0, t_end, t, am, hi_m, xm);
                        } else {
                            let lo_m = interpolated_median_quad(a, &l, &q, t_end);
                            flags |= params.range_test(t_end, 1.0, t, lo_m, dm, xm);
                        }
                    }
                }
                if classify(t, xm, flags) {
                    return true;
                }
            }
        }
    }
    false
}

/// Solve a quadratic; returns the number of real roots (sorted ascending).
fn solve_quadratic(roots: &mut [f64; 2], a: f64, b: f64, c: f64) -> usize {
    if a.abs() <= f64::EPSILON {
        if b.abs() <= f64::EPSILON {
            return 0;
        }
        roots[0] = -c / b;
        return 1;
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return 0;
    }
    let sq = disc.sqrt();
    roots[0] = (-b - sq) / (2.0 * a);
    roots[1] = (-b + sq) / (2.0 * a);
    if roots[0] > roots[1] {
        roots.swap(0, 1);
    }
    if sq.abs() <= f64::EPSILON {
        1
    } else {
        2
    }
}

/// Bilinear interpolation of the channel values at a texel-space position.
/// The texel (i, j) covers [i, i+1) × [j, j+1) in texel space, so a sample at
/// (x + 0.5 + tx, y + 0.5 + ty) blends the four surrounding texels with the
/// standard bilinear weights.
fn bilinear_at(values: &[f32], width: usize, height: usize, px: f64, py: f64) -> [f32; 3] {
    let fx = (px - 0.5).floor().max(0.0) as usize;
    let fy = (py - 0.5).floor().max(0.0) as usize;
    let u = (px - 0.5 - fx as f64).clamp(0.0, 1.0) as f32;
    let v = (py - 0.5 - fy as f64).clamp(0.0, 1.0) as f32;
    let x0 = fx.min(width.saturating_sub(1));
    let y0 = fy.min(height.saturating_sub(1));
    let x1 = (fx + 1).min(width.saturating_sub(1));
    let y1 = (fy + 1).min(height.saturating_sub(1));
    let mut out = [0.0_f32; 3];
    for ch in 0..3 {
        let a = values[(y0 * width + x0) * 3 + ch];
        let b = values[(y0 * width + x1) * 3 + ch];
        let c = values[(y1 * width + x0) * 3 + ch];
        let d = values[(y1 * width + x1) * 3 + ch];
        let top = a + (b - a) * u;
        let bottom = c + (d - c) * u;
        out[ch] = top + (bottom - top) * v;
    }
    out
}

/// Detect and flag erroneous texels. When `contours` is provided, candidates
/// are additionally verified against the exact shape distance.
fn find_errors(
    stencil: &mut [u8],
    values: &[f32],
    width: usize,
    height: usize,
    config: &CorrectionConfig,
    shape: Option<(&[ColoredContour], f64, f64)>,
) {
    let at = |x: usize, y: usize| -> [f32; 3] {
        let base = (y * width + x) * 3;
        [values[base], values[base + 1], values[base + 2]]
    };
    for y in 0..height {
        for x in 0..width {
            let c = at(x, y);
            let cm = median3(c[0], c[1], c[2]);
            let protected = stencil[y * width + x] & PROTECTED != 0;
            let params = RangeParams {
                span: config.min_deviation_ratio,
                protected,
            };

            // Evaluate one linear artifact test with the given classifier.
            let linear = |b: [f32; 3], dx: f64, dy: f64| -> bool {
                let classify = |t: f64, _xm: f32, flags: u8| -> bool {
                    if flags & 2 != 0 {
                        return true;
                    }
                    if flags & 1 == 0 {
                        return false;
                    }
                    shape_check(t, dx, dy, x, y, c, cm, values, width, height, config, shape)
                };
                has_linear_artifact(cm, &c, &b, params, &classify)
            };
            let diagonal = |b: [f32; 3], c3: [f32; 3], d: [f32; 3], dx: f64, dy: f64| -> bool {
                let classify = |t: f64, _xm: f32, flags: u8| -> bool {
                    if flags & 2 != 0 {
                        return true;
                    }
                    if flags & 1 == 0 {
                        return false;
                    }
                    shape_check(t, dx, dy, x, y, c, cm, values, width, height, config, shape)
                };
                has_diagonal_artifact(cm, [c, b, c3, d], params, &classify)
            };

            let flagged = (x > 0 && linear(at(x - 1, y), -1.0, 0.0))
                || (y > 0 && linear(at(x, y - 1), 0.0, -1.0))
                || (x + 1 < width && linear(at(x + 1, y), 1.0, 0.0))
                || (y + 1 < height && linear(at(x, y + 1), 0.0, 1.0))
                || (x > 0
                    && y > 0
                    && diagonal(at(x - 1, y), at(x, y - 1), at(x - 1, y - 1), -1.0, -1.0))
                || (x + 1 < width
                    && y > 0
                    && diagonal(at(x + 1, y), at(x, y - 1), at(x + 1, y - 1), 1.0, -1.0))
                || (x > 0
                    && y + 1 < height
                    && diagonal(at(x - 1, y), at(x, y + 1), at(x - 1, y + 1), -1.0, 1.0))
                || (x + 1 < width
                    && y + 1 < height
                    && diagonal(at(x + 1, y), at(x, y + 1), at(x + 1, y + 1), 1.0, 1.0));
            if flagged {
                stencil[y * width + x] |= ERROR;
            }
        }
    }
}

/// The shape-distance improvement test: equalizing the texel must reduce the
/// reconstruction error at the artifact point (compared to the exact shape
/// distance).
#[allow(clippy::too_many_arguments)]
fn shape_check(
    t: f64,
    dx: f64,
    dy: f64,
    x: usize,
    y: usize,
    texel: [f32; 3],
    texel_median: f32,
    values: &[f32],
    width: usize,
    height: usize,
    config: &CorrectionConfig,
    shape: Option<(&[ColoredContour], f64, f64)>,
) -> bool {
    let Some((contours, x_min_t, y_min_t)) = shape else {
        return false;
    };
    let tx = t * dx;
    let ty = t * dy;
    // The artifact point in box-local page-texel coordinates.
    let px = x as f64 + 0.5 + tx;
    let py = y as f64 + 0.5 + ty;
    let old_msd = bilinear_at(values, width, height, px, py);
    let a_weight = ((1.0 - tx.abs()) * (1.0 - ty.abs())) as f32;
    // The channels after the current texel is equalized to its median.
    let new_msd = [
        old_msd[0] + a_weight * (texel_median - texel[0]),
        old_msd[1] + a_weight * (texel_median - texel[1]),
        old_msd[2] + a_weight * (texel_median - texel[2]),
    ];
    let old_psd = median3(old_msd[0], old_msd[1], old_msd[2]);
    let new_psd = median3(new_msd[0], new_msd[1], new_msd[2]);
    // The exact shape distance at the artifact point (outline y-up).
    let ref_psd = shape_distance(Point::new(px + x_min_t, -(py + y_min_t)), contours) as f32;
    config.min_improve_ratio * f64::from((new_psd - ref_psd).abs())
        < f64::from((old_psd - ref_psd).abs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Line;

    /// Build a tiny colored square outline (texel units) for shape checks.
    fn square_contours() -> Vec<ColoredContour> {
        let edges = vec![
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
        ];
        vec![(edges, vec![0b110, 0b101, 0b011, 0b110])]
    }

    #[test]
    fn well_behaved_grid_is_untouched() {
        // A flat grid (all texels deep inside): the pass must not modify it.
        let mut values = vec![0.1_f32; 12 * 12 * 3];
        let contours = square_contours();
        let before = values.clone();
        let (base, shape) = correct_msdf(
            &mut values,
            12,
            12,
            &contours,
            -3.0,
            -13.0,
            &CorrectionConfig::default(),
        );
        assert_eq!(values, before);
        assert_eq!(base, 0);
        assert_eq!(shape, 0);
    }

    #[test]
    fn idempotent() {
        let mut values = vec![0.5_f32; 12 * 12 * 3];
        // Introduce a mild channel imbalance on a few texels.
        values[0] = 0.2;
        values[3 * 12 * 3 + 1] = 0.8;
        let contours = square_contours();
        let mut a = values.clone();
        let mut b = values.clone();
        let _ = correct_msdf(
            &mut a,
            12,
            12,
            &contours,
            -3.0,
            -13.0,
            &CorrectionConfig::default(),
        );
        let _ = correct_msdf(
            &mut b,
            12,
            12,
            &contours,
            -3.0,
            -13.0,
            &CorrectionConfig::default(),
        );
        assert_eq!(a, b);
        // No value leaves the channel domain.
        for v in &a {
            assert!((0.0..=1.0).contains(v));
        }
    }

    #[test]
    fn inversion_artifact_is_detected() {
        // Two texels whose channel crossing makes the interpolated median dip
        // to 0.5 while both boundary medians are above 0.5 (an inversion
        // artifact). The base pass must flag the farther texel.
        // a = (0.4, 0.6, 0.9), b = (0.6, 0.4, 0.9): the (0,1) crossing at
        // t = 0.5 has median 0.5; am = bm = 0.6.
        let mut values = vec![0.9_f32; 4 * 4 * 3];
        values[0] = 0.4;
        values[1] = 0.6;
        values[3] = 0.6;
        values[4] = 0.4;
        let contours = square_contours();
        let (base, _shape) = correct_msdf(
            &mut values,
            4,
            4,
            &contours,
            -3.0,
            -13.0,
            &CorrectionConfig::default(),
        );
        // The texel farther from 0.5 (texel 0: |0.4 − 0.5| = 0.1 vs texel 1:
        // |0.6 − 0.5| = 0.1 — equal, so the first texel examined is flagged
        // when the pair is scanned from it). At least one of the two texels
        // must be equalized (single-channel).
        let equalized = values.chunks(3).any(|t| {
            let m = median3(t[0], t[1], t[2]);
            (t[0] - m).abs() < 1e-5 && (t[1] - m).abs() < 1e-5 && (t[2] - m).abs() < 1e-5
        });
        let _ = base;
        assert!(
            equalized,
            "the inversion texel must be equalized: {values:?}"
        );
    }
}
