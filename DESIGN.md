# Design — glyphcull-compiler

Status: Phase 0 (foundations). Design decisions are recorded here as they are made and are
final for v1 once implemented. Every decision records rationale, alternatives considered,
and tradeoffs.

## D1. Language: Rust

- **Rationale**: memory safety without GC (hardening against hostile documents), deterministic
  single-binary CLI, first-class testing tooling (cargo test, proptest, cargo-fuzz), strong
  ecosystem for the required domains (HTML5 parsing, font parsing), and a natural fit for the
  byte-level format work that is this repo's core.
- **Alternatives**: C++ (unsafe surface too large for a parser-heavy pipeline), Go (GC pauses,
  weaker FFI for font rasterization), TypeScript (fine for runtimes, but the compiler benefits
  from native speed and single-binary distribution).
- **Tradeoffs**: longer compile times; a smaller contributor pool than JS — accepted; the
  compiler is infrastructure, and correctness outweighs breadth of contribution.

## D2. The `.cull` package is the only contract

- **Rationale**: runtimes must never know how compilation occurred. A byte-level format with a
  reference implementation, golden vectors, and two independent reader implementations (JS,
  Rust) is the strongest possible contract enforcement. It mirrors C→ELF→Linux and PNG→viewer.
- **Alternatives**: a JSON/HTML-based interchange (slow, ambiguous, re-introduces DOM-shaped
  semantics), an in-memory object contract (not durable, no versioning).
- **Tradeoffs**: a binary format is less debuggable by hand — mitigated by `cull inspect` and
  golden hex fixtures.

## D3. Versioned binary container with a section table

- **Rationale**: random access for streaming runtimes (seek to chunk sections without decoding
  the whole file), forward compatibility (readers skip unknown section kinds), integrity
  (per-section CRC-32 over decoded payloads), and small size.
- **Alternatives**: single concatenated blob (no random access), zip-like container (extra
  complexity, no content hash tree).
- **Tradeoffs**: fixed 32-byte table entries cost a little space for tiny packages — negligible.

## D4. Determinism is a hard guarantee

- **Rationale**: reproducible builds are the foundation of trustworthy infrastructure. Two
  compilations of the same input with the same toolchain must produce identical bytes. This
  enables golden testing, caching, content addressing, and reproducible archiving.
- **Mechanics**: no wall-clock timestamps; no randomness; stable sorting everywhere; no
  unordered map iteration in any emission path; fixed compression level; NFC-normalized text;
  document ids derived from content digests.
- **Tests**: determinism property tests (compile twice, byte-compare), golden packages.

## D5. Flat (resolved) styles instead of CSS cascade in the package

- **Rationale**: the runtime must not implement a CSS cascade — it would re-introduce browser
  architecture. The compiler folds inheritance and cascade into flat computed styles per
  chunk at compile time. The runtime consumes style *records*.
- **Tradeoffs**: repeated style bytes — mitigated by style interning (`dedupe` transform);
  and a lost ability to re-theme at runtime — out of scope for v1 and recorded in ROADMAP.

## D6. MSDF glyph atlases generated at compile time

- **Rationale**: one resolution-independent asset serves every font size and zoom level; the
  runtime samples a texture instead of running font rasterization; the compiler owns the
  expensive, precision-critical work exactly once.
- **Alternatives**: runtime font rasterization (re-introduces font engine into every runtime;
  non-deterministic across platforms), SDF at single-channel quality (poor corners — the whole
  point of MSDF is the median-of-three reconstruction).
- **Tradeoffs**: atlas generation is complex; it is owned by this repo with dedicated tests
  and rendering validation. This is deliberate: complexity is centralized in the compiler,
  and runtimes stay small.

## D7. Exact quadratic distance + bounded-error cubic conversion

- **Rationale**: true distance to line and quadratic Bézier segments is computable exactly via
  root finding of a low-degree polynomial. Cubics are converted to quadratics with a
  documented, tested Hausdorff error bound, which is the standard practical approach
  (msdfgen does the same). Exactness is verified by comparing MSDF-reconstructed coverage
  against a direct reference rasterizer.
- **Alternatives**: iterative distance solvers (nondeterministic tolerances), flattened
  polygon distance (incorrect at edges).
- **Tradeoffs**: conversion cost at atlas-build time — acceptable; done once per glyph.

## D8. NFC normalization at the boundary

- **Rationale**: NFC normalization makes text byte-deterministic across input encodings and
  makes glyph lookup canonical (one glyph per normalized codepoint).
- **Tradeoffs**: NFC can merge codepoints (e.g., ligature-era canonicals); acceptable for the
  v1 text scope (documented in DESIGN D12 scope table).

## D9. html5ever + pulldown-cmark, nothing larger

- **Rationale**: html5ever is the only fully spec-compliant HTML5 parser in the ecosystem and
  is hardened against hostile input by design. pulldown-cmark is CommonMark-compliant,
  deterministic, and dependency-light. Both are maintained, memory-safe crates.
- **Alternatives**: hand-written parsers (never as correct), regex-based extraction (not
  parsing), browser engines (enormous, re-introduce the DOM pipeline we are replacing).

## D10. Dependency policy

- Runtime dependencies of the compiled pipeline are kept to a small, audited set: html5ever,
  pulldown-cmark, ttf-parser, flate2, sha2, clap. No dependency is added without a written
  justification in DESIGN.md and an entry in SECURITY.md's supply-chain section. Compression
  (flate2) and hashing (sha2) are battle-tested implementations of the exact primitives the
  format specifies; CRC-32 is implemented in-repo with known-answer tests.

## D11. Single responsibility per crate

- The workspace is split by pipeline stage (format / semantic / chunk / atlas / cli) so that
  each crate's invariants can be tested in isolation and its API surface kept minimal.
  No crate may depend on a later stage.

## D12. v1 text scope (explicit boundary, documented, not silently missing)

- Supported: Latin and other scripts whose shaping is per-codepoint + combining marks;
  Unicode word-boundary-aware line breaking (UAX #29); Knuth–Plass-quality line breaking is a
  *runtime* concern (runtimes own layout) — the compiler stores text + styles only.
- Explicitly out of scope for v1 (documented in SECURITY.md and DESIGN.md): complex script
  shaping (Arabic, Indic, Hangul jamo composition), bidi (RTL), hyphenation, vertical text,
  CSS media queries, runtime re-theming. Each is tracked in ROADMAP.md as a future phase
  candidate. Claiming these unsupported is a design decision, not a bug.

## D13. Semantics before presentation

- HTML maps to *meaning* (heading, list item, quote), never to presentational boxes.
  Presentational leftovers that carry no semantics (e.g., `<b>` vs `<strong>`) are
  canonicalized to semantic kinds. This is what makes the Chunk Graph a stable contract
  independent of input format.
