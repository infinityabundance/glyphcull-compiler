//! GLYF section codec (SPEC.md §2.5): MSDF glyph atlases.

use crate::error::{Error, Result};
use crate::util::{Cursor, Writer};

/// Maximum atlases (defensive; bounded by section size in practice).
pub const MAX_ATLAS_COUNT: u32 = 1 << 16;

/// Maximum glyphs per atlas (SPEC.md §1.3).
pub const MAX_GLYPH_COUNT: u32 = 1 << 16;

/// Maximum kerning pairs per atlas (SPEC.md §1.3).
pub const MAX_KERNING_COUNT: u32 = 1 << 24;

/// Maximum atlas page dimension in texels (SPEC.md §1.3).
pub const MAX_PAGE_DIM: u32 = 8192;

/// Maximum font family name length (defensive).
pub const MAX_FAMILY_LEN: usize = 1024;

/// A glyph record is exactly 32 bytes.
pub const GLYPH_RECORD_LEN: usize = 32;

/// Glyph flag bits (SPEC.md §2.5).
pub mod glyph_flags {
    /// The glyph has no outline (space, combining).
    pub const NO_OUTLINE: u8 = 1 << 0;
    /// Combining mark (advance 0, positioned by the base glyph).
    pub const COMBINING: u8 = 1 << 1;

    /// All defined bits.
    pub const ALL: u8 = 0x03;
}

/// One glyph record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphRecord {
    /// Unicode codepoint (unique within the atlas).
    pub codepoint: u32,
    /// Advance width in em.
    pub advance: f32,
    /// Left bearing in em.
    pub bearing_x: f32,
    /// Top bearing in em (baseline-relative, positive upward).
    pub bearing_y: f32,
    /// Box left edge in texels (page space).
    pub box_x: u16,
    /// Box top edge in texels (page space).
    pub box_y: u16,
    /// Box width in texels (≥ 1).
    pub box_w: u16,
    /// Box height in texels (≥ 1).
    pub box_h: u16,
    /// Atlas page index.
    pub page_index: u16,
    /// Glyph flags (see [`glyph_flags`]).
    pub flags: u8,
}

impl GlyphRecord {
    /// Encode as exactly [`GLYPH_RECORD_LEN`] bytes.
    #[must_use]
    pub fn encode(&self) -> [u8; GLYPH_RECORD_LEN] {
        let mut w = Writer::new();
        w.u32(self.codepoint);
        w.f32(self.advance);
        w.f32(self.bearing_x);
        w.f32(self.bearing_y);
        w.u16(self.box_x);
        w.u16(self.box_y);
        w.u16(self.box_w);
        w.u16(self.box_h);
        w.u16(self.page_index);
        w.u8(self.flags);
        w.u8(0); // reserved
        w.u32(0); // reserved
        let bytes = w.into_bytes();
        // Fixed write sequence of exactly GLYPH_RECORD_LEN bytes; cannot fail.
        #[allow(clippy::expect_used)]
        {
            bytes.try_into().expect("glyph record is exactly 32 bytes")
        }
    }

    /// Decode a glyph record.
    pub fn decode(bytes: &[u8; GLYPH_RECORD_LEN]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let codepoint = c.u32("glyph codepoint")?;
        let advance = c.f32("glyph advance")?;
        let bearing_x = c.f32("glyph bearing_x")?;
        let bearing_y = c.f32("glyph bearing_y")?;
        let box_x = c.u16("glyph box_x")?;
        let box_y = c.u16("glyph box_y")?;
        let box_w = c.u16("glyph box_w")?;
        let box_h = c.u16("glyph box_h")?;
        let page_index = c.u16("glyph page_index")?;
        let flags = c.u8("glyph flags")?;
        let reserved = c.u8("glyph reserved")?;
        let reserved2 = c.u32("glyph reserved2")?;
        c.finish("glyph record")?;
        if reserved != 0 || reserved2 != 0 {
            return Err(Error::ReservedBitsSet);
        }
        if flags & !glyph_flags::ALL != 0 {
            return Err(Error::UnknownValue {
                what: "glyph flags",
                value: u64::from(flags),
            });
        }
        if codepoint > 0x10FFFF {
            return Err(Error::UnknownValue {
                what: "glyph codepoint",
                value: u64::from(codepoint),
            });
        }
        if box_w == 0 || box_h == 0 {
            return Err(Error::LimitExceeded {
                what: "glyph box dimension",
                value: 0,
                limit: 1,
            });
        }
        for v in [advance, bearing_x, bearing_y] {
            if !v.is_finite() {
                return Err(Error::UnknownValue {
                    what: "glyph metric",
                    value: 0,
                });
            }
        }
        Ok(Self {
            codepoint,
            advance,
            bearing_x,
            bearing_y,
            box_x,
            box_y,
            box_w,
            box_h,
            page_index,
            flags,
        })
    }
}

/// A kerning pair (SPEC.md §2.5), sorted by (left, right).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KerningPair {
    /// Left codepoint.
    pub left: u32,
    /// Right codepoint.
    pub right: u32,
    /// Advance adjustment in em.
    pub adjust: f32,
}

/// One atlas.
#[derive(Debug, Clone, PartialEq)]
pub struct Atlas {
    /// Font id (referenced by styles).
    pub font_id: u32,
    /// Format; v1 requires 0 (MSDF RGBA8).
    pub format: u8,
    /// Padding texels around each glyph box.
    pub padding: u16,
    /// Atlas density, fixed-point ×1024 texels per em.
    pub texels_per_em: u32,
    /// Typographic ascent in em.
    pub ascent: f32,
    /// Typographic descent in em (positive).
    pub descent: f32,
    /// Line gap in em.
    pub line_gap: f32,
    /// Cap height in em.
    pub cap_height: f32,
    /// X height in em.
    pub x_height: f32,
    /// Font units per em.
    pub units_per_em: f32,
    /// Font family name (UTF-8).
    pub family: String,
    /// Font weight 100..=900.
    pub weight: u16,
    /// Italic flag.
    pub italic: bool,
    /// Page width in texels.
    pub page_width: u32,
    /// Page height in texels.
    pub page_height: u32,
    /// Glyph records.
    pub glyphs: Vec<GlyphRecord>,
    /// Kerning pairs sorted by (left, right).
    pub kerning: Vec<KerningPair>,
    /// Page pixel data: RGBA8, `page_width × page_height × 4` bytes per page.
    pub pages: Vec<Vec<u8>>,
}

/// The decoded GLYF section.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphSection {
    /// Atlases.
    pub atlases: Vec<Atlas>,
}

impl GlyphSection {
    /// Encode to the GLYF payload (glyphs sorted by codepoint; kerning sorted).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.atlases.len() as u32);
        for atlas in &self.atlases {
            w.u32(atlas.font_id);
            w.u32(atlas.glyphs.len() as u32);
            w.u16(atlas.pages.len() as u16);
            w.u8(atlas.format);
            w.u8(0); // flags
            w.u16(atlas.padding);
            w.u32(atlas.texels_per_em);
            w.f32(atlas.ascent);
            w.f32(atlas.descent);
            w.f32(atlas.line_gap);
            w.f32(atlas.cap_height);
            w.f32(atlas.x_height);
            w.f32(atlas.units_per_em);
            w.u16(atlas.family.len() as u16);
            w.bytes(atlas.family.as_bytes());
            w.u16(atlas.weight);
            w.u8(u8::from(atlas.italic));
            w.u8(0); // reserved
            w.u32(atlas.page_width);
            w.u32(atlas.page_height);
            let mut glyphs = atlas.glyphs.clone();
            glyphs.sort_by_key(|g| g.codepoint);
            for glyph in &glyphs {
                w.bytes(&glyph.encode());
            }
            let mut kerning = atlas.kerning.clone();
            kerning.sort_by_key(|k| (k.left, k.right));
            w.u32(kerning.len() as u32);
            for pair in &kerning {
                w.u32(pair.left);
                w.u32(pair.right);
                w.f32(pair.adjust);
            }
            for page in &atlas.pages {
                w.bytes(page);
            }
        }
        w.into_bytes()
    }

    /// Decode and structurally validate the GLYF payload.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let atlas_count = c.u32("atlas count")?;
        if atlas_count > MAX_ATLAS_COUNT {
            return Err(Error::LimitExceeded {
                what: "atlas count",
                value: u64::from(atlas_count),
                limit: u64::from(MAX_ATLAS_COUNT),
            });
        }
        let mut atlases = Vec::with_capacity(atlas_count as usize);
        for _ in 0..atlas_count {
            let font_id = c.u32("atlas font_id")?;
            let glyph_count = c.u32("atlas glyph count")?;
            let page_count = usize::from(c.u16("atlas page count")?);
            let format = c.u8("atlas format")?;
            let flags = c.u8("atlas flags")?;
            let padding = c.u16("atlas padding")?;
            let texels_per_em = c.u32("atlas texels_per_em")?;
            let ascent = c.f32("atlas ascent")?;
            let descent = c.f32("atlas descent")?;
            let line_gap = c.f32("atlas line_gap")?;
            let cap_height = c.f32("atlas cap_height")?;
            let x_height = c.f32("atlas x_height")?;
            let units_per_em = c.f32("atlas units_per_em")?;
            let family_len = usize::from(c.u16("atlas family len")?);
            if family_len > MAX_FAMILY_LEN {
                return Err(Error::LimitExceeded {
                    what: "atlas family len",
                    value: family_len as u64,
                    limit: MAX_FAMILY_LEN as u64,
                });
            }
            let family = c.utf8(family_len, "atlas family")?.to_string();
            let weight = c.u16("atlas weight")?;
            let italic = c.u8("atlas italic")?;
            let reserved = c.u8("atlas reserved")?;
            let page_width = c.u32("atlas page_width")?;
            let page_height = c.u32("atlas page_height")?;

            if flags != 0 || reserved != 0 {
                return Err(Error::ReservedBitsSet);
            }
            if format != 0 {
                return Err(Error::UnknownValue {
                    what: "atlas format",
                    value: u64::from(format),
                });
            }
            if texels_per_em == 0 {
                return Err(Error::LimitExceeded {
                    what: "texels_per_em",
                    value: 0,
                    limit: 1,
                });
            }
            if !(100..=900).contains(&weight) {
                return Err(Error::UnknownValue {
                    what: "atlas weight",
                    value: u64::from(weight),
                });
            }
            if italic > 1 {
                return Err(Error::UnknownValue {
                    what: "atlas italic",
                    value: u64::from(italic),
                });
            }
            if page_width == 0
                || page_height == 0
                || page_width > MAX_PAGE_DIM
                || page_height > MAX_PAGE_DIM
            {
                return Err(Error::LimitExceeded {
                    what: "atlas page dimension",
                    value: u64::from(page_width.max(page_height)),
                    limit: u64::from(MAX_PAGE_DIM),
                });
            }
            for v in [
                ascent,
                descent,
                line_gap,
                cap_height,
                x_height,
                units_per_em,
            ] {
                if !v.is_finite() || v < 0.0 {
                    return Err(Error::UnknownValue {
                        what: "atlas metric",
                        value: 0,
                    });
                }
            }

            if glyph_count > MAX_GLYPH_COUNT {
                return Err(Error::LimitExceeded {
                    what: "glyph count",
                    value: u64::from(glyph_count),
                    limit: u64::from(MAX_GLYPH_COUNT),
                });
            }
            let mut glyphs = Vec::with_capacity(glyph_count as usize);
            let mut seen = std::collections::HashSet::with_capacity(glyph_count as usize);
            for _ in 0..glyph_count {
                let record_bytes = c.take(GLYPH_RECORD_LEN, "glyph record")?;
                let record: &[u8; GLYPH_RECORD_LEN] =
                    record_bytes.try_into().map_err(|_| Error::UnexpectedEof {
                        what: "glyph record",
                    })?;
                let glyph = GlyphRecord::decode(record)?;
                if !seen.insert(glyph.codepoint) {
                    return Err(Error::UnknownValue {
                        what: "duplicate glyph codepoint",
                        value: u64::from(glyph.codepoint),
                    });
                }
                if u32::from(glyph.page_index) >= page_count as u32 {
                    return Err(Error::DanglingReference {
                        what: "glyph page",
                        id: u32::from(glyph.page_index),
                    });
                }
                if u32::from(glyph.box_x) + u32::from(glyph.box_w) > page_width
                    || u32::from(glyph.box_y) + u32::from(glyph.box_h) > page_height
                {
                    return Err(Error::OutOfBounds {
                        offset: u64::from(glyph.box_x) + u64::from(glyph.box_y),
                        length: u64::from(glyph.box_w) + u64::from(glyph.box_h),
                        file_len: u64::from(page_width) + u64::from(page_height),
                    });
                }
                glyphs.push(glyph);
            }

            let kerning_count = c.u32("kerning count")?;
            if kerning_count > MAX_KERNING_COUNT {
                return Err(Error::LimitExceeded {
                    what: "kerning count",
                    value: u64::from(kerning_count),
                    limit: u64::from(MAX_KERNING_COUNT),
                });
            }
            let mut kerning = Vec::with_capacity(kerning_count as usize);
            let mut prev: Option<(u32, u32)> = None;
            for _ in 0..kerning_count {
                let left = c.u32("kerning left")?;
                let right = c.u32("kerning right")?;
                let adjust = c.f32("kerning adjust")?;
                if !adjust.is_finite() {
                    return Err(Error::UnknownValue {
                        what: "kerning adjust",
                        value: 0,
                    });
                }
                if let Some(p) = prev {
                    if (left, right) <= p {
                        return Err(Error::UnknownValue {
                            what: "kerning ordering",
                            value: 0,
                        });
                    }
                }
                prev = Some((left, right));
                kerning.push(KerningPair {
                    left,
                    right,
                    adjust,
                });
            }

            let page_bytes = (page_width as usize)
                .checked_mul(page_height as usize)
                .and_then(|v| v.checked_mul(4))
                .ok_or(Error::LimitExceeded {
                    what: "atlas page bytes",
                    value: u64::MAX,
                    limit: u64::from(MAX_PAGE_DIM) * u64::from(MAX_PAGE_DIM) * 4,
                })?;
            let mut pages = Vec::with_capacity(page_count);
            for _ in 0..page_count {
                pages.push(c.take(page_bytes, "atlas page")?.to_vec());
            }

            atlases.push(Atlas {
                font_id,
                format,
                padding,
                texels_per_em,
                ascent,
                descent,
                line_gap,
                cap_height,
                x_height,
                units_per_em,
                family,
                weight,
                italic: italic != 0,
                page_width,
                page_height,
                glyphs,
                kerning,
                pages,
            });
        }
        c.finish("GLYF payload")?;
        Ok(Self { atlases })
    }
}

#[cfg(test)]
mod tests {
    use super::{glyph_flags, Atlas, GlyphRecord, GlyphSection, KerningPair};

    fn sample_atlas() -> Atlas {
        let page_w = 64_u32;
        let page_h = 64_u32;
        let mut page = vec![0_u8; (page_w * page_h * 4) as usize];
        page[0] = 255; // distinct content so round-trip is meaningful
        Atlas {
            font_id: 0,
            format: 0,
            padding: 4,
            texels_per_em: 32768,
            ascent: 0.75,
            descent: 0.25,
            line_gap: 0.0,
            cap_height: 0.7,
            x_height: 0.5,
            units_per_em: 1000.0,
            family: "Test Sans".to_string(),
            weight: 400,
            italic: false,
            page_width: page_w,
            page_height: page_h,
            glyphs: vec![
                GlyphRecord {
                    codepoint: ' ' as u32,
                    advance: 0.25,
                    bearing_x: 0.0,
                    bearing_y: 0.0,
                    box_x: 4,
                    box_y: 4,
                    box_w: 1,
                    box_h: 1,
                    page_index: 0,
                    flags: glyph_flags::NO_OUTLINE,
                },
                GlyphRecord {
                    codepoint: 'A' as u32,
                    advance: 0.6,
                    bearing_x: 0.05,
                    bearing_y: 0.7,
                    box_x: 4,
                    box_y: 4,
                    box_w: 32,
                    box_h: 48,
                    page_index: 0,
                    flags: 0,
                },
            ],
            kerning: vec![KerningPair {
                left: 'A' as u32,
                right: 'V' as u32,
                adjust: -0.05,
            }],
            pages: vec![page],
        }
    }

    fn sample_section() -> GlyphSection {
        GlyphSection {
            atlases: vec![sample_atlas()],
        }
    }

    #[test]
    fn round_trip() {
        let section = sample_section();
        let bytes = section.encode();
        assert_eq!(GlyphSection::decode(&bytes).expect("decode"), section);
    }

    #[test]
    fn glyphs_sorted_by_codepoint() {
        let mut section = sample_section();
        section.atlases[0].glyphs.reverse();
        let bytes = section.encode();
        let decoded = GlyphSection::decode(&bytes).expect("decode");
        let cps: Vec<u32> = decoded.atlases[0]
            .glyphs
            .iter()
            .map(|g| g.codepoint)
            .collect();
        assert_eq!(cps, vec![0x20, 0x41]);
    }

    #[test]
    fn duplicate_codepoint_rejected() {
        let mut section = sample_section();
        section.atlases[0].glyphs[0].codepoint = 'A' as u32; // space → A collides
        assert!(GlyphSection::decode(&section.encode()).is_err());
    }

    #[test]
    fn box_out_of_page_rejected() {
        let mut section = sample_section();
        section.atlases[0].glyphs[0].box_w = 100; // page is 64 wide
        assert!(GlyphSection::decode(&section.encode()).is_err());
    }

    #[test]
    fn unsorted_kerning_rejected() {
        let mut section = sample_section();
        section.atlases[0].kerning.push(KerningPair {
            left: 'A' as u32,
            right: 'V' as u32,
            adjust: 0.0,
        });
        assert!(GlyphSection::decode(&section.encode()).is_err());
    }

    #[test]
    fn bad_page_size_rejected() {
        let mut section = sample_section();
        section.atlases[0].pages[0].truncate(10);
        assert!(GlyphSection::decode(&section.encode()).is_err());
    }
}
