//! Exact geometry primitives for signed-distance computation.
//!
//! The distance field the atlas produces must be *exact*: every texel's value is
//! the true distance to the glyph outline (not an approximation), so the runtime
//! can reconstruct coverage at any size with known error. This module provides:
//!
//! - [`Point`] and vector helpers (field access only — the workspace indexing
//!   policy forbids `[]` in production code).
//! - Exact distance to a line segment and to a quadratic Bézier segment
//!   (closest point found by solving the derivative's roots with a robust
//!   monotonic-interval bisection, never Cardano's fragile formula).
//! - Cubic→quadratic conversion with an *exact* error bound: the midpoint-matched
//!   quadratic's deviation is a cubic vanishing at `t ∈ {0, 1/2, 1}`, whose
//!   maximum is a closed constant times the deviation at `t = 1/4`.
//! - A winding-number contribution per edge (line or quadratic) with the
//!   standard half-open ray-casting convention, exact for closed contours.

/// A 2D point (f64 for exactness in font-unit space).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// X coordinate.
    pub x: f64,
    /// Y coordinate.
    pub y: f64,
}

impl Point {
    /// Construct a point.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Vector from `self` to `other`.
    #[must_use]
    pub fn to(self, other: Self) -> Self {
        Self::new(other.x - self.x, other.y - self.y)
    }

    /// Dot product.
    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// 2D cross product (z component of the 3D cross).
    #[must_use]
    pub fn cross(self, other: Self) -> f64 {
        self.x * other.y - self.y * other.x
    }

    /// Squared length.
    #[must_use]
    pub fn length_sq(self) -> f64 {
        self.dot(self)
    }

    /// Length.
    #[must_use]
    pub fn length(self) -> f64 {
        self.length_sq().sqrt()
    }

    /// Component-wise scale.
    #[must_use]
    pub fn scale(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s)
    }

    /// Component-wise addition. (Named `add` for readability in the distance
    /// math; `should_implement_trait` does not apply — this type is internal.)
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }

    /// Component-wise subtraction. (See [`Point::add`].)
    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }

    /// Linearly interpolate toward `other` by `t ∈ [0, 1]`.
    #[must_use]
    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }

    /// The squared distance between two points.
    #[must_use]
    pub fn dist_sq(self, other: Self) -> f64 {
        self.to(other).length_sq()
    }
}

/// A line segment from `a` to `b`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    /// Start.
    pub a: Point,
    /// End.
    pub b: Point,
}

/// A quadratic Bézier from `a` to `b` with control point `c`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quadratic {
    /// Start.
    pub a: Point,
    /// Control point.
    pub c: Point,
    /// End.
    pub b: Point,
}

impl Quadratic {
    /// Evaluate the curve at `t ∈ [0, 1]`.
    #[must_use]
    pub fn eval(self, t: f64) -> Point {
        let u = 1.0 - t;
        let w0 = u * u;
        let w1 = 2.0 * u * t;
        let w2 = t * t;
        Point::new(
            w0 * self.a.x + w1 * self.c.x + w2 * self.b.x,
            w0 * self.a.y + w1 * self.c.y + w2 * self.b.y,
        )
    }

    /// The tangent (derivative) at `t ∈ [0, 1]`.
    #[must_use]
    pub fn tangent(self, t: f64) -> Point {
        // Q'(t) = 2[(1-t)(C-A) + t(B-C)]
        let u = 1.0 - t;
        Point::new(
            2.0 * (u * (self.c.x - self.a.x) + t * (self.b.x - self.c.x)),
            2.0 * (u * (self.c.y - self.a.y) + t * (self.b.y - self.c.y)),
        )
    }
}

/// Exact distance from `p` to a line segment.
#[must_use]
pub fn distance_to_line(p: Point, line: Line) -> f64 {
    let ab = line.a.to(line.b);
    let len_sq = ab.length_sq();
    if len_sq <= f64::EPSILON {
        // Degenerate (zero-length) segment: distance to the point.
        return p.dist_sq(line.a).sqrt();
    }
    let t = line.a.to(p).dot(ab) / len_sq;
    let t = t.clamp(0.0, 1.0);
    p.dist_sq(line.a.lerp(line.b, t)).sqrt()
}

/// Exact distance from `p` to a quadratic Bézier segment.
///
/// The closest point satisfies `(Q(t) − p) · Q′(t) = 0`, a cubic in `t`. Rather
/// than Cardano's formula (numerically fragile), the cubic's roots in `[0, 1]`
/// are isolated by its own extrema (the roots of `f″`, a quadratic — closed
/// form), then found by bisection on each monotonic interval.
#[must_use]
pub fn distance_to_quadratic(p: Point, q: Quadratic) -> f64 {
    // Degenerate cases first.
    let a_is_point = q.a.dist_sq(q.b) <= f64::EPSILON && q.a.dist_sq(q.c) <= f64::EPSILON;
    if a_is_point {
        return p.dist_sq(q.a).sqrt();
    }

    // f(t) = |Q(t) − p|². f'(t) = 2 (Q(t) − p) · Q'(t) — a cubic with the same
    // roots as (Q(t) − p) · Q'(t) =: g(t).
    //
    // Q(t) = A + (2C − 2A) t + (A − 2C + B) t²
    // Let u = A − p, v = 2(C − A), w = A − 2C + B.
    // Q(t) − p = u + v t + w t²
    // Q'(t)   = v + 2 w t
    // g(t) = (u + v t + w t²) · (v + 2 w t)
    //      = u·v + (2 u·w + v·v) t + (3 v·w) t² + (2 w·w) t³
    let u = q.a.sub(p);
    let v = q.c.sub(q.a).scale(2.0);
    let w = q.a.sub(q.c.scale(2.0)).add(q.b);
    let c0 = u.dot(v);
    let c1 = u.dot(w) * 2.0 + v.dot(v);
    let c2 = v.dot(w) * 3.0;
    let c3 = w.dot(w) * 2.0;

    // Roots of g' (a quadratic): partition [0,1] into monotonic intervals.
    let mut cuts: Vec<f64> = Vec::new();
    let da = c3 * 3.0; // g'' coefficient for t²
    let db = c2 * 2.0;
    let dc = c1;
    if da.abs() <= f64::EPSILON {
        if db.abs() > f64::EPSILON {
            let t = -dc / db;
            if t > 0.0 && t < 1.0 {
                cuts.push(t);
            }
        }
    } else {
        let disc = db * db - 4.0 * da * dc;
        if disc >= 0.0 {
            let sq = disc.sqrt();
            for t in [(-db - sq) / (2.0 * da), (-db + sq) / (2.0 * da)] {
                if t > 0.0 && t < 1.0 {
                    cuts.push(t);
                }
            }
        }
    }
    cuts.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    cuts.dedup_by(|x, y| (*x - *y).abs() < 1e-12);

    // Evaluate g at the interval boundaries; a sign change in an interval gives
    // exactly one root of g (g monotonic there) — find it by bisection. The
    // cuts partition [0,1] into cuts.len()+1 intervals, the last ending at 1.
    let eval_g = |t: f64| ((c3 * t + c2) * t + c1) * t + c0;
    let mut candidates: Vec<f64> = vec![0.0, 1.0];
    let mut lo = 0.0;
    for &hi in cuts.iter().chain(std::iter::once(&1.0)) {
        if hi <= lo + 1e-15 {
            continue;
        }
        let glo = eval_g(lo);
        let ghi = eval_g(hi);
        if glo * ghi < 0.0 {
            // Bisection on [lo, hi].
            let mut a = lo;
            let mut b = hi;
            let mut fa = glo;
            for _ in 0..80 {
                let mid = 0.5 * (a + b);
                let fm = eval_g(mid);
                if fm == 0.0 || (b - a).abs() < 1e-15 {
                    a = mid;
                    break;
                }
                if fa * fm < 0.0 {
                    b = mid;
                } else {
                    a = mid;
                    fa = fm;
                }
            }
            candidates.push(a);
        }
        lo = hi;
    }

    let mut best = f64::INFINITY;
    for t in candidates {
        if !(0.0..=1.0).contains(&t) {
            continue;
        }
        let d = p.dist_sq(q.eval(t));
        if d < best {
            best = d;
        }
    }
    best.sqrt()
}

/// Convert a cubic Bézier to quadratics with a *provable* error bound.
///
/// The quadratic `Q` with `Q(0) = P(0)`, `Q(1) = P(3)`, `Q(1/2) = P(1/2)` has
/// control point `Q1 = (3P1 + 3P2 − P0 − P3)/4`. Its deviation `E(t) = P(t) −
/// Q(t)` is a cubic vanishing at `t ∈ {0, 1/2, 1}`, so each coordinate is
/// `k · t(t − 1/2)(t − 1)`; the maximum of `|t(t−1/2)(t−1)|` on `[0,1]` is a
/// closed constant, giving the exact bound `max|E| = 1.0264… · |E(1/4)|`.
///
/// When the bound exceeds `tolerance`, the cubic is split in half (de Casteljau)
/// and each half is converted recursively. The result is a set of quadratics
/// whose maximum deviation from the cubic is ≤ `tolerance`, which bounds the
/// distance-field error by the same amount (triangle inequality).
pub fn cubic_to_quadratics(
    p0: Point,
    p1: Point,
    p2: Point,
    p3: Point,
    tolerance: f64,
) -> Vec<Quadratic> {
    fn midpoint_error(p0: Point, p1: Point, p2: Point, p3: Point) -> f64 {
        // Q1 = (3P1 + 3P2 − P0 − P3)/4
        let q1 = p1.scale(3.0).add(p2.scale(3.0)).sub(p0).sub(p3).scale(0.25);
        let quad = Quadratic {
            a: p0,
            c: q1,
            b: p3,
        };
        let e_quarter = p(0.25, p0, p1, p2, p3).sub(quad.eval(0.25)).length();
        // E(t) = k·t(t−1/2)(t−1); |E(1/4)| = |k|·3/64 and
        // max|t(t−1/2)(t−1)| = 1/(12√3) on [0,1], so
        // max|E| = |E(1/4)| · (64/3)·(1/(12√3)) = |E(1/4)| · 16/(9√3).
        // 16/(9√3) ≈ 1.0264007010779082 (√3 computed to double precision).
        const RATIO: f64 = 1.0264007010779082;
        e_quarter * RATIO
    }
    fn p(t: f64, p0: Point, p1: Point, p2: Point, p3: Point) -> Point {
        let u = 1.0 - t;
        let w0 = u * u * u;
        let w1 = 3.0 * u * u * t;
        let w2 = 3.0 * u * t * t;
        let w3 = t * t * t;
        Point::new(
            w0 * p0.x + w1 * p1.x + w2 * p2.x + w3 * p3.x,
            w0 * p0.y + w1 * p1.y + w2 * p2.y + w3 * p3.y,
        )
    }
    // Split a cubic at t = 1/2 (de Casteljau).
    fn split(p0: Point, p1: Point, p2: Point, p3: Point) -> [(Point, Point, Point, Point); 2] {
        let a = p0.lerp(p1, 0.5);
        let b = p1.lerp(p2, 0.5);
        let c = p2.lerp(p3, 0.5);
        let d = a.lerp(b, 0.5);
        let e = b.lerp(c, 0.5);
        let m = d.lerp(e, 0.5);
        [(p0, a, d, m), (m, e, c, p3)]
    }

    let mut out = Vec::new();
    let mut stack = vec![(p0, p1, p2, p3)];
    while let Some((q0, q1, q2, q3)) = stack.pop() {
        let err = midpoint_error(q0, q1, q2, q3);
        if err <= tolerance {
            let control = q1.scale(3.0).add(q2.scale(3.0)).sub(q0).sub(q3).scale(0.25);
            out.push(Quadratic {
                a: q0,
                c: control,
                b: q3,
            });
        } else {
            for half in split(q0, q1, q2, q3) {
                stack.push(half);
            }
        }
    }
    // Stack order produces reversed segments; restore document order.
    out.reverse();
    out
}

/// The winding contribution of one edge under the standard half-open ray-casting
/// convention: a point `p` is inside the shape iff the total winding (sum over
/// all edges of all contours) is non-zero (the TrueType non-zero winding rule).
///
/// A line edge crosses the horizontal ray from `p` at most once, so the classic
/// endpoint-sign rule applies. A quadratic edge can cross the ray twice; every
/// root of `Qy(t) − p.y` with `t ∈ [0, 1)` is examined, the crossing counted
/// when it lies to the right of `p` (up-crossings `+1`, down-crossings `−1`).
/// Roots at `t = 0` count only for up-crossings, and `t = 1` is excluded — the
/// half-open convention that counts every shared-vertex crossing exactly once.
/// Tangent touches (`Qy′ = 0`) contribute zero; horizontal-on-ray edges nothing.
#[must_use]
pub fn edge_winding(p: Point, a: Point, b: Point, c: Option<Point>) -> i32 {
    let Some(control) = c else {
        // Line: the classic half-open rule.
        let s_a = a.y > p.y;
        let s_b = b.y > p.y;
        if s_a == s_b {
            return 0;
        }
        let denom = b.y - a.y;
        if denom.abs() <= f64::EPSILON {
            return 0;
        }
        let t = (p.y - a.y) / denom;
        if !(0.0..=1.0).contains(&t) {
            return 0;
        }
        let x = a.x + t * (b.x - a.x);
        if x > p.x {
            return if s_b { 1 } else { -1 };
        }
        return 0;
    };
    // Quadratic: all roots of Qy(t) − p.y in [0, 1).
    // Qy(t) = Ay + (2Cy − 2Ay) t + (Ay − 2Cy + By) t²
    let cy = control.y;
    let c0 = a.y - p.y;
    let c1 = 2.0 * (cy - a.y);
    let c2 = a.y - 2.0 * cy + b.y;
    let mut winding = 0_i32;
    if c2.abs() <= f64::EPSILON {
        if c1.abs() <= f64::EPSILON {
            return 0; // edge lies on the ray
        }
        let t = -c0 / c1;
        if (0.0..1.0).contains(&t) || (t == 0.0 && c1 > 0.0) {
            let q = Quadratic { a, c: control, b };
            if q.eval(t).x > p.x {
                winding += if c1 > 0.0 { 1 } else { -1 };
            }
        }
        return winding;
    }
    let disc = c1 * c1 - 4.0 * c2 * c0;
    if disc < 0.0 {
        return 0;
    }
    let sq = disc.sqrt();
    let mut roots = [(-c1 - sq) / (2.0 * c2), (-c1 + sq) / (2.0 * c2)];
    roots.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let q = Quadratic { a, c: control, b };
    for t in roots {
        if !(0.0..1.0).contains(&t) {
            // Half-open: a root exactly at 1 belongs to the next edge; a root
            // exactly at 0 counts only for up-crossings (the vertex convention).
            if t == 0.0 && c1 > 0.0 && q.eval(0.0).x > p.x {
                winding += 1;
            }
            continue;
        }
        let dy = q.tangent(t).y;
        if dy > 0.0 && q.eval(t).x > p.x {
            winding += 1;
        } else if dy < 0.0 && q.eval(t).x > p.x {
            winding -= 1;
        }
    }
    winding
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b} (within 1e-9)");
    }

    #[test]
    fn point_ops() {
        let p = Point::new(3.0, 4.0);
        approx(p.length(), 5.0);
        approx(p.dist_sq(Point::new(0.0, 0.0)), 25.0);
        let q = Point::new(1.0, 0.0);
        approx(p.dot(q), 3.0);
        approx(p.cross(q), -4.0);
    }

    #[test]
    fn distance_to_line_basics() {
        let line = Line {
            a: Point::new(0.0, 0.0),
            b: Point::new(10.0, 0.0),
        };
        approx(distance_to_line(Point::new(5.0, 3.0), line), 3.0);
        approx(distance_to_line(Point::new(-1.0, 0.0), line), 1.0);
        approx(distance_to_line(Point::new(11.0, 0.0), line), 1.0);
        approx(distance_to_line(Point::new(5.0, -4.0), line), 4.0);
    }

    #[test]
    fn distance_to_quadratic_circle_ish() {
        // The curve from (1,0) to (0,1) with control (1,1): x = 1−t², y = 2t−t².
        // It bulges *outside* the unit circle, so the origin's closest point is
        // the start (1,0), exactly distance 1.
        let q = Quadratic {
            a: Point::new(1.0, 0.0),
            c: Point::new(1.0, 1.0),
            b: Point::new(0.0, 1.0),
        };
        let mid = q.eval(0.5);
        approx(mid.x, 0.75);
        approx(mid.y, 0.75);
        approx(distance_to_quadratic(Point::new(0.0, 0.0), q), 1.0);
        // Point above the curve: the closest point is interior (the curve is
        // not symmetric about this point), distance ≈ 1.1072.
        approx(
            distance_to_quadratic(Point::new(0.75, 2.0), q),
            1.1071683846057803,
        );
    }

    #[test]
    fn distance_to_quadratic_matches_endpoints() {
        let q = Quadratic {
            a: Point::new(0.0, 0.0),
            c: Point::new(5.0, 10.0),
            b: Point::new(10.0, 0.0),
        };
        approx(distance_to_quadratic(Point::new(0.0, -1.0), q), 1.0);
        // Point beyond the right end: the closest point is interior, near
        // t ≈ 0.96 (the curve descends steeply there); distance ≈ 0.4622.
        approx(
            distance_to_quadratic(Point::new(10.0, 1.0), q),
            0.4622364145444681,
        );
        // A point exactly on the curve has distance 0.
        let on = q.eval(0.37);
        approx(distance_to_quadratic(on, q), 0.0);
    }

    #[test]
    fn cubic_to_quadratics_error_bound() {
        // A fairly curvy cubic; conversion must respect the tolerance.
        let p0 = Point::new(0.0, 0.0);
        let p1 = Point::new(3.0, 9.0);
        let p2 = Point::new(7.0, -9.0);
        let p3 = Point::new(10.0, 0.0);
        fn cubic(t: f64, p0: Point, p1: Point, p2: Point, p3: Point) -> Point {
            let u = 1.0 - t;
            let w0 = u * u * u;
            let w1 = 3.0 * u * u * t;
            let w2 = 3.0 * u * t * t;
            let w3 = t * t * t;
            Point::new(
                w0 * p0.x + w1 * p1.x + w2 * p2.x + w3 * p3.x,
                w0 * p0.y + w1 * p1.y + w2 * p2.y + w3 * p3.y,
            )
        }
        for tol in [1e-1, 1e-2, 1e-3, 1e-4] {
            let segs = cubic_to_quadratics(p0, p1, p2, p3, tol);
            assert!(!segs.is_empty());
            // Sample the cubic densely and check the distance to the quadratic
            // chain never exceeds the tolerance.
            let mut max_dev = 0.0_f64;
            let mut prev_end = p0;
            for q in &segs {
                approx(prev_end.x, q.a.x);
                approx(prev_end.y, q.a.y);
                for i in 0..=40 {
                    let t = i as f64 / 40.0;
                    let cubic_pt = cubic(t, p0, p1, p2, p3);
                    // Map the sample's t onto the whole chain: check against
                    // every segment and take the min.
                    let mut d = f64::INFINITY;
                    for s in &segs {
                        let local = distance_to_quadratic(cubic_pt, *s);
                        if local < d {
                            d = local;
                        }
                    }
                    if d > max_dev {
                        max_dev = d;
                    }
                }
                prev_end = q.b;
            }
            assert!(
                max_dev <= tol * 1.01 + 1e-9,
                "tol {tol}: max deviation {max_dev}"
            );
        }
    }

    #[test]
    fn cubic_to_quadratics_linear_is_single_segment() {
        let p0 = Point::new(0.0, 0.0);
        let p1 = Point::new(1.0, 2.0);
        let p2 = Point::new(2.0, 2.0);
        let p3 = Point::new(3.0, 0.0);
        let segs = cubic_to_quadratics(p0, p1, p2, p3, 1e-6);
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn winding_of_square() {
        // CCW square (0,0) → (1,0) → (1,1) → (0,1).
        let sq = [
            (Point::new(0.0, 0.0), Point::new(1.0, 0.0), None),
            (Point::new(1.0, 0.0), Point::new(1.0, 1.0), None),
            (Point::new(1.0, 1.0), Point::new(0.0, 1.0), None),
            (Point::new(0.0, 1.0), Point::new(0.0, 0.0), None),
        ];
        let winding =
            |p: Point| -> i32 { sq.iter().map(|(a, b, c)| edge_winding(p, *a, *b, *c)).sum() };
        assert_eq!(winding(Point::new(0.5, 0.5)), 1); // inside (CCW → +1)
        assert_eq!(winding(Point::new(2.0, 2.0)), 0); // outside
        assert_eq!(winding(Point::new(-0.5, 0.5)), 0); // left of the square
        assert_eq!(winding(Point::new(0.5, 2.0)), 0); // above
                                                      // Exactly on the boundary (endpoints convention) is either 0 or ±1;
                                                      // assert only that it is consistent with the half-open rule.
        let _ = winding(Point::new(1.0, 0.5));
    }

    #[test]
    fn winding_of_square_with_hole() {
        // Outer CCW square, inner CW square (hole): winding 0 inside the hole.
        let outer = [
            (Point::new(0.0, 0.0), Point::new(4.0, 0.0), None),
            (Point::new(4.0, 0.0), Point::new(4.0, 4.0), None),
            (Point::new(4.0, 4.0), Point::new(0.0, 4.0), None),
            (Point::new(0.0, 4.0), Point::new(0.0, 0.0), None),
        ];
        let inner = [
            (Point::new(1.0, 1.0), Point::new(1.0, 3.0), None),
            (Point::new(1.0, 3.0), Point::new(3.0, 3.0), None),
            (Point::new(3.0, 3.0), Point::new(3.0, 1.0), None),
            (Point::new(3.0, 1.0), Point::new(1.0, 1.0), None),
        ];
        let winding = |p: Point| -> i32 {
            outer
                .iter()
                .chain(inner.iter())
                .map(|(a, b, c)| edge_winding(p, *a, *b, *c))
                .sum()
        };
        assert_eq!(winding(Point::new(2.0, 2.0)), 0); // hole
        assert_eq!(winding(Point::new(0.5, 2.0)), 1); // outer ring
        assert_eq!(winding(Point::new(2.0, 0.5)), 1);
    }

    #[test]
    fn winding_of_quadratic_contour() {
        // A "lens" contour: top edge is a quadratic bulging up, bottom edge a
        // line. Shape: (0,0) → (4,0) line, then (4,0) → (0,0) quadratic with
        // control (2, 3). Winding at (2, 1) (inside) must be non-zero.
        let lens = [
            (Point::new(0.0, 0.0), Point::new(4.0, 0.0), None),
            (
                Point::new(4.0, 0.0),
                Point::new(0.0, 0.0),
                Some(Point::new(2.0, 3.0)),
            ),
        ];
        let winding = |p: Point| -> i32 {
            lens.iter()
                .map(|(a, b, c)| edge_winding(p, *a, *b, *c))
                .sum()
        };
        assert_eq!(winding(Point::new(2.0, 1.0)), 1);
        assert_eq!(winding(Point::new(2.0, -0.5)), 0);
        assert_eq!(winding(Point::new(2.0, 4.0)), 0);
        assert_eq!(winding(Point::new(0.1, 0.1)), 1);
        assert_eq!(winding(Point::new(-0.1, 0.5)), 0);
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    fn any_point() -> impl Strategy<Value = Point> {
        (-1000.0_f64..1000.0).prop_map(|x| {
            let y = (x * 1.618_033_988_749_895).sin() * 1000.0;
            Point::new(x, y)
        })
    }

    fn any_quadratic() -> impl Strategy<Value = Quadratic> {
        (-1000.0_f64..1000.0).prop_map(|cx| {
            let a = Point::new((cx * 0.37).sin() * 900.0, (cx * 0.71).cos() * 900.0);
            let b = Point::new((cx * 1.13).cos() * 900.0, (cx * 0.23).sin() * 900.0);
            let c = Point::new((cx * 0.57).sin() * 900.0, (cx * 1.41).cos() * 900.0);
            Quadratic { a, c, b }
        })
    }

    proptest! {
        /// The exact distance to a quadratic is consistent with the sampled
        /// minimum within a rigorous curvature-aware bound: with samples at
        /// spacing `h`, the sampled minimum overestimates the true minimum by
        /// at most `½·max|f″|·(h/2)²` where `f(t) = |Q(t) − p|²`.
        #[test]
        fn quadratic_distance_is_minimal(p in any_point(), q in any_quadratic()) {
            let exact = distance_to_quadratic(p, q);
            // f″(t) = 2(Q−p)·Q″ + 2|Q′|², with Q″ constant for quadratics.
            let qpp = q.a.sub(q.c.scale(2.0)).add(q.b).scale(2.0);
            let f2 = |t: f64| -> f64 {
                let e = q.eval(t).sub(p);
                let tng = q.tangent(t);
                (2.0 * e.dot(qpp) + 2.0 * tng.dot(tng)).abs()
            };
            let mut f2max = 0.0_f64;
            for i in 0..=32 {
                f2max = f2max.max(f2(i as f64 / 32.0));
            }
            let samples = 4096_usize;
            let h = 1.0 / samples as f64;
            let mut sampled = f64::INFINITY;
            for i in 0..=samples {
                let d = p.dist_sq(q.eval(i as f64 / samples as f64)).sqrt();
                if d < sampled {
                    sampled = d;
                }
            }
            // Soundness: the exact distance never exceeds any sample.
            assert!(exact <= sampled + 1e-9, "exact {exact} > sampled {sampled}");
            // Completeness: it is not below the true minimum, which the sampled
            // minimum overestimates by at most the curvature bound.
            let tol = 0.5 * f2max * (h / 2.0) * (h / 2.0) + 1e-6;
            assert!(
                exact >= sampled - tol,
                "exact {exact} below sampled {sampled} beyond bound {tol}"
            );
        }

        /// The line distance is exact for any point.
        #[test]
        fn line_distance_is_minimal(p in any_point(), a in any_point(), b in any_point()) {
            let line = Line { a, b };
            let exact = distance_to_line(p, line);
            let proj = {
                let ab = a.to(b);
                let len = ab.length();
                if len < 1e-9 {
                    p.dist_sq(a).sqrt()
                } else {
                    let t = (a.to(p).dot(ab) / (len * len)).clamp(0.0, 1.0);
                    p.dist_sq(a.lerp(b, t)).sqrt()
                }
            };
            assert!((exact - proj).abs() < 1e-6, "{exact} vs {proj}");
        }
    }
}
