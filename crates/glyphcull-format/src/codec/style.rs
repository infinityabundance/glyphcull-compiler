//! STYL section codec (SPEC.md §2.3): resolved (flat) style records.
//!
//! Styles carry no inheritance: the compiler folds the cascade. Absent properties
//! take the documented defaults. Style id 0 is the implicit document default.

use crate::error::{Error, Result};
use crate::util::{Cursor, Writer};

/// Maximum style count (SPEC.md §1.3).
pub const MAX_STYLE_COUNT: u32 = 1 << 24;

/// Maximum properties per record (defensive).
pub const MAX_PROPERTIES_PER_STYLE: u16 = 64;

/// Property tags (SPEC.md §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum PropertyTag {
    /// `u32` index into GLYF atlases. Default 0.
    FontId = 1,
    /// `f32` px. Default 16.0.
    FontSizePx = 2,
    /// `f32` multiplier of font size. Default 1.5.
    LineHeight = 3,
    /// `u16` 100..=900. Default 400.
    FontWeight = 4,
    /// `u8` 0/1. Default 0.
    Italic = 5,
    /// `u32` RGBA. Default 0x000000FF.
    Color = 6,
    /// `u32` RGBA. Default 0x00000000.
    BackgroundColor = 7,
    /// `f32` px. Default 0.0.
    MarginTop = 8,
    /// `f32` px. Default 0.0.
    MarginBottom = 9,
    /// `u8`: 0 start, 1 center, 2 end, 3 justify. Default 0.
    TextAlign = 10,
    /// `f32` px. Default 0.0.
    TextIndent = 11,
    /// `u8`: 0 none, 1 disc, 2 circle, 3 square, 4 decimal, 5 lower_alpha,
    /// 6 upper_alpha, 7 lower_roman, 8 upper_roman. Default 0.
    ListStyle = 12,
    /// `u8` 0/1. Default 0.
    Code = 13,
    /// `u8` 0/1. Default 0.
    Underline = 14,
    /// `f32` px. Default 0.0.
    LetterSpacing = 15,
    /// `u8`: 0 normal, 1 pre, 2 nowrap. Default 0.
    WhiteSpace = 16,
}

impl PropertyTag {
    /// The wire value.
    #[must_use]
    pub const fn to_u16(self) -> u16 {
        self as u16
    }

    /// Parse a wire value; unknown tags are errors in v1.
    pub const fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::FontId),
            2 => Some(Self::FontSizePx),
            3 => Some(Self::LineHeight),
            4 => Some(Self::FontWeight),
            5 => Some(Self::Italic),
            6 => Some(Self::Color),
            7 => Some(Self::BackgroundColor),
            8 => Some(Self::MarginTop),
            9 => Some(Self::MarginBottom),
            10 => Some(Self::TextAlign),
            11 => Some(Self::TextIndent),
            12 => Some(Self::ListStyle),
            13 => Some(Self::Code),
            14 => Some(Self::Underline),
            15 => Some(Self::LetterSpacing),
            16 => Some(Self::WhiteSpace),
            _ => None,
        }
    }

    /// The encoded value width in bytes.
    #[must_use]
    pub const fn value_width(self) -> u8 {
        match self {
            Self::FontId => 4,
            Self::FontSizePx => 4,
            Self::LineHeight => 4,
            Self::FontWeight => 2,
            Self::Italic => 1,
            Self::Color => 4,
            Self::BackgroundColor => 4,
            Self::MarginTop => 4,
            Self::MarginBottom => 4,
            Self::TextAlign => 1,
            Self::TextIndent => 4,
            Self::ListStyle => 1,
            Self::Code => 1,
            Self::Underline => 1,
            Self::LetterSpacing => 4,
            Self::WhiteSpace => 1,
        }
    }

    /// Whether the value is a single byte (enum-like) property.
    #[must_use]
    pub const fn is_u8(self) -> bool {
        self.value_width() == 1
    }
}

/// A typed property value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropertyValue {
    /// 1-byte value.
    U8(u8),
    /// 2-byte value.
    U16(u16),
    /// 4-byte integer value.
    U32(u32),
    /// 4-byte float value.
    F32(f32),
}

/// One style property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StyleProperty {
    /// The property tag.
    pub tag: PropertyTag,
    /// The value.
    pub value: PropertyValue,
}

/// A resolved style record. Absent properties take documented defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleRecord {
    /// Style id (0 = implicit document default).
    pub id: u32,
    /// Properties (may be empty).
    pub properties: Vec<StyleProperty>,
}

/// The decoded STYL section.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleSection {
    /// Records with `record[i].id == i`; id 0 may be absent (implicit).
    pub styles: Vec<StyleRecord>,
}

impl StyleSection {
    /// Encode to the STYL payload. Properties are emitted in tag order.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.styles.len() as u32);
        for style in &self.styles {
            let mut props = style.properties.clone();
            props.sort_by_key(|p| p.tag);
            let mut blob = Writer::new();
            for prop in &props {
                blob.u16(prop.tag.to_u16());
                match prop.value {
                    PropertyValue::U8(v) => blob.u8(v),
                    PropertyValue::U16(v) => blob.u16(v),
                    PropertyValue::U32(v) => blob.u32(v),
                    PropertyValue::F32(v) => blob.f32(v),
                }
            }
            let blob = blob.into_bytes();
            w.u32(style.id);
            w.u16(props.len() as u16);
            w.u16(blob.len() as u16);
            w.bytes(&blob);
        }
        w.into_bytes()
    }

    /// Decode and structurally validate the STYL payload.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let style_count = c.u32("style count")?;
        if style_count > MAX_STYLE_COUNT {
            return Err(Error::LimitExceeded {
                what: "style count",
                value: u64::from(style_count),
                limit: u64::from(MAX_STYLE_COUNT),
            });
        }
        let mut styles = Vec::with_capacity(style_count as usize);
        for i in 0..style_count {
            let id = c.u32("style id")?;
            if id != i {
                return Err(Error::UnknownValue {
                    what: "style id order",
                    value: u64::from(id),
                });
            }
            let property_count = c.u16("style property count")?;
            let blob_len = usize::from(c.u16("style blob len")?);
            if property_count > MAX_PROPERTIES_PER_STYLE {
                return Err(Error::LimitExceeded {
                    what: "properties per style",
                    value: u64::from(property_count),
                    limit: u64::from(MAX_PROPERTIES_PER_STYLE),
                });
            }
            let blob = c.take(blob_len, "style blob")?;
            let mut properties = Vec::with_capacity(usize::from(property_count));
            let mut bc = Cursor::new(blob);
            for _ in 0..property_count {
                let tag_value = bc.u16("property tag")?;
                let tag = PropertyTag::from_u16(tag_value).ok_or(Error::UnknownValue {
                    what: "property tag",
                    value: u64::from(tag_value),
                })?;
                let value = match tag.value_width() {
                    1 => PropertyValue::U8(bc.u8("property value")?),
                    2 => PropertyValue::U16(bc.u16("property value")?),
                    4 => {
                        // Ambiguous between u32 and f32: disambiguate by tag.
                        let raw = bc.u32("property value")?;
                        match tag {
                            PropertyTag::FontId
                            | PropertyTag::Color
                            | PropertyTag::BackgroundColor => PropertyValue::U32(raw),
                            _ => PropertyValue::F32(f32::from_bits(raw)),
                        }
                    }
                    _ => {
                        return Err(Error::UnknownValue {
                            what: "property width",
                            value: u64::from(tag.value_width()),
                        })
                    }
                };
                properties.push(StyleProperty { tag, value });
            }
            bc.finish("style blob")?;
            styles.push(StyleRecord { id, properties });
        }
        c.finish("STYL payload")?;
        Ok(Self { styles })
    }
}

#[cfg(test)]
mod tests {
    use super::{PropertyTag, PropertyValue, StyleProperty, StyleRecord, StyleSection};

    fn sample_section() -> StyleSection {
        StyleSection {
            styles: vec![
                StyleRecord {
                    id: 0,
                    properties: vec![
                        StyleProperty {
                            tag: PropertyTag::FontId,
                            value: PropertyValue::U32(0),
                        },
                        StyleProperty {
                            tag: PropertyTag::FontSizePx,
                            value: PropertyValue::F32(16.0),
                        },
                        StyleProperty {
                            tag: PropertyTag::LineHeight,
                            value: PropertyValue::F32(1.5),
                        },
                        StyleProperty {
                            tag: PropertyTag::Color,
                            value: PropertyValue::U32(0x00_0000FF),
                        },
                    ],
                },
                StyleRecord {
                    id: 1,
                    properties: vec![
                        StyleProperty {
                            tag: PropertyTag::FontWeight,
                            value: PropertyValue::U16(700),
                        },
                        StyleProperty {
                            tag: PropertyTag::Italic,
                            value: PropertyValue::U8(1),
                        },
                    ],
                },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let section = sample_section();
        let bytes = section.encode();
        assert_eq!(StyleSection::decode(&bytes).expect("decode"), section);
    }

    #[test]
    fn properties_sorted_on_encode() {
        let mut section = sample_section();
        // Insert out of order; encode must sort by tag.
        section.styles[0].properties.push(StyleProperty {
            tag: PropertyTag::Underline,
            value: PropertyValue::U8(1),
        });
        section.styles[0].properties[0..].rotate_right(1); // Underline now first
        let bytes = section.encode();
        let decoded = StyleSection::decode(&bytes).expect("decode");
        let tags: Vec<PropertyTag> = decoded.styles[0].properties.iter().map(|p| p.tag).collect();
        assert_eq!(
            tags,
            vec![
                PropertyTag::FontId,
                PropertyTag::FontSizePx,
                PropertyTag::LineHeight,
                PropertyTag::Color,
                PropertyTag::Underline
            ]
        );
    }

    #[test]
    fn unknown_tag_rejected() {
        let section = sample_section();
        let bytes = section.encode();
        let mut corrupted = bytes;
        // First property tag: after count(4) + id(4) + pc(2) + blob_len(2) = 12.
        corrupted[12] = 0xFF;
        assert!(StyleSection::decode(&corrupted).is_err());
    }

    #[test]
    fn style_id_order_enforced() {
        let section = sample_section();
        let bytes = section.encode();
        let mut corrupted = bytes;
        // Second record id field at 12 (first blob) ... locate: header 4 + rec0
        // (4+2+2+blob_len). Compute from sample: rec0 blob = 4 props * (2+4) = 24;
        // rec0 total = 8 + 24 = 32; rec1 id at 4 + 32 = 36.
        corrupted[36] = 3;
        assert!(StyleSection::decode(&corrupted).is_err());
    }
}
