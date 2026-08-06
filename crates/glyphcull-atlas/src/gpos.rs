//! Kerning extraction: GPOS PairPos (formats 1 and 2) plus the legacy `kern`
//! table, filtered to a used codepoint set.
//!
//! The OpenType kerning model: adjustments from every matching lookup apply in
//! order and accumulate. We extract horizontal x-advance adjustments only (the
//! v1 format carries a single `adjust` per pair). GPOS contextual lookups
//! (type 8) and mark/ligature positioning (types 4–6) are out of scope: they
//! belong to complex shaping, an explicit v1 exclusion.
//!
//! The parser is defensive: any malformed structure is skipped (the compiler
//! must never crash on hostile fonts), and the result is deterministic (all
//! output is collected into sorted maps).

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use ttf_parser::Face;

/// A bounds-checked reader over a table slice. All reads are fallible; a read
/// past the end yields `None` and aborts the current parse branch.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Read a u16 at an absolute offset (relative to the table start).
    fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
        let bytes = data.get(offset..offset.checked_add(2)?)?;
        Some(u16::from_be_bytes([
            bytes.first().copied()?,
            bytes.get(1).copied()?,
        ]))
    }

    fn u16(&mut self) -> Option<u16> {
        let v = Self::u16_at(self.data, self.pos)?;
        self.pos += 2;
        Some(v)
    }

    fn i16(&mut self) -> Option<i16> {
        let v = Self::u16_at(self.data, self.pos)?;
        self.pos += 2;
        Some(v as i16)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(())
    }
}

/// A coverage table: the set of glyph ids it covers.
fn parse_coverage(data: &[u8], offset: usize) -> Option<BTreeSet<u16>> {
    let mut r = Reader::new(data);
    r.skip(offset)?;
    let format = r.u16()?;
    let mut out = BTreeSet::new();
    match format {
        1 => {
            let count = r.u16()?;
            for _ in 0..count {
                let g = r.u16()?;
                out.insert(g);
            }
        }
        2 => {
            let count = r.u16()?;
            for _ in 0..count {
                let start = r.u16()?;
                let end = r.u16()?;
                let _start_index = r.u16()?;
                for g in start..=end {
                    out.insert(g);
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

/// The class of a glyph under a class-def table (0 when uncovered).
fn class_of(data: &[u8], offset: usize, glyph: u16) -> Option<u16> {
    let mut r = Reader::new(data);
    r.skip(offset)?;
    let format = r.u16()?;
    match format {
        1 => {
            let start = r.u16()?;
            let count = r.u16()?;
            if glyph < start || u32::from(glyph) >= u32::from(start) + u32::from(count) {
                return Some(0);
            }
            let idx = usize::from(glyph - start);
            let v = r.data.get(r.pos + idx).copied()?;
            Some(u16::from(v))
        }
        2 => {
            let count = r.u16()?;
            for _ in 0..count {
                let start = r.u16()?;
                let end = r.u16()?;
                let class = r.u16()?;
                if glyph >= start && glyph <= end {
                    return Some(class);
                }
            }
            Some(0)
        }
        _ => None,
    }
}

/// The byte size of a value record for a value format: two bytes per field bit
/// (0..4) plus two bytes per device-offset bit (8..12).
fn value_record_size(value_format: u16) -> usize {
    let fields = (value_format & 0x000F).count_ones() as usize;
    let devices = ((value_format >> 8) & 0x000F).count_ones() as usize;
    (fields + devices) * 2
}

/// Parse one value record, returning its x-advance. Placement/advance fields
/// (i16, bits 0..4) come first, then device offsets (u16, bits 8..12).
fn value_x_advance(r: &mut Reader<'_>, value_format: u16) -> Option<i16> {
    let mut x_advance = 0_i16;
    for bit in 0..4_u16 {
        if value_format & (1 << bit) != 0 {
            let v = r.i16()?;
            if bit == 2 {
                x_advance = v;
            }
        }
    }
    for bit in 8..12_u16 {
        if value_format & (1 << bit) != 0 {
            r.skip(2)?;
        }
    }
    Some(x_advance)
}

/// Accumulate PairPos adjustments (glyph ids) from the GPOS lookup list,
/// querying only the used glyph pairs.
fn parse_gpos(data: &[u8], used: &[u16], adjustments: &mut BTreeMap<(u16, u16), i32>) {
    // GPOS header: version (u16,u16), scriptList, featureList, lookupList.
    let lookup_list = match Reader::u16_at(data, 8) {
        Some(v) => usize::from(v),
        None => return,
    };
    let mut r = Reader::new(data);
    if r.skip(lookup_list).is_none() {
        return;
    }
    let lookup_count = match r.u16() {
        Some(v) => v,
        None => return,
    };
    let mut lookups: Vec<usize> = Vec::new();
    for _ in 0..lookup_count {
        let off = match r.u16() {
            Some(v) => usize::from(v),
            None => return,
        };
        lookups.push(lookup_list + off);
    }
    for lookup_off in lookups {
        let mut lr = Reader::new(data);
        if lr.skip(lookup_off).is_none() {
            continue;
        }
        let (lookup_type, sub_count) = match (lr.u16(), lr.u16(), lr.u16()) {
            (Some(t), Some(_flag), Some(c)) => (t, c),
            _ => continue,
        };
        if lookup_type != 2 {
            continue; // PairPos only (v1 scope)
        }
        let mut subtables: Vec<usize> = Vec::new();
        for _ in 0..sub_count {
            let off = match lr.u16() {
                Some(v) => usize::from(v),
                None => break,
            };
            subtables.push(lookup_off + off);
        }
        for sub_off in subtables {
            parse_pair_pos(data, sub_off, used, adjustments);
        }
    }
}

/// Parse one PairPos subtable (format 1: explicit glyph pairs; format 2:
/// class-based) and accumulate x-advance adjustments for used pairs.
fn parse_pair_pos(
    data: &[u8],
    sub_off: usize,
    used: &[u16],
    adjustments: &mut BTreeMap<(u16, u16), i32>,
) {
    let format = match Reader::u16_at(data, sub_off) {
        Some(v) => v,
        None => return,
    };
    let coverage_off = match Reader::u16_at(data, sub_off + 2) {
        Some(v) => usize::from(v),
        None => return,
    };
    let value_format1 = match Reader::u16_at(data, sub_off + 4) {
        Some(v) => v,
        None => return,
    };
    let value_format2 = match Reader::u16_at(data, sub_off + 6) {
        Some(v) => v,
        None => return,
    };
    let coverage = match parse_coverage(data, sub_off + coverage_off) {
        Some(c) => c,
        None => return,
    };

    match format {
        1 => {
            let pair_set_count = match Reader::u16_at(data, sub_off + 8) {
                Some(v) => v,
                None => return,
            };
            let mut pair_sets: Vec<usize> = Vec::new();
            for i in 0..pair_set_count {
                let off = match Reader::u16_at(data, sub_off + 10 + usize::from(i) * 2) {
                    Some(v) => usize::from(v),
                    None => return,
                };
                pair_sets.push(sub_off + off);
            }
            // Coverage index i ↔ pair_set i (coverage sorted ascending), so a
            // used first-glyph maps to its pair set by coverage rank.
            for &first in used {
                let Some(rank) = coverage.iter().position(|&g| g == first) else {
                    continue;
                };
                let Some(set_off) = pair_sets.get(rank).copied() else {
                    continue;
                };
                let mut pr = Reader::new(data);
                if pr.skip(set_off).is_none() {
                    continue;
                }
                let count = match pr.u16() {
                    Some(v) => v,
                    None => continue,
                };
                for _ in 0..count {
                    let second = match pr.u16() {
                        Some(v) => v,
                        None => break,
                    };
                    let Some(vr1) = value_x_advance(&mut pr, value_format1) else {
                        break;
                    };
                    let Some(vr2) = value_x_advance(&mut pr, value_format2) else {
                        break;
                    };
                    if !used.contains(&second) {
                        continue;
                    }
                    let adjust = i32::from(vr1) + i32::from(vr2);
                    if adjust != 0 {
                        let entry = adjustments.entry((first, second)).or_insert(0);
                        *entry += adjust;
                    }
                }
            }
        }
        2 => {
            let class_def1_off = match Reader::u16_at(data, sub_off + 8) {
                Some(v) => usize::from(v),
                None => return,
            };
            let class_def2_off = match Reader::u16_at(data, sub_off + 10) {
                Some(v) => usize::from(v),
                None => return,
            };
            let class1_count = match Reader::u16_at(data, sub_off + 12) {
                Some(v) => v,
                None => return,
            };
            let class2_count = match Reader::u16_at(data, sub_off + 14) {
                Some(v) => v,
                None => return,
            };
            let records_base = sub_off + 16;
            let record_size = value_record_size(value_format1) + value_record_size(value_format2);
            for &first in used {
                if !coverage.contains(&first) {
                    continue;
                }
                let Some(class1) = class_of(data, sub_off + class_def1_off, first) else {
                    continue;
                };
                if u32::from(class1) >= u32::from(class1_count) {
                    continue;
                }
                for &second in used {
                    let Some(class2) = class_of(data, sub_off + class_def2_off, second) else {
                        continue;
                    };
                    if u32::from(class2) >= u32::from(class2_count) {
                        continue;
                    }
                    let index =
                        usize::from(class1) * usize::from(class2_count) + usize::from(class2);
                    let offset = records_base + index * record_size;
                    let mut vr = Reader::new(data);
                    if vr.skip(offset).is_none() {
                        continue;
                    }
                    let (Some(x1), Some(x2)) = (
                        value_x_advance(&mut vr, value_format1),
                        value_x_advance(&mut vr, value_format2),
                    ) else {
                        continue;
                    };
                    let adjust = i32::from(x1) + i32::from(x2);
                    if adjust != 0 {
                        let entry = adjustments.entry((first, second)).or_insert(0);
                        *entry += adjust;
                    }
                }
            }
        }
        _ => {}
    }
}

/// Extract kerning adjustments (font units) for every pair whose left and right
/// glyphs both map from `codepoints`. Deterministic: sorted output, file-order
/// accumulation.
pub fn kerning_pairs(face: &Face<'_>, codepoints: &BTreeSet<u32>) -> BTreeMap<(u32, u32), i32> {
    let mut glyph_to_cp: BTreeMap<u16, u32> = BTreeMap::new();
    for &cp in codepoints {
        let Some(c) = char::from_u32(cp) else {
            continue;
        };
        let Some(gid) = face.glyph_index(c) else {
            continue;
        };
        glyph_to_cp.insert(gid.0, cp);
    }
    if glyph_to_cp.is_empty() {
        return BTreeMap::new();
    }
    let used: Vec<u16> = glyph_to_cp.keys().copied().collect();

    let mut adjustments: BTreeMap<(u16, u16), i32> = BTreeMap::new();

    // GPOS PairPos lookups (accumulate in file order).
    if let Some(gpos) = face.raw_face().table(ttf_parser::Tag::from_bytes(b"GPOS")) {
        parse_gpos(gpos, &used, &mut adjustments);
    }

    // Legacy `kern` table: query every used pair against every horizontal
    // subtable and accumulate (OpenType model).
    if let Some(kern) = face.tables().kern {
        for sub in kern.subtables {
            if !sub.horizontal || sub.has_state_machine || sub.has_cross_stream {
                continue;
            }
            for &left in &used {
                for &right in &used {
                    if let Some(v) =
                        sub.glyphs_kerning(ttf_parser::GlyphId(left), ttf_parser::GlyphId(right))
                    {
                        let entry = adjustments.entry((left, right)).or_insert(0);
                        *entry += i32::from(v);
                    }
                }
            }
        }
    }

    // Filter to used pairs and map glyphs → codepoints.
    let mut out: BTreeMap<(u32, u32), i32> = BTreeMap::new();
    for ((left, right), adjust) in adjustments {
        if adjust == 0 {
            continue;
        }
        if let (Some(&lcp), Some(&rcp)) = (glyph_to_cp.get(&left), glyph_to_cp.get(&right)) {
            out.insert((lcp, rcp), adjust);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundled Noto Sans Regular.
    const NOTO: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");

    fn used(s: &str) -> BTreeSet<u32> {
        s.chars().map(|c| c as u32).collect()
    }

    #[test]
    fn noto_av_kerning_negative() {
        let face = Face::parse(NOTO, 0).expect("font");
        let pairs = kerning_pairs(&face, &used("AVToWYa"));
        let av = (('A' as u32), ('V' as u32));
        assert!(
            pairs.get(&av).copied().unwrap_or(0) < 0,
            "A/V should kern negatively, got {pairs:?}"
        );
        // Deterministic and sorted output.
        assert!(pairs.iter().is_sorted_by_key(|((l, r), _)| (*l, *r)));
    }

    #[test]
    fn kerning_is_deterministic() {
        let face = Face::parse(NOTO, 0).expect("font");
        let cps = used("AVToWYaHYL");
        assert_eq!(kerning_pairs(&face, &cps), kerning_pairs(&face, &cps));
    }

    #[test]
    fn empty_used_set_yields_empty() {
        let face = Face::parse(NOTO, 0).expect("font");
        assert!(kerning_pairs(&face, &BTreeSet::new()).is_empty());
    }
}
