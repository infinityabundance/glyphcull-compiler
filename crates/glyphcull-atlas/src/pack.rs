//! Deterministic rectangle packing (skyline algorithm).
//!
//! Glyph boxes are packed into pages of a fixed size. The skyline maintains the
//! envelope of placed rects; each rect is placed at the leftmost position where
//! its bottom (the maximum skyline height over its footprint) is minimal — the
//! classic bottom-left heuristic — and rects that do not fit open a new page.
//!
//! Determinism: rects are processed in a caller-chosen fixed order (the packer
//! never sorts by unstable keys), and ties are broken by position (leftmost,
//! then topmost). The packer is purely functional: no randomness, no hash-map
//! iteration.

/// A placed rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedRect {
    /// Left edge (page space).
    pub x: u16,
    /// Top edge (page space).
    pub y: u16,
    /// Page index.
    pub page: u16,
}

/// One skyline segment: from `x` to `x + width` the occupied height is `y`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Segment {
    x: u32,
    y: u32,
    width: u32,
}

/// Pack `rects` (width, height) into pages of `page_width × page_height`.
/// Returns one [`PlacedRect`] per input rect, in input order.
pub fn pack_rects(rects: &[(u32, u32)], page_width: u32, page_height: u32) -> Vec<PlacedRect> {
    let mut out: Vec<PlacedRect> = Vec::with_capacity(rects.len());
    let mut skyline: Vec<Segment> = vec![Segment {
        x: 0,
        y: 0,
        width: page_width,
    }];
    let mut page: u16 = 0;

    for &(w, h) in rects {
        if w > page_width || h > page_height {
            // Oversized for any page: own page, top-left.
            page = page.saturating_add(1);
            out.push(PlacedRect { x: 0, y: 0, page });
            continue;
        }
        let placement = find_position(&skyline, w, h, page_width, page_height);
        match placement {
            Some((x, y)) => {
                insert(&mut skyline, x, y, w, h);
                out.push(PlacedRect {
                    x: x as u16,
                    y: y as u16,
                    page,
                });
            }
            None => {
                page = page.saturating_add(1);
                skyline = vec![Segment {
                    x: 0,
                    y: 0,
                    width: page_width,
                }];
                // A fresh page always fits (the guard above guarantees
                // w ≤ page_width and h ≤ page_height), so place at the origin.
                insert(&mut skyline, 0, 0, w, h);
                out.push(PlacedRect { x: 0, y: 0, page });
            }
        }
    }
    out
}

/// Find the leftmost position where the rect's bottom (max skyline height over
/// its footprint) is minimal and the rect fits the page.
fn find_position(
    skyline: &[Segment],
    w: u32,
    h: u32,
    page_width: u32,
    page_height: u32,
) -> Option<(u32, u32)> {
    let mut best: Option<(u32, u32)> = None; // (x, y)
    for seg in skyline {
        let x = seg.x;
        if x + w > page_width {
            continue;
        }
        let mut y = seg.y;
        for other in skyline {
            if other.x >= x && other.x < x + w && other.y > y {
                y = other.y;
            }
        }
        if y + h > page_height {
            continue;
        }
        // Bottom-left heuristic: minimal y, then leftmost x.
        let better = match best {
            None => true,
            Some((bx, by)) => y < by || (y == by && x < bx),
        };
        if better {
            best = Some((x, y));
        }
    }
    best
}

/// Insert a rect into the skyline at (x, y), merging adjacent segments.
fn insert(skyline: &mut Vec<Segment>, x: u32, y: u32, w: u32, h: u32) {
    // The skyline segments are contiguous and cover [0, page_width]. Split the
    // covered range [x, x+w) and raise those segments to y + h.
    let mut new_sky: Vec<Segment> = Vec::with_capacity(skyline.len() + 2);
    for seg in skyline.drain(..) {
        let seg_end = seg.x + seg.width;
        if seg_end <= x || seg.x >= x + w {
            new_sky.push(seg);
            continue;
        }
        // Overlap with [x, x+w): keep the part before the rect, then raise.
        if seg.x < x {
            new_sky.push(Segment {
                x: seg.x,
                y: seg.y,
                width: x - seg.x,
            });
        }
        let overlap_start = seg.x.max(x);
        let overlap_end = seg_end.min(x + w);
        if overlap_start < overlap_end {
            // The raised segment spans the overlap; extend the raised run when
            // it is adjacent to the previous one. NOTE: the tail below must run
            // in every case — the `continue` in an earlier draft skipped it and
            // dropped the skyline's coverage past `x + w`, corrupting the
            // envelope (a later rect could then straddle the gap and overflow
            // the page; regression-tested in `insert_preserves_full_coverage`).
            let mut merged = false;
            if let Some(last) = new_sky.last_mut() {
                if last.y == y + h && last.x + last.width == overlap_start {
                    last.width += overlap_end - overlap_start;
                    merged = true;
                }
            }
            if !merged {
                new_sky.push(Segment {
                    x: overlap_start,
                    y: y + h,
                    width: overlap_end - overlap_start,
                });
            }
        }
        if seg_end > x + w {
            new_sky.push(Segment {
                x: x + w,
                y: seg.y,
                width: seg_end - (x + w),
            });
        }
    }
    // Merge any adjacent same-height segments for compactness.
    let mut merged: Vec<Segment> = Vec::with_capacity(new_sky.len());
    for seg in new_sky {
        if let Some(last) = merged.last_mut() {
            if last.y == seg.y && last.x + last.width == seg.x {
                last.width += seg.width;
                continue;
            }
        }
        merged.push(seg);
    }
    *skyline = merged;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_rect_at_origin() {
        let placed = pack_rects(&[(10, 10)], 100, 100);
        assert_eq!(placed.len(), 1);
        assert_eq!(
            placed[0],
            PlacedRect {
                x: 0,
                y: 0,
                page: 0
            }
        );
    }

    #[test]
    fn side_by_side_rects() {
        let placed = pack_rects(&[(10, 10), (10, 10)], 100, 100);
        assert_eq!(placed.len(), 2);
        // Both fit on one page, never overlapping.
        let a = placed[0];
        let b = placed[1];
        assert_eq!(a.page, 0);
        assert_eq!(b.page, 0);
        let overlap = u32::from(a.x) < u32::from(b.x) + 10
            && u32::from(b.x) < u32::from(a.x) + 10
            && u32::from(a.y) < u32::from(b.y) + 10
            && u32::from(b.y) < u32::from(a.y) + 10;
        assert!(!overlap);
    }

    #[test]
    fn overflow_opens_new_page() {
        let placed = pack_rects(&[(200, 200), (200, 200)], 250, 250);
        assert_eq!(placed[0].page, 0);
        assert_eq!(placed[1].page, 1);
        assert_eq!(placed[1].x, 0);
        assert_eq!(placed[1].y, 0);
    }

    #[test]
    fn never_overlaps_and_stays_in_page() {
        let mut rects = Vec::new();
        for i in 0..50_u32 {
            let w = 3 + (i * 7) % 40;
            let h = 2 + (i * 11) % 30;
            rects.push((w, h));
        }
        let placed = pack_rects(&rects, 256, 256);
        assert_eq!(placed.len(), rects.len());
        for (i, p) in placed.iter().enumerate() {
            let (w, h) = rects[i];
            assert!(u32::from(p.x) + w <= 256);
            assert!(u32::from(p.y) + h <= 256);
        }
        for i in 0..placed.len() {
            for j in (i + 1)..placed.len() {
                if placed[i].page != placed[j].page {
                    continue;
                }
                let a = placed[i];
                let b = placed[j];
                let (aw, ah) = rects[i];
                let (bw, bh) = rects[j];
                let overlap = u32::from(a.x) < u32::from(b.x) + bw
                    && u32::from(b.x) < u32::from(a.x) + aw
                    && u32::from(a.y) < u32::from(b.y) + bh
                    && u32::from(b.y) < u32::from(a.y) + ah;
                assert!(!overlap, "rects {i} and {j} overlap");
            }
        }
    }

    #[test]
    fn deterministic() {
        let rects = [(9, 3), (4, 7), (12, 2), (5, 5), (20, 20), (6, 6)];
        assert_eq!(pack_rects(&rects, 64, 64), pack_rects(&rects, 64, 64));
    }

    #[test]
    fn skyline_merges_adjacent() {
        let mut sky: Vec<Segment> = vec![Segment {
            x: 0,
            y: 0,
            width: 100,
        }];
        insert(&mut sky, 0, 0, 10, 10);
        assert_eq!(sky.len(), 2);
        assert_eq!(sky[0].y, 10);
        assert_eq!(sky[0].width, 10);
        assert_eq!(sky[1].x, 10);
    }

    #[test]
    fn insert_preserves_full_coverage() {
        // Regression: the merge path used to `continue` past the tail handling,
        // dropping the skyline's coverage past `x + w`. The sequence below
        // triggers the merge (the raised overlap extends the previous raised
        // run) AND needs the tail (the current segment extends past `x + w`),
        // which is exactly the case that lost the tail.
        let mut sky: Vec<Segment> = vec![Segment {
            x: 0,
            y: 0,
            width: 100,
        }];
        insert(&mut sky, 0, 0, 10, 10); // (0,10,10) raised; tail (10,0,90)
        insert(&mut sky, 10, 0, 10, 10); // overlap [10,20) merges into (0,10,20);
                                         // the tail (20,0,80) must survive
        assert_eq!(
            sky,
            vec![
                Segment {
                    x: 0,
                    y: 10,
                    width: 20
                },
                Segment {
                    x: 20,
                    y: 0,
                    width: 80
                }
            ]
        );
        // Coverage invariant: contiguous segments covering [0, page_width).
        let mut cursor = 0_u32;
        for seg in &sky {
            assert_eq!(seg.x, cursor, "skyline gap at {cursor}");
            cursor += seg.width;
        }
        assert_eq!(cursor, 100);
    }

    #[test]
    fn pack_stays_in_page_with_merges() {
        // Regression: the corrupted skyline let a rect straddle a gap and
        // overflow the page bottom. A row of equal-height rects forces the
        // merge path on every insert; every rect must stay in page bounds.
        let rects: Vec<(u32, u32)> = (0..40).map(|_| (25, 35)).collect();
        let placed = pack_rects(&rects, 512, 512);
        assert_eq!(placed.len(), rects.len());
        for (i, p) in placed.iter().enumerate() {
            let (w, h) = rects[i];
            assert!(
                u32::from(p.x) + w <= 512 && u32::from(p.y) + h <= 512,
                "rect {i} at {p:?} overflows 512x512"
            );
        }
    }
}
