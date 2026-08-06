# Changelog

All notable changes to this repository are recorded here, in reverse chronological order.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed (0.2.0 republish — `glyphcull-cli`, `glyphcull` umbrella)

- `glyphcull-cli` 0.2.0: the previously published 0.1.0 artifact predated Phase 2
  (the `cull compile` subcommand); the current tree republishes at 0.2.0 (SemVer
  minor for the new command). `glyphcull` (umbrella) 0.2.0 follows with the
  pipeline re-exports and the updated CLI dependency.

### Added (Phase 0 — Foundations)

- Repository scaffolding: README, Architecture.md, DESIGN.md, ROADMAP.md, TESTING.md,
  PERFORMANCE.md, SECURITY.md, CONTRIBUTING.md, LICENSE (Apache-2.0), CHANGELOG,
  .gitignore, .editorconfig.
- Canonical format specification (`docs/format/SPEC.md`) and terminology standard
  (`docs/format/GLOSSARY.md`).

### Added (Phase 1 — The contract)

- `glyphcull-format` crate: container (header, section table, in-repo CRC-32,
  deterministic zlib), the seven section codecs (INFO/CHNK/STYL/CONT/GLYF/IMGS/SEAL),
  strict bounds-checked reader (never panics), deterministic writer with optional SEAL,
  semantic validation (`validate_package`).
- `cull validate` and `cull inspect` CLI subcommands.
- Golden tests: hand-pinned minimal package byte vector, committed fixture
  (`tests/fixtures/v1-minimal.cull`), regeneration/verification scripts.
- Malformed corpus: every-truncation and byte-flip suites; forward-compat unknown-section
  test; proptest properties (round-trip, totality, determinism, tree validity).
- SPEC.md finalized against the implementation (1-based chunk ids and content_index,
  non-circular SEAL overall hash; see SPEC.md §7 History).

### Added (Phase 2 — Compiler pipeline)

- `glyphcull-semantic` crate: the Semantic Graph (NFC boundary, bounded depth/arity,
  shape invariants) with an HTML5 front end (custom html5ever `TreeSink`, CSS extraction,
  metadata) and a Markdown front end (pulldown-cmark, CommonMark scope); a strict CSS
  subset parser; whitespace normalization that preserves style ownership; deterministic
  inline-image hoisting with link-target transfer.
- `glyphcull-chunk` crate: the Chunk Graph partition (SPEC.md CHNK) with the CSS cascade
  folded into flat resolved styles (defaults owned by a reviewed built-in stylesheet),
  per-node style resolution for inline content, multi-run link extras, hoisted image
  links, codepoint-per-face collection.
- `glyphcull-atlas` crate: exact MSDF generation — exact signed distance to line and
  quadratic Bézier segments, bounded-error cubic→quadratic conversion, msdfgen-style
  two-bit edge coloring (the median-reconstruction property), a faithful translation of
  msdfgen's error-correction pass, deterministic skyline packing, GPOS PairPos
  (formats 1 & 2) + kern kerning extraction, supersampled reference rasterization with
  committed rendering-validation tolerances, proptest geometry properties.
- `glyphcull-pipeline` crate: the deterministic end-to-end compiler — front ends → chunk
  graph → per-face MSDF atlases (adaptive page sizing) → PNG/JPEG decode → INFO/CHNK/
  STYL/CONT/GLYF/IMGS/SEAL assembly with content-addressed `document_id`, bundled Noto
  Sans registry, self-validating output, golden fixtures, stress tests.
- `cull compile <input> [-s style.css]... [-o out.cull]` CLI subcommand.
- SPEC.md clarification: `document_id` is computed over the decoded content sections
  (non-circular; see SPEC.md §7 History).
