//! Raster image decoding (PNG, JPEG) for the IMGS section.
//!
//! The compiler decodes source images at compile time (SPEC.md §2.6: runtimes
//! never decode image formats). Only PNG and JPEG are supported in v1; output
//! is RGBA8 (PNG with an alpha channel) or RGB8. Dimensions are validated
//! against the SPEC limit (≤ 16384 px per side).

use glyphcull_format::codec::image::{Image, ImageFormat};

/// An image decode failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// The input is neither a PNG nor a JPEG.
    UnsupportedFormat,
    /// The decoder rejected the input.
    DecodeFailed,
    /// The image exceeds the SPEC dimension limit.
    TooLarge,
    /// The image has no pixels.
    Empty,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::UnsupportedFormat => write!(f, "unsupported image format (PNG/JPEG only)"),
            ImageError::DecodeFailed => write!(f, "image decode failed"),
            ImageError::TooLarge => write!(f, "image exceeds the 16384 px dimension limit"),
            ImageError::Empty => write!(f, "image has no pixels"),
        }
    }
}

impl std::error::Error for ImageError {}

/// The SPEC's maximum image dimension in pixels (§1.3).
pub const MAX_IMAGE_DIMENSION: u32 = 16_384;

/// Decode an image into the IMGS wire form.
pub fn decode(bytes: &[u8]) -> Result<Image, ImageError> {
    if is_png(bytes) {
        decode_png(bytes)
    } else if is_jpeg(bytes) {
        decode_jpeg(bytes)
    } else {
        Err(ImageError::UnsupportedFormat)
    }
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8])
}

/// Validate the dimensions against the SPEC limit.
fn check_dims(width: u32, height: u32) -> Result<(u16, u16), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::Empty);
    }
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(ImageError::TooLarge);
    }
    Ok((width as u16, height as u16))
}

fn decode_png(bytes: &[u8]) -> Result<Image, ImageError> {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().map_err(|_| ImageError::DecodeFailed)?;
    let mut info = reader.info().clone();
    // Expand all inputs to 8-bit channels.
    info.bit_depth = png::BitDepth::Eight;
    let has_alpha = matches!(
        info.color_type,
        png::ColorType::Rgba | png::ColorType::GrayscaleAlpha
    );
    let mut buf = vec![0_u8; reader.output_buffer_size()];
    let out_info = reader
        .next_frame(&mut buf)
        .map_err(|_| ImageError::DecodeFailed)?;
    let (w, h) = check_dims(out_info.width, out_info.height)?;
    let len = usize::from(w) * usize::from(h) * if has_alpha { 4 } else { 3 };
    let data = buf.get(..len).ok_or(ImageError::DecodeFailed)?.to_vec();
    Ok(Image {
        width: w,
        height: h,
        format: if has_alpha {
            ImageFormat::Rgba8
        } else {
            ImageFormat::Rgb8
        },
        data,
    })
}

fn decode_jpeg(bytes: &[u8]) -> Result<Image, ImageError> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    let pixels = decoder.decode().map_err(|_| ImageError::DecodeFailed)?;
    let info = decoder.info().ok_or(ImageError::DecodeFailed)?;
    let (w, h) = check_dims(info.width as u32, info.height as u32)?;
    // jpeg-decoder always yields RGB (3 channels per pixel).
    let expected = usize::from(w) * usize::from(h) * 3;
    if pixels.len() != expected {
        return Err(ImageError::DecodeFailed);
    }
    Ok(Image {
        width: w,
        height: h,
        format: ImageFormat::Rgb8,
        data: pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny 2×1 RGBA PNG (red, transparent).
    const PNG_RGBA: &[u8] = include_bytes!("../assets/test/2x1-rgba.png");
    /// A tiny 1×1 RGB JPEG (black).
    const JPEG_RGB: &[u8] = include_bytes!("../assets/test/1x1-rgb.jpg");

    #[test]
    fn decodes_png_rgba() {
        let img = decode(PNG_RGBA).expect("png");
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.format, ImageFormat::Rgba8);
        assert_eq!(img.data.len(), 8);
        // First pixel red with alpha 255.
        assert_eq!(&img.data[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn decodes_jpeg_rgb() {
        let img = decode(JPEG_RGB).expect("jpeg");
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(img.format, ImageFormat::Rgb8);
        assert_eq!(img.data.len(), 3);
    }

    #[test]
    fn rejects_unknown_format() {
        assert_eq!(decode(b"not an image"), Err(ImageError::UnsupportedFormat));
        assert_eq!(decode(&[]), Err(ImageError::UnsupportedFormat));
    }

    #[test]
    fn rejects_garbage_signatures() {
        // A PNG signature with truncated data.
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0; 8]);
        assert!(matches!(decode(&bytes), Err(ImageError::DecodeFailed)));
    }
}
