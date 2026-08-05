# Performance — glyphcull-compiler

Status: Phase 0 (foundations). Methodology and budgets are defined now; measurements land with
each phase. GlyphCull never prematurely optimizes: build deterministic architecture, profile,
measure, optimize based on evidence.

## 1. Objectives

The compiler is batch infrastructure: throughput and bounded memory matter more than latency.
Targets (v1, to be confirmed by measurement in Phase 2):

- Compile a 1 MB Markdown document (≈ 150k words) in well under 30 s on a reference
  workstation (2020-era 8-core), single-threaded; multi-threaded within a phase only if
  measurement shows single-thread is insufficient and determinism is preserved (deterministic
  fork-join with stable output order — no parallel unordered emission).
- Peak memory < 16 × decoded package size during pipeline execution (measured, not assumed).
- Atlas generation dominates; its throughput target is documented in `glyphcull-atlas`.

## 2. Methodology

- Benchmarks are committed under `benches/` (criterion). Baselines are committed in
  `benches/baselines/` as machine-relative numbers (per-byte costs, ratios), not absolute
  wall-clock, because CI machines vary.
- Regression checks are ratio-based: e.g., `t(N)/t(N/2) ∈ [1.7, 2.3]` for linear scaling
  expectations; absolute wall-clock regressions are reported, not failed, on non-pinned CI.
- Memory is measured with a test-only counting allocator (increment/decrement per
  allocation) so regressions are deterministic and reported in bytes, not RSS.
- Every optimization must be (a) motivated by a profile, (b) behavior-preserving
  (byte-identical output — verified by the determinism suite), and (c) covered by the
  performance regression tests.

## 3. Budgets (v1, confirmed by measurement)

| Metric | Budget |
|---|---|
| Compile: 1 MB Markdown → `.cull` | < 30 s, single-threaded reference |
| Compile: atlas for 2 font faces × 512 glyphs | < 10 s |
| Peak pipeline memory | < 16 × decoded package size |
| Determinism | byte-identical across runs, platforms, thread counts |

## 4. Known hot paths (to profile in Phase 2, not optimize blind)

- HTML5 tree building (html5ever) — expected linear in input size.
- MSDF distance computation — expected dominant; algorithmic work is bounded by
  (pixels per glyph × edges per glyph); packing order affects cache locality.
- Style resolution — one pass; interning keeps the style table small.
- zlib compression of CONT (text) — expected linear; level 9 is fixed for determinism.

## 5. Evidence log

To be appended as phases land: profile command, machine, measurements, and decisions taken.
