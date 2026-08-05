//! `glyphcull-atlas` — the GlyphCull compiler's MSDF glyph atlas generator.
//!
//! Produces the resolution-independent multi-channel signed distance field atlases
//! (SPEC.md GLYF) that runtimes sample at any size: exact signed distance to line
//! and quadratic Bézier edges, bounded-error cubic→quadratic conversion, MSDF edge
//! coloring with pseudo-distance corners, deterministic rect packing, and a
//! reference rasterizer for rendering validation.
//!
//! (Modules land with the Phase 2 atlas stage.)

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
