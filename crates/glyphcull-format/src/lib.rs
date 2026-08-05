//! `glyphcull-format` — reference implementation of the `.cull` document package
//! format (SPEC.md).
//!
//! This crate owns the contract:
//!
//! - **Container**: header, section table, CRC-32, deterministic zlib
//!   ([`header`], [`table`], [`section`], [`crc32`], [`compress`]).
//! - **Writer**: deterministic package assembly ([`writer::PackageBuilder`]).
//! - **Reader**: strict, bounds-checked, panic-free parsing ([`reader::parse`]).
//! - **Codecs**: the seven section payloads ([`codec`]).
//! - **Validation**: semantic cross-section checks ([`validate`]).
//!
//! The JS and Rust runtimes implement independent readers from the specification;
//! this crate is the reference both are checked against.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
// Tests may use `unwrap`/`expect`/`panic` and index freely; the production-code
// denies above apply to everything outside `#[cfg(test)]` and the integration
// test binaries.
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod codec;
pub mod compress;
pub mod crc32;
pub mod error;
pub mod header;
pub mod reader;
pub mod section;
pub mod table;
pub mod util;
pub mod validate;
pub mod writer;

pub use error::{Error, Result};
pub use header::{Header, VERSION};
pub use reader::{parse, DecodedSection, ParsedPackage};
pub use section::SectionKind;
pub use table::{Compression, SectionEntry};
pub use validate::ValidationIssue;
