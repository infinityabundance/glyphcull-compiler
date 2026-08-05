//! `glyphcull-semantic` — the GlyphCull compiler's Semantic Graph and its front
//! ends.
//!
//! The Semantic Graph is the compiler's meaning-level IR: both the HTML and
//! Markdown front ends converge on it, and it encodes *what a document means*
//! (headings, paragraphs, lists, tables, quotes, images, links, inline emphasis)
//! rather than how it is presented. Style enters as hints (classes, ids, inline
//! declarations) that the style resolver folds into flat computed styles.
//!
//! ```text
//! HTML ─┐
//!       ├─▶ Semantic Graph ──▶ (glyphcull-chunk: Chunk Graph + transforms)
//! Markdown ─┘
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

pub mod css;
pub mod dom;
pub mod html;
pub mod markdown;
pub mod model;

pub use html::{parse_html, FrontEndOutput};
pub use markdown::parse_markdown;
pub use model::{
    normalize_nfc, validate_tree, SemanticKind, SemanticNode, StyleHints, MAX_CHILDREN, MAX_DEPTH,
};
