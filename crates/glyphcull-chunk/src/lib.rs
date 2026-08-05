//! `glyphcull-chunk` — the GlyphCull compiler's Chunk Graph and transform
//! pipeline.
//!
//! The Chunk Graph is the renderable-unit IR that runtimes walk: semantic nodes
//! become chunks with flat resolved styles, content payloads, and tree links
//! (SPEC.md CHNK). This crate also owns style resolution (the CSS cascade folded
//! into flat computed styles — the runtime never sees CSS) and the deterministic
//! partition transform that normalizes, styles, orders, and dedupes.
//!
//! ```text
//! Semantic Graph ──▶ resolve styles (cascade) ──▶ partition ──▶ Chunk Graph
//! ```

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

pub mod graph;
pub mod styles;

pub use graph::{build_chunk_model, ChunkModel, ImageSource};
pub use styles::{resolve_node, resolve_styles, ruleset, FaceKey, ResolvedStyle, StyleTable};
