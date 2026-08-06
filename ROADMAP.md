# Roadmap — glyphcull-compiler

Execution order is mandatory and matches the master plan in the root `PLAN.md`. Phases are
never reordered, merged, deferred, or skipped.

## Phase 1 — The contract (complete)

`crates/glyphcull-format` — the `.cull` format reference implementation.

- [x] Container: header, section table, CRC-32 (in-repo, known-answer tested), zlib v1 sections (flate2).
- [x] Section codecs: INFO, CHNK, STYL, CONT, GLYF, IMGS, SEAL per `docs/format/SPEC.md`.
- [x] Strict bounds-checked reader; no panics on malformed input; truncation corpus tests.
- [x] Deterministic writer; golden minimal-package byte vector (hand-pinned).
- [x] `cull-validate` (structural + semantic) and `cull-inspect` (diagnostics).
- [x] Test pyramid: unit, golden, round-trip, malformed corpus, determinism, property tests.
- [x] `docs/format/SPEC.md` finalized against the implementation; both runtimes will implement
      independent readers from the spec.

## Phase 2 — Compiler pipeline

- [x] `glyphcull-semantic`: Semantic Graph model + invariants; HTML5 front end; Markdown front end.
- [x] CSS subset parser + style binding (cascade into annotations).
- [x] `glyphcull-chunk`: Chunk Graph, transforms (`normalize`, `resolve_styles`, `partition`, `order`, `dedupe`, `emit`).
- [x] `glyphcull-atlas`: font parsing (ttf-parser), exact quadratic distance, MSDF coloring +
      pseudo-distance, cubic→quadratic conversion with error bounds, rect packing, metrics table.
- [x] Atlas verification: reference rasterizer comparison with committed tolerances.
- [x] `glyphcull-cli`: `cull compile|validate|inspect`, exit codes, diagnostics.
- [x] Determinism suite, golden packages, stress tests, memory regression tests.
- [x] Documentation: pipeline architecture (Architecture.md), design rationale (DESIGN.md),
      transform docs, data flow diagrams.

## Future phase candidates (recorded, not scheduled; each requires its own design pass)

- Font subsetting + multiple-font atlas merging heuristics.
- Search index section (IDXM).
- Signature section for package authenticity (SEAL extension).
- CSS media queries / responsive layouts (requires runtime contract extension).
- Footnotes, cross-references, citations (semantic graph extension).
- Hyphenation dictionaries (layout-affecting; runtime contract).

## Definition of done (applies to every phase)

- All tests green (`cargo test`), clippy clean (`cargo clippy -- -D warnings`), formatted
  (`cargo fmt --check`).
- Documentation updated in lockstep (this repo's seven mandated docs + per-subsystem docs).
- No `TODO`, `FIXME`, `unimplemented!`, `todo!`, or placeholder code.
- Performance and memory regression baselines recorded (see PERFORMANCE.md).
- Every bug fixed during the phase has a permanent regression test.
