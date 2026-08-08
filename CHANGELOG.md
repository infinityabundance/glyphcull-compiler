# Changelog

All notable changes to this repository are recorded here, in reverse chronological order.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed (HTML table captions compile valid packages + readable self-validation)

- The HTML parser mapped `figcaption` but not `<caption>` (the HTML table-caption
  element) to `SemanticKind::Caption`; a `<caption>` became a transparent-with-text
  node, which the chunker wrapped into a `Paragraph` under the `Table` — and the
  validator rejects that (`Table => TableRow only`). The Linux-wikipedia demo page
  (a 5805-chunk article with 4 tables, 1 caption) hit this and failed the compiler's
  own self-validation. The parser now maps `"figcaption" | "caption"` and the
  validator allows `Table => TableRow | Caption` (SPEC.md §2.2). Regression tests:
  `html.rs` `table_caption_is_a_caption_node`, `validate.rs`
  `table_caption_child_is_allowed`.
- The self-validation error surfaced the failure as `UnknownValue 0` — opaque. The
  pipeline now reports `Error::Validation { detail }` listing up to 8 issues with
  their section names, so a bad compile says what is wrong instead of a magic
  number.

### Fixed (MSDF sign convention — the canonical direction, addendum A–M)

- The atlas generator (`glyphcull-atlas/src/sdf.rs` + the exact-distance
  helper in `correction.rs`) encoded the sign opposite to the canonical
  convention (SPEC.md §2.5): inside ≈ 0.0, outside ≈ 1.0. Every decoder (JS
  `render/msdf.ts`, the WebGL shader, the Canvas 2D fallback, the Rust wgpu
  shader + CPU reconstruction, the demo reference compositor) already decoded
  `(median − 0.5)` with positive = inside, so the inverted atlas rendered
  glyph interiors as holes and quad exteriors as solid rectangles. The
  compiler's own reference reconstruction (`raster.rs`) contained the
  compensating `1.0 − smooth`, which is why the internal validation passed.
- The generator now encodes inside > 0.5 / outside < 0.5 (canonical);
  `raster.rs` returns `smooth` directly (no inversion). The atlas unit tests
  were updated to assert the canonical direction.
- Full evidence, the bug ledger, and the regression coverage are documented
  in `glyphcull-demo/docs/rendering/MSDF-SIGN-CONVENTION.md`; the SPEC §2.5
  sign-convention paragraph and the GLOSSARY entry are normative now.

### Added (release receipts, hardening pass H7)

- `release/` — the receipt system: `templates/receipt.template.json` (the schema),
  `scripts/generate-release-receipt.sh` (dirty-tree-refusing; records commit,
  deterministic source-tree hash, the real `cargo package` tarball hash, toolchain,
  commands, results, UTC timestamp), `scripts/check-release-receipts.sh` (`--fast` CI
  gate; `--full` recomputes every package hash from a git worktree of its recorded
  commit — proving reproducibility), and `scripts/release-dry-run.sh` (all seven
  crates assemble in release order).
- `release/receipts/` — committed receipts for all seven published crates
  (`glyphcull-format` 0.1.2, `glyphcull-semantic`/`glyphcull-chunk`/`glyphcull-atlas`/
  `glyphcull-pipeline` 0.1.0, `glyphcull-cli` 0.1.0/0.2.0, `glyphcull` 0.2.0) with
  build/test/conformance/package gates recorded as pass (full gates re-run at
  generation).
- The umbrella crate's `exclude` list now covers `release/**` (repo metadata is not
  crate content).
- CI: the `gate` job runs `release/scripts/check-release-receipts.sh --fast`.
  `release/README.md` documents the release order and the receipt contract.

### Changed (the `.cull` v1 compatibility policy, hardening pass H6)

- `docs/format/SPEC.md` §4 is now the full compatibility policy: format versioning
  (§4.1), reader behavior (§4.2), writer behavior (§4.3), required/optional sections
  (§4.4), unknown-section handling and the critical bit (§4.5), forward and backward
  compatibility (§4.6/§4.7), the explicit **within-v1 vs v2 change table** (§4.8),
  canonical serialization + determinism (§4.9), rejection requirements (§4.10),
  security/resource limits (§4.11), compression/checksum/SEAL rules (§4.12), and
  experimental-section rules (§4.13). The spec header now reads **"v1 — locked"**;
  the change is recorded in §7.
- No code changes: the rules were implemented in hardening pass H2 (the critical bit,
  canonical order, required INFO — `glyphcull-format` 0.1.2) and are proven by the
  nine-case v1-compatibility matrix in `glyphcull-demo/conformance/` (valid fixtures
  + `future-minor` + hostile `unknown-critical-section` / `bad-version` /
  `bad-compression` / `oversized-section` / `bad-crc` / `bad-seal`), which CI runs
  on every push.

### Changed (README status correction, hardening pass H4)

- The README now leads with the tagline **"A compiled GPU document runtime."** and a
  status block: v0.1 experimental infrastructure prototype; Latin-script per-codepoint
  rendering only (complex shaping, bidi, vertical text, Indic/Arabic scripts, and full
  international publishing are documented exclusions); not DRM; does not make scraping
  impossible — it raises the cost of ordinary DOM-based extraction. Added the CI badge
  and the conformance-suite link.

### Fixed (glyphcull-atlas — the glyph packer)

- Two correctness bugs in the MSDF atlas packer, found by the hardening-pass CI
  smoke (a plain Markdown document failed the compiler's own self-validation
  with an out-of-page glyph box):
  - `pack::insert` dropped the skyline's tail when the raised overlap merged
    into the previous raised run (a stray `continue`), corrupting the envelope
    so a later rect could straddle the gap and overflow the page.
  - `build_atlas` keyed the placement map on the enumerate `index` over `work`
    instead of `g.order`; `work` skips codepoints with no glyph (e.g. U+000A,
    absent from the bundled Noto Sans), so past the first missing codepoint
    every glyph was recorded at another glyph's box.
  Regression tests: `insert_preserves_full_coverage`,
  `pack_stays_in_page_with_merges`, and
  `records_map_to_their_own_placements_with_missing_codepoints`. `golden.cull`
  regenerated (the old bytes encoded the buggy layout; deliberate golden
  change).

### Changed (0.1.1 republish — `glyphcull-format`)

- `glyphcull-format` 0.1.1: the published 0.1.0 validator rejected `quote →
  paragraph` chunk trees, but the compiler emits them (quote blocks contain
  paragraph children). The HEAD validator (fixed in the semantic-graph-fidelity
  commit) allows block children under `Quote`; this republish brings crates.io
  in line with the tree. Also includes an intra-doc-link cleanup in `util.rs`.
  Downstream requirement `0.1.0` (caret) resolves to 0.1.1, so no other crate
  needs a version bump.

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
