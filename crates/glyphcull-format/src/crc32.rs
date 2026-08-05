//! CRC-32 (IEEE 802.3, reflected polynomial `0xEDB88320`).
//!
//! Implemented in-repo because the format specifies the primitive itself; the
//! implementation is table-driven and covered by known-answer tests (RFC 1952 /
//! PNG test vectors). Initial value `0xFFFFFFFF`, final XOR `0xFFFFFFFF`.

/// The reflected CRC-32 polynomial used by zlib, PNG, and this format.
const POLYNOMIAL: u32 = 0xEDB8_8320;

/// Precomputed table of 256 reflected remainders.
const TABLE: [u32; 256] = make_table();

/// Build the CRC-32 lookup table at compile time.
/// `i` ranges 0..256, so the writes are provably in bounds.
#[allow(clippy::indexing_slicing)]
const fn make_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    let mut i = 0_usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0_u32;
        while k < 8 {
            c = if c & 1 != 0 {
                POLYNOMIAL ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// Compute the CRC-32 of `data`.
///
/// # Examples
///
/// ```
/// use glyphcull_format::crc32::crc32;
/// assert_eq!(crc32(b""), 0x0000_0000);
/// assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
/// ```
// The table index is masked to 0..=255, so the access is provably in bounds.
#[allow(clippy::indexing_slicing)]
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFF_u32;
    for &byte in data {
        c = TABLE[((c ^ u32::from(byte)) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

/// An incremental CRC-32 accumulator, equivalent to one-shot [`crc32`].
///
/// Useful for streaming validation without materializing a concatenated buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Crc32 {
    state: u32,
}

impl Crc32 {
    /// Create an accumulator in the standard initial state.
    #[must_use]
    pub const fn new() -> Self {
        Self { state: 0xFFFF_FFFF }
    }

    /// Feed `data` into the accumulator.
    #[allow(clippy::indexing_slicing)]
    pub fn update(&mut self, data: &[u8]) {
        let mut c = self.state;
        for &byte in data {
            c = TABLE[((c ^ u32::from(byte)) & 0xFF) as usize] ^ (c >> 8);
        }
        self.state = c;
    }

    /// Finish and return the CRC-32.
    #[must_use]
    pub const fn finalize(&self) -> u32 {
        !self.state
    }
}

#[cfg(test)]
mod tests {
    use super::{crc32, Crc32};

    #[test]
    fn known_answers() {
        // Canonical vectors (RFC 1952 check value; PNG spec; common practice).
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"abc"), 0x3524_41C2);
    }

    #[test]
    fn incremental_equals_oneshot() {
        let data = b"the incremental path must agree with the one-shot path";
        let mut acc = Crc32::new();
        for chunk in data.chunks(3) {
            acc.update(chunk);
        }
        assert_eq!(acc.finalize(), crc32(data));
        assert_eq!(acc.finalize(), crc32(data));
    }

    #[test]
    fn empty_incremental() {
        assert_eq!(Crc32::new().finalize(), 0x0000_0000);
    }
}
