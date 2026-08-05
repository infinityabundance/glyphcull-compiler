//! `glyphcull-pipeline` — the GlyphCull compiler pipeline.
//!
//! Orchestrates the deterministic end-to-end compile: source (HTML/Markdown) →
//! Semantic Graph (glyphcull-semantic) → Chunk Graph + resolved styles
//! (glyphcull-chunk) → MSDF glyph atlases (glyphcull-atlas) → a compiled `.cull`
//! package (glyphcull-format). Same input + same toolchain ⇒ identical bytes.
//!
//! (Modules land with the Phase 2 pipeline orchestration.)

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
