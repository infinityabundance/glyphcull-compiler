# Architecture — glyphcull-compiler

Status: Phase 0 (foundations). This document is living and is updated as each subsystem
completes. See [`ROADMAP.md`](ROADMAP.md) for the phase state.

## 1. Purpose

The compiler translates human-authored documents (HTML, Markdown) into a single compiled
artifact, the `.cull` package, which is the only thing runtimes consume. It is the primary
intellectual contribution of GlyphCull. The compiler is treated the way LLVM treats IR
generation: the intermediate representations and transforms are first-class, tested,
documented artifacts.

## 2. Pipeline overview

```
                        ┌───────────────────────────┐
                        │      source document      │
                        │    HTML5 | Markdown        │
                        └─────────────┬─────────────┘
                                      ▼
                        ┌───────────────────────────┐
                        │      Semantic Graph       │  ── meaning, not presentation
                        └─────────────┬─────────────┘
                                      ▼
                        ┌───────────────────────────┐
                        │        Chunk Graph        │  ── renderable units + tree
                        └─────────────┬─────────────┘
                                      ▼
                        ┌───────────────────────────┐
                        │    Transform Pipeline     │  ── style resolution, interning,
                        │                           │     ordering, extraction, dead elim.
                        └─────────────┬─────────────┘
                                      ▼
                        ┌───────────────────────────┐
                        │       Compression         │  ── deterministic zlib (v1 sections)
                        └─────────────┬─────────────┘
                                      ▼
                        ┌───────────────────────────┐
                        │  Glyph Atlas Generation   │  ── MSDF from font outlines
                        └─────────────┬─────────────┘
                                      ▼
                        ┌───────────────────────────┐
                        │    Compiled .cull package │
                        └───────────────────────────┘
```

Stages are separated by explicit, documented data structures. No stage reaches into another
stage's representation.

## 3. Stage 1 — Source parsing (Phase 2)

Two front ends, one backend:

- **HTML5 front end**: a spec-compliant HTML5 parser (html5ever tree builder). Produces a
  normalized parse tree. The compiler then maps presentational HTML into semantic
  constructs (headings, paragraphs, lists, tables, blockquotes, code blocks, images, links,
  inline emphasis) — never passing HTML-shaped nodes downstream.
- **Markdown front end**: a spec-compliant Markdown event stream (CommonMark; tables,
  fenced code, images, inline formatting). Converges on the identical semantic model.

CSS: a documented subset of CSS is parsed (from a linked `<style>` block or a `--style`
argument) and applied during semantic construction to annotate semantic nodes with style
bindings. The runtime never sees CSS; the compiler owns translation. Media queries are out of
scope for v1 and documented as such in DESIGN.md.

Both front ends produce the **Semantic Graph**.

## 4. Stage 2 — Semantic Graph

The Semantic Graph is the compiler's meaning-level IR:

- Node types: document, heading (levels 1–6), paragraph, quote, list (ordered/unordered),
  list item, code block, table / row / cell, image, caption, link, inline run (plain,
  emphasis, strong, code, link), line break, horizontal rule.
- Each node carries: semantic type, style binding, content (text or image reference),
  structural annotations (list markers, cell spans, link targets).
- **Invariants** (property-tested):
  - Single root (document), no cycles, every node reachable.
  - Tree edges are typed; child type sets are constrained per parent (e.g., table rows only
    under table; list items only under list).
  - Text content is valid UTF-8, NFC-normalized.
  - Deterministic ordering: sibling order is total.

## 5. Stage 3 — Chunk Graph

The Chunk Graph is the renderable-unit IR. It is what the runtime walks.

- Every semantic node that produces visible content becomes a **chunk**; inline runs are
  chunks under their paragraph; structural wrappers (document, list, table) are structural
  chunks with no direct geometry.
- Chunk records are fixed-size and randomly addressable in the package (see SPEC.md, CHNK).
- Each chunk carries: id, kind, flags (hidden, keep-with-next, break-before, no-wrap),
  resolved style id, tree links (parent / prev / next / first-child / last-child), content
  index, ordinal (document order), depth.
- **Invariants** (property-tested): tree consistency (child links form a forest with one
  root; parent depth = child depth − 1); ordinals are dense and match document order;
  content indices resolve; style ids resolve.

## 6. Stage 4 — Transform pipeline

Transforms are named, ordered, pure functions over the chunk graph (or semantic graph),
each with unit tests and documented pre/post conditions:

1. `normalize` — input normalization (NFC, whitespace policy, empty-node elimination).
2. `resolve_styles` — CSS cascade folded into flat, inherited-computed styles per chunk.
   The runtime therefore implements *no* cascade; it consumes resolved styles only.
3. `partition` — semantic nodes → chunks with content extraction (text payloads, image refs).
4. `order` — deterministic ordinal and id assignment (document order).
5. `dedupe` — style interning and content deduplication.
6. `emit` — section encoding + package assembly (see Stage 7).

Transforms never render, never measure text, and never depend on viewport state. Layout is a
runtime concern; the package is layout-independent.

## 7. Stage 5/6 — Compression

- Per-section deterministic zlib (deflate, level 9), v1 sections (INFO, CHNK, STYL, CONT).
- Atlas/image sections are stored uncompressed (deterministic; SDF and raw pixels do not
  compress usefully); compression mode is recorded per section in the section table.
- Compression is byte-deterministic: same input ⇒ same compressed bytes.

## 8. Stage 7 — Glyph Atlas Generation

The compiler rasterizes glyphs into **MSDF** (multi-channel signed distance field) atlases
so runtimes can render text at arbitrary sizes from one resolution-independent asset.

- Font parsing: `ttf-parser` (memory-safe, deterministic) for outlines and metrics.
- Distance computation: exact signed distance to line and quadratic Bézier edges;
  cubic segments converted to quadratics with a documented, tested error bound.
- MSDF construction per Chlumský: three channels with edge-index tie-breaking (edge
  coloring via 3-coloring of the outline graph), pseudo-distance correction at corners.
- Atlas packing: rect packing of glyph boxes into pages; metrics table (advance, bearings,
  boxes) in em units so one atlas serves every font size.
- Verification: rasterized coverage compared against a direct reference rasterizer with
  committed tolerances (rendering validation).

## 9. Output — the `.cull` package

Assembled per [`docs/format/SPEC.md`](docs/format/SPEC.md): header, section table, sections
(INFO, CHNK, STYL, CONT, GLYF, IMGS, SEAL), CRC-32 integrity per section, content hash tree.
Byte-deterministic for identical input and toolchain.

## 10. Crate layout (compiler workspace)

```
crates/
  glyphcull-format/    Phase 1 — the contract: container, section codecs, validation
  glyphcull-semantic/  Phase 2 — Semantic Graph model + HTML/Markdown front ends
  glyphcull-chunk/     Phase 2 — Chunk Graph, transforms
  glyphcull-atlas/     Phase 2 — MSDF generation, packing
  glyphcull-cli/       Phase 2 — `cull` binary (compile/validate/inspect)
src/                   Phase 2 — pipeline orchestration, shared utilities
```

Each crate owns its tests; cross-crate integration tests live in `tests/` with committed
fixtures in `tests/fixtures/`.

## 11. Data flow and ownership

- Semantic Graph owns text; Chunk Graph borrows nothing — it references payloads by index.
- The package is the only thing that crosses the compile→runtime boundary. Nothing else.
- Memory ownership is crate-local; no global state; no interior mutability in the pipeline;
  every stage is a pure function of its input. This is what makes determinism testable.

## 12. Related documents

- [`DESIGN.md`](DESIGN.md) — rationale for every major decision.
- [`docs/format/SPEC.md`](docs/format/SPEC.md) — the byte-level contract.
- [`docs/format/GLOSSARY.md`](docs/format/GLOSSARY.md) — canonical terminology.
- [`TESTING.md`](TESTING.md), [`PERFORMANCE.md`](PERFORMANCE.md), [`SECURITY.md`](SECURITY.md).
