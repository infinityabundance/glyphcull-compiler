# Performance — glyphcull-compiler

Status: Phase 2 (compiler pipeline) complete. Methodology and budgets are defined; measurements
are logged in §5 as they land. GlyphCull never prematurely optimizes: build deterministic
architecture, profile, measure, optimize based on evidence.

## 1. Objectives

The compiler is batch infrastructure: throughput and bounded memory matter more than latency.
Targets (v1, confirmed by measurement in Phase 2):

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

### Phase 2 (2026-08, debug build, single-threaded, this workstation)

- `cull compile` over a ~66 KB generated Markdown document (≈ 8 000 chunks, 1 007 content
  payloads, 3 font faces, ~120 glyphs): **0.68 s** wall, 747 KB package. The 1 MB / 30 s
  budget is met by a wide margin even unoptimized; atlas generation dominates the remaining
  budget (fixed page fill + packing + MSDF per texel), as predicted in §4.
- Atlas rendering validation (Noto Sans Regular, 64 texels/em, 7 glyphs, texel-center and
  within-pixel samples): RMSE 0.023 at texel centers and ≤ 0.058 within-pixel, worst-case
  single-texel ≤ 0.43 — the committed tolerances are RMSE < 0.1 and max < 0.6 (see
  TESTING.md).
- Determinism: identical input ⇒ byte-identical packages across repeated runs (golden
  fixture test, `crates/glyphcull-pipeline/tests/golden.rs`).
- Next: profile the atlas stage (per-glyph distance + packing) and the style cascade to
  confirm the expected hot paths before any optimization work.
