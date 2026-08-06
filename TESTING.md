# Testing — glyphcull-compiler

Status: Phase 0 (foundations). The test pyramid below is the target for every phase; each
phase lands its slice of the pyramid before it is considered complete.

## 1. Principles

- Every subsystem ships with unit, integration, stress, and (where appropriate) property
  tests.
- Every bug receives a permanent regression test.
- Tests must be deterministic: no wall-clock dependencies, no network, no unordered iteration.
- Malformed input tests assert *typed errors*, never panics (panic-assertions use
  `std::panic::catch_unwind` in the malformed corpus).
- Golden artifacts (packages, hex dumps, reference images) are committed and regenerated only
  through a checked-in script with an explicit, reviewed diff.

## 2. Layers

### Unit (per crate, `#[cfg(test)]` in each module)

- CRC-32 known-answer vectors (empty, `"123456789"`, RFC 1952 examples) + incremental
  equivalence.
- Header / section-table encode/decode round-trips; corruption detection (bad magic, bad
  version, bad CRC, truncated table, out-of-bounds offsets, overflow in offset+length).
- Compression: round-trip; byte-determinism (same input ⇒ identical zlib output); output size
  limits enforced on decompress.
- Codecs: round-trip per section kind; strict rejection of unknown tags/kinds/values;
  UTF-8 validation for text payloads; length validation for image payloads.
- Transforms (Phase 2): each transform's pre/post conditions; identity on canonical input.
  Implemented: whitespace collapse (style-ownership preserving, line-end rules), image
  hoisting (link-target transfer, paragraph splitting), chunk partition (document root,
  per-node cascade, multi-run link extras).
- MSDF (Phase 2): distance field correctness vs analytic references; edge coloring validity
  (adjacent edges differ in channel, two-bit masks); corner behavior via the median
  reconstruction; atlas packing invariants (no overlap, all glyphs present, bounds within
  page); error-correction pass (idempotence, inversion detection, shape-improvement test);
  proptest properties for the exact distance functions (curvature-aware sampling bound,
  line-distance closed form).

### Integration (`tests/`)

- Writer → reader round-trip over a synthetic full-featured document (every chunk kind, every
  style property, images, kerning).
- `cull validate` rejects structurally valid but semantically broken packages (dangling
  parent links, non-dense ordinals, cycle) with precise diagnostics.
- `cull inspect` output stability (golden text fixture).
- Golden package fixtures committed under `tests/fixtures/` with a regeneration script
  (`scripts/regenerate-golden.sh`) that refuses to run on a dirty tree.
- End-to-end compiler tests: `cull compile` → `cull validate` over Markdown and HTML
  (golden byte-exact fixture + stress documents, see `glyphcull-pipeline/tests/`).

### Property (`proptest`)

- Serialization round-trip: `decode(encode(x)) == x` over generated section models.
- Tree invariants on generated chunk graphs: single root, acyclicity, reachability,
  depth = parent depth + 1, dense ordinals.
- Determinism: two encodes of equal inputs are byte-equal.
- Bounds: decode of arbitrary bytes never panics and either errors or round-trips
  (`decode∘encode` is total on the preimage of encode).

### Stress

- Maximum-size sections at declared limits (SPEC.md limits); 10k-chunk documents; wide tables;
  long unbreakable runs; deep nesting; all-glyph atlas; pathological packing input.
  Assert bounded time and memory (see PERFORMANCE.md budgets).

### Malformed corpus

- Every proper prefix of a valid package ⇒ `Err` (truncation); single-bit flips across a
  package ⇒ either `Err` or a *valid* package (i.e., corruption is either detected or
  innocuous — flips in content bytes are permitted to produce different valid content, since
  content is arbitrary; flips in structure must be detected).
- Fuzz harness (cargo-fuzz) for the reader, documented in `fuzz/`.

### Performance regression

- `cargo bench` (criterion) with committed baselines in `benches/baselines/`; ratio checks
  (e.g., compile time scales near-linearly in chunk count within a tolerance band). See
  PERFORMANCE.md.

### Memory regression

- Peak and retained allocation measurements per pipeline stage via a test-only counting
  allocator; assert bounds (e.g., peak live bytes < K × decoded package size).

### Rendering validation (Phase 2)

- MSDF-reconstructed coverage compared against a direct supersampled reference rasterizer
  for a real-glyph corpus (Noto Sans Regular: A H O V g p q) at 64 texels/em, at texel
  centers and within-pixel bilinear offsets. Committed tolerances: RMSE < 0.1, worst-case
  single-texel error < 0.6 (measured ≈ 0.023 / 0.43); the residual error concentrates in the
  one-texel boundary band and at corners — the inherent limit of 8-bit MSDF at 1:1.

### Package validation

- `cull validate` implements SPEC.md reader rules + semantic cross-checks (style/font/content
  reference resolution, chunk tree integrity); validated by the golden + corruption suites.

## 3. Tooling

- `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo bench`,
  proptest (`proptest!`), cargo-fuzz (documented; run in CI on a schedule).
- Test helper crate `glyphcull-testkit` (dev-only) for fixture loading and byte utilities.

## 4. CI

- Workflow: fmt → clippy → test (all targets) → bench smoke (short iteration) → doc build.
- Golden fixtures are verified byte-exact in CI (regeneration script must produce no diff).
