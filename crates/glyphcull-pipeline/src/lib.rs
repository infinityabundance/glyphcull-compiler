//! `glyphcull-pipeline` — the GlyphCull compiler pipeline.
//!
//! Orchestrates the deterministic end-to-end compile: source (HTML/Markdown) →
//! Semantic Graph (glyphcull-semantic) → Chunk Graph + resolved styles
//! (glyphcull-chunk) → MSDF glyph atlases (glyphcull-atlas) → decoded images →
//! a compiled `.cull` package (glyphcull-format). Same input + same options ⇒
//! identical bytes.
//!
//! # Determinism
//!
//! - Faces are processed in sorted order (font ids are positional in that
//!   order); styles reference faces by the resolved font id.
//! - Sections are emitted in canonical order (`INFO, CHNK, STYL, CONT, GLYF,
//!   IMGS, SEAL`); the SEAL is computed by the format writer.
//! - `source_digest` is the SHA-256 of the raw source; `document_id` is the
//!   first 16 bytes of the SHA-256 over the decoded content sections
//!   (CHNK..IMGS, canonical order) — content-addressed and non-circular.
//! - No timestamps, randomness, or environment-dependent bytes.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod fonts;
pub mod images;
pub mod pipeline;

pub use fonts::{FontError, FontRegistry, DEFAULT_FAMILY};
pub use images::{decode as decode_image, ImageError};
pub use pipeline::{compile, CompileOptions, CompileReport, Error, InputKind};
