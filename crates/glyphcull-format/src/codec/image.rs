//! IMGS section codec (SPEC.md §2.6): decoded raster images.

use crate::error::{Error, Result};
use crate::util::{Cursor, Writer};

/// Maximum image count (SPEC.md §1.3).
pub const MAX_IMAGE_COUNT: u32 = 1 << 20;

/// Maximum image dimension in pixels (SPEC.md §1.3).
pub const MAX_IMAGE_DIM: u32 = 16384;

/// Image pixel formats (SPEC.md §2.6).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ImageFormat {
    /// 4 bytes per pixel: R, G, B, A.
    Rgba8 = 0,
    /// 3 bytes per pixel: R, G, B.
    Rgb8 = 1,
}

impl ImageFormat {
    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Parse a wire value.
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Rgba8),
            1 => Some(Self::Rgb8),
            _ => None,
        }
    }

    /// Bytes per pixel.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8 => 4,
            Self::Rgb8 => 3,
        }
    }
}

/// One image (`images[i].id == i`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Pixel format.
    pub format: ImageFormat,
    /// Raw pixels, row-major, top-to-bottom, no padding.
    pub data: Vec<u8>,
}

/// The decoded IMGS section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSection {
    /// Images in dense id order.
    pub images: Vec<Image>,
}

impl ImageSection {
    /// Encode to the IMGS payload.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(self.images.len() as u32);
        for (i, image) in self.images.iter().enumerate() {
            w.u32(i as u32);
            w.u16(image.width);
            w.u16(image.height);
            w.u8(image.format.to_u8());
            w.u8(0); // flags
            w.u32(image.data.len() as u32);
            w.bytes(&image.data);
        }
        w.into_bytes()
    }

    /// Decode and structurally validate the IMGS payload.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let image_count = c.u32("image count")?;
        if image_count > MAX_IMAGE_COUNT {
            return Err(Error::LimitExceeded {
                what: "image count",
                value: u64::from(image_count),
                limit: u64::from(MAX_IMAGE_COUNT),
            });
        }
        let mut images = Vec::with_capacity(image_count as usize);
        for i in 0..image_count {
            let id = c.u32("image id")?;
            if id != i {
                return Err(Error::UnknownValue {
                    what: "image id order",
                    value: u64::from(id),
                });
            }
            let width = u32::from(c.u16("image width")?);
            let height = u32::from(c.u16("image height")?);
            let format =
                ImageFormat::from_u8(c.u8("image format")?).ok_or(Error::UnknownValue {
                    what: "image format",
                    value: 0,
                })?;
            let flags = c.u8("image flags")?;
            let byte_len = c.u32("image byte len")?;
            if flags != 0 {
                return Err(Error::ReservedBitsSet);
            }
            if width == 0 || height == 0 || width > MAX_IMAGE_DIM || height > MAX_IMAGE_DIM {
                return Err(Error::LimitExceeded {
                    what: "image dimension",
                    value: u64::from(width.max(height)),
                    limit: u64::from(MAX_IMAGE_DIM),
                });
            }
            let expected = (width as usize)
                .checked_mul(height as usize)
                .and_then(|v| v.checked_mul(format.bytes_per_pixel()))
                .ok_or(Error::LimitExceeded {
                    what: "image byte len",
                    value: u64::MAX,
                    limit: u64::MAX,
                })?;
            if byte_len as usize != expected {
                return Err(Error::LimitExceeded {
                    what: "image byte len",
                    value: u64::from(byte_len),
                    limit: expected as u64,
                });
            }
            let data = c.take(expected, "image data")?.to_vec();
            images.push(Image {
                width: width as u16,
                height: height as u16,
                format,
                data,
            });
        }
        c.finish("IMGS payload")?;
        Ok(Self { images })
    }
}

#[cfg(test)]
mod tests {
    use super::{Image, ImageFormat, ImageSection};

    fn sample() -> ImageSection {
        ImageSection {
            images: vec![
                Image {
                    width: 2,
                    height: 2,
                    format: ImageFormat::Rgba8,
                    data: vec![
                        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                    ],
                },
                Image {
                    width: 1,
                    height: 1,
                    format: ImageFormat::Rgb8,
                    data: vec![10, 20, 30],
                },
            ],
        }
    }

    #[test]
    fn round_trip() {
        let section = sample();
        let bytes = section.encode();
        assert_eq!(ImageSection::decode(&bytes).expect("decode"), section);
    }

    #[test]
    fn byte_len_must_match() {
        let mut section = sample();
        section.images[0].data.truncate(12);
        assert!(ImageSection::decode(&section.encode()).is_err());
    }

    #[test]
    fn zero_dimension_rejected() {
        let mut section = sample();
        section.images[0].width = 0;
        assert!(ImageSection::decode(&section.encode()).is_err());
    }

    #[test]
    fn id_order_enforced() {
        let section = sample();
        let bytes = section.encode();
        let mut corrupted = bytes;
        corrupted[4] = 2; // first image id
        assert!(ImageSection::decode(&corrupted).is_err());
    }
}
