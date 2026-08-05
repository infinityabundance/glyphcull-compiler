//! Deterministic zlib (RFC 1950) compression and decompression with size limits.
//!
//! The format specifies zlib deflate at a fixed level (9) and fixed strategy, which
//! is byte-deterministic: identical input produces identical output on every run and
//! platform. Decompression is always bounded by the caller-provided `decoded_len`.
//!
//! The `io` adapters (`ZlibEncoder`/`ZlibDecoder`) are used rather than the raw
//! `Compress`/`Decompress` state machines: the adapters handle buffer growth and
//! flushing correctly for arbitrary inputs, which the raw fixed-buffer API does not
//! (flate2 returns a raw error when a fixed output buffer fills mid-stream).

use std::io::{Read, Write};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::error::{Error, Result};

/// The fixed deflate level the format specifies (SPEC.md §1.5).
pub const ZLIB_LEVEL: u32 = 9;

/// Compress `data` with the format's fixed zlib settings.
///
/// Output is byte-deterministic for a given input. Fails only on encoder errors,
/// which are impossible for in-memory output in practice but are typed nonetheless.
pub fn zlib_compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(ZLIB_LEVEL));
    encoder.write_all(data).map_err(|_| Error::CompressError)?;
    encoder.finish().map_err(|_| Error::CompressError)
}

/// Decompress a zlib stream, enforcing that the output is exactly `expected_len` bytes.
///
/// Returns [`Error::DecompressError`] on malformed streams (including streams whose
/// RFC 1950 header or trailing Adler-32 checksum is invalid),
/// [`Error::DecompressMismatch`] when the output length disagrees with the declared
/// `decoded_len`. The reader caps the number of bytes read to `expected_len + 1` so
/// oversized streams are detected without unbounded allocation.
pub fn zlib_decompress(data: &[u8], expected_len: usize) -> Result<Vec<u8>> {
    verify_zlib_header(data)?;
    let decoder = ZlibDecoder::new(data);
    let mut out = Vec::with_capacity(expected_len);
    decoder
        .take((expected_len + 1) as u64)
        .read_to_end(&mut out)
        .map_err(|_| Error::DecompressError)?;
    if out.len() != expected_len {
        return Err(Error::DecompressMismatch {
            expected: expected_len as u64,
            actual: out.len() as u64,
        });
    }
    // flate2's decoder does not verify the trailing Adler-32; we verify it against
    // the decoded output so truncated stored streams are rejected. (The container
    // CRC-32 already covers decoded content; this additionally protects the stored
    // form itself.)
    verify_zlib_adler32(data, &out)?;
    Ok(out)
}

/// Validate the two-byte zlib header (RFC 1950): CMF must declare deflate and
/// `(CMF << 8 | FLG) % 31` must be zero.
fn verify_zlib_header(data: &[u8]) -> Result<()> {
    let (cmf, flg) = match data.first() {
        Some(&cmf) => (cmf, *data.get(1).ok_or(Error::DecompressError)?),
        None => return Err(Error::DecompressError),
    };
    if cmf & 0x0F != 8 {
        return Err(Error::DecompressError);
    }
    if (u16::from(cmf) << 8 | u16::from(flg)) % 31 != 0 {
        return Err(Error::DecompressError);
    }
    Ok(())
}

/// Verify the trailing Adler-32 (RFC 1950) against the decoded output.
fn verify_zlib_adler32(stored: &[u8], decoded: &[u8]) -> Result<()> {
    let tail = stored
        .get(stored.len().checked_sub(4).ok_or(Error::DecompressError)?..)
        .ok_or(Error::DecompressError)?;
    let expected = u32::from_be_bytes(tail.try_into().map_err(|_| Error::DecompressError)?);
    if expected != adler32(decoded) {
        return Err(Error::DecompressError);
    }
    Ok(())
}

/// The Adler-32 checksum (RFC 1950).
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a = 1_u32;
    let mut b = 0_u32;
    for &byte in data {
        a = (a + u32::from(byte)) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::{zlib_compress, zlib_decompress};
    use crate::error::Error;

    #[test]
    fn round_trip() {
        let data = "deterministic zlib round-trip \u{00e9}\u{00fc}\u{4e2d}\u{6587}"
            .as_bytes()
            .to_vec();
        let compressed = zlib_compress(&data).expect("compress");
        assert_eq!(
            zlib_decompress(&compressed, data.len()).expect("decompress"),
            data
        );
    }

    #[test]
    fn empty_round_trip() {
        let compressed = zlib_compress(b"").expect("compress");
        assert_eq!(zlib_decompress(&compressed, 0).expect("decompress"), b"");
    }

    #[test]
    fn deterministic_output() {
        let data = b"the same input must compress to the same bytes, every time";
        assert_eq!(
            zlib_compress(data).expect("compress"),
            zlib_compress(data).expect("compress")
        );
    }

    #[test]
    fn compressible_content_is_smaller() {
        let data = vec![b'a'; 4096];
        let compressed = zlib_compress(&data).expect("compress");
        assert!(compressed.len() < data.len());
    }

    #[test]
    fn length_mismatch_rejected() {
        let data = b"hello world".to_vec();
        let compressed = zlib_compress(&data).expect("compress");
        assert_eq!(
            zlib_decompress(&compressed, data.len() + 1).expect_err("too short declared"),
            Error::DecompressMismatch {
                expected: 12,
                actual: 11
            }
        );
        assert_eq!(
            zlib_decompress(&compressed, data.len() - 1).expect_err("too long declared"),
            Error::DecompressMismatch {
                expected: 10,
                actual: 11
            }
        );
    }

    #[test]
    fn malformed_stream_rejected() {
        // Bogus header: rejected by the RFC 1950 header check.
        assert!(zlib_decompress(b"\x00\x00\x00", 1).is_err());
        // Truncated valid stream (checksum or deflate bytes missing): rejected.
        let data = b"truncation must be caught".to_vec();
        let compressed = zlib_compress(&data).expect("compress");
        for cut in 1..=6 {
            let truncated = &compressed[..compressed.len() - cut];
            assert!(
                zlib_decompress(truncated, data.len()).is_err(),
                "cut {cut} was not rejected"
            );
        }
    }

    #[test]
    fn adler32_known_answers() {
        assert_eq!(super::adler32(b""), 1);
        assert_eq!(super::adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn oversized_stream_rejected() {
        let data = vec![0x42_u8; 2048];
        let compressed = zlib_compress(&data).expect("compress");
        assert!(matches!(
            zlib_decompress(&compressed, 1024),
            Err(Error::DecompressMismatch { .. })
        ));
    }

    #[test]
    fn large_stream_round_trip() {
        // Output larger than the internal scratch buffer exercises the loop.
        let data: Vec<u8> = (0..200_000_u32).map(|i| (i % 251) as u8).collect();
        let compressed = zlib_compress(&data).expect("compress");
        let decoded = zlib_decompress(&compressed, data.len()).expect("decompress");
        assert_eq!(decoded, data);
    }
}
