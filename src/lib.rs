//! GlyphCull — a compiled GPU document runtime.
//!
//! This is the **umbrella crate** for the GlyphCull compiler workspace. GlyphCull
//! treats documents as compiled assets: HTML and Markdown are compiled once into a
//! `.cull` package (the contract), which runtimes stream, materialize, and paint on
//! the GPU. The browser is one execution host; the format is the contract, nothing
//! else.
//!
//! ```text
//! HTML / Markdown
//!         ↓  (glyphcull compile)
//!   .cull package   ←── the contract (SPEC.md)
//!         ↓
//!   runtimes (glyphcull-runtime-js / -rs)
//! ```
//!
//! This crate re-exports the workspace:
//!
//! - [`glyphcull_format`] — the `.cull` format reference implementation: the
//!   container (header, section table, CRC-32, deterministic zlib), the seven
//!   section codecs, the strict panic-free reader, the deterministic writer, and
//!   semantic validation.
//! - [`glyphcull_cli`] — the `cull` command-line tool (validate/inspect today;
//!   `compile` with the Phase 2 pipeline).
//!
//! # Example
//!
//! ```
//! use glyphcull::glyphcull_format::section::SectionKind;
//! use glyphcull::glyphcull_format::table::Compression;
//! use glyphcull::glyphcull_format::writer::PackageBuilder;
//! use glyphcull::glyphcull_format::parse;
//!
//! let mut builder = PackageBuilder::new().with_seal(true);
//! builder.add(SectionKind::Content, b"hello".to_vec(), Compression::Zlib).expect("add");
//! let bytes = builder.build().expect("build");
//! let package = parse(&bytes).expect("valid package");
//! assert_eq!(package.header.section_count, 2);
//! ```

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

/// The `.cull` format reference implementation (container, codecs, reader, writer, validation).
pub use glyphcull_format;

/// The `cull` command-line tool.
pub use glyphcull_cli;

/// The current `.cull` format version.
pub use glyphcull_format::VERSION;

/// Parse a `.cull` package (see [`glyphcull_format::parse`]).
pub use glyphcull_format::parse;

/// Validate a parsed package (see [`glyphcull_format::validate`]).
pub use glyphcull_format::validate;
