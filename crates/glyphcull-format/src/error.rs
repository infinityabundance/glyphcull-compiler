//! Typed errors for the `.cull` format.
//!
//! Every failure path in this crate returns a precise [`Error`] variant. The reader
//! and decoders never panic on malformed input; they return these errors.

use core::fmt;

/// A typed error produced while reading, decoding, writing, or validating a package.
///
/// The variants are stable public API: runtimes match on them to produce diagnostics.
///
/// Field-level docs are omitted deliberately: every variant is documented above and
/// the field names are self-descriptive; this is the single documented exception to
/// the crate's `missing_docs` policy.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Fewer than [`crate::header::HEADER_LEN`] bytes were provided.
    TooShort,
    /// The magic bytes are not `CULL`.
    BadMagic,
    /// The format version is not supported by this implementation.
    UnsupportedVersion(u16),
    /// The header CRC-32 does not match the header bytes.
    HeaderCrcMismatch { expected: u32, actual: u32 },
    /// `section_count` exceeds the declared limit.
    TooManySections { count: u32, limit: u32 },
    /// The section table does not fit within the provided bytes.
    TruncatedTable,
    /// A section offset/length pair exceeds the file bounds or overflows.
    OutOfBounds {
        offset: u64,
        length: u64,
        file_len: u64,
    },
    /// A section table entry carries an unsupported compression code.
    UnsupportedCompression { kind: u32, code: u8 },
    /// A reserved field or flag is non-zero (v1 is strict about reserved bits).
    ReservedBitsSet,
    /// `decoded_len` exceeds the single-section limit.
    DecodedLenExceeded { decoded_len: u64, limit: u64 },
    /// The decompressed stream length differs from the declared `decoded_len`.
    DecompressMismatch { expected: u64, actual: u64 },
    /// The decoded payload CRC-32 does not match the table entry.
    CrcMismatch {
        kind: u32,
        expected: u32,
        actual: u32,
    },
    /// More than one section with the same kind was present.
    DuplicateSection { kind: u32 },
    /// The known sections are not in canonical relative order (SPEC.md §1.6).
    InvalidSectionOrder { kind: u32, previous: u32 },
    /// An unknown section kind carries the critical flag (SPEC.md §1.2).
    UnknownCriticalSection { kind: u32 },
    /// The payload ended before the encoded structure was complete.
    UnexpectedEof { what: &'static str },
    /// Trailing bytes remained after the encoded structure was decoded.
    TrailingBytes { what: &'static str },
    /// A byte sequence was not valid UTF-8 where UTF-8 is required.
    InvalidUtf8,
    /// An enum field carried a value outside the v1 table.
    UnknownValue { what: &'static str, value: u64 },
    /// A length/value exceeded the documented v1 limit.
    LimitExceeded {
        what: &'static str,
        value: u64,
        limit: u64,
    },
    /// A property value's width did not match its tag's declared type.
    PropertyTypeMismatch { tag: u16, expected: &'static str },
    /// The SEAL section hash tree did not verify.
    SealMismatch { what: &'static str },
    /// A zlib stream could not be decoded.
    DecompressError,
    /// A zlib stream could not be encoded (in-memory encoder failure).
    CompressError,
    /// A section required by the format is missing.
    MissingSection { what: &'static str },
    /// An overflow occurred in offset/length arithmetic.
    OverflowGuard,
    /// A chunk graph reference does not resolve.
    DanglingReference { what: &'static str, id: u32 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => write!(f, "input shorter than the package header"),
            Self::BadMagic => write!(f, "bad magic: not a .cull package"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported format version {v}"),
            Self::HeaderCrcMismatch { expected, actual } => {
                write!(f, "header CRC mismatch (expected {expected:#010x}, actual {actual:#010x})")
            }
            Self::TooManySections { count, limit } => {
                write!(f, "section count {count} exceeds limit {limit}")
            }
            Self::TruncatedTable => write!(f, "section table truncated"),
            Self::OutOfBounds { offset, length, file_len } => write!(
                f,
                "section at offset {offset} with length {length} exceeds file length {file_len}"
            ),
            Self::UnsupportedCompression { kind, code } => {
                write!(f, "section kind {kind} uses unsupported compression code {code}")
            }
            Self::ReservedBitsSet => write!(f, "reserved bits are set (v1 requires zero)"),
            Self::DecodedLenExceeded { decoded_len, limit } => {
                write!(f, "decoded length {decoded_len} exceeds limit {limit}")
            }
            Self::DecompressMismatch { expected, actual } => write!(
                f,
                "decompressed length {actual} differs from declared decoded length {expected}"
            ),
            Self::CrcMismatch { kind, expected, actual } => write!(
                f,
                "section kind {kind} CRC mismatch (expected {expected:#010x}, actual {actual:#010x})"
            ),
            Self::DuplicateSection { kind } => write!(f, "duplicate section kind {kind}"),
            Self::InvalidSectionOrder { kind, previous } => write!(
                f,
                "section kind {kind} appears after {previous} (canonical order violated)"
            ),
            Self::UnknownCriticalSection { kind } => {
                write!(f, "unknown section kind {kind} marked critical (v1 rejects unknown critical sections)")
            }
            Self::UnexpectedEof { what } => write!(f, "unexpected end of input while decoding {what}"),
            Self::TrailingBytes { what } => write!(f, "trailing bytes after {what}"),
            Self::InvalidUtf8 => write!(f, "invalid UTF-8"),
            Self::UnknownValue { what, value } => write!(f, "unknown value {value} for {what}"),
            Self::LimitExceeded { what, value, limit } => {
                write!(f, "{what} value {value} exceeds limit {limit}")
            }
            Self::PropertyTypeMismatch { tag, expected } => {
                write!(f, "property tag {tag} has wrong value width (expected {expected})")
            }
            Self::SealMismatch { what } => write!(f, "SEAL verification failed: {what}"),
            Self::DecompressError => write!(f, "zlib decompression failed"),
            Self::CompressError => write!(f, "zlib compression failed"),
            Self::MissingSection { what } => write!(f, "required section missing: {what}"),
            Self::OverflowGuard => write!(f, "integer overflow in offset/length arithmetic"),
            Self::DanglingReference { what, id } => write!(f, "dangling {what} reference to id {id}"),
        }
    }
}

impl std::error::Error for Error {}

/// Convenience alias used throughout the crate.
pub type Result<T> = core::result::Result<T, Error>;
