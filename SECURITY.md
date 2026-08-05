# Security — glyphcull-compiler

Status: Phase 0 (foundations). Threat model and controls are defined now; concrete audits
land with each phase.

## 1. Position

GlyphCull is not marketed as impossible to scrape; that claim is not technically supportable
on an open client. The extraction-resistant properties of the compiled representation raise
the engineering cost of high-quality, large-scale automated extraction. Security claims in
this repository are technically accurate and evidence-backed.

## 2. Trust model

- **Inputs are untrusted**: source documents (HTML, Markdown), stylesheets, and font files
  may be hostile. This is the primary attack surface of the compiler.
- **Outputs are trusted by downstream systems**: packages may be archived for decades and
  served to runtimes on end-user devices. The compiler must never emit a package that
  crashes, hangs, or degrades a runtime beyond declared resource budgets.
- **The toolchain is trusted**: compilers, dependencies, and the build environment are
  assumed correct (supply-chain controls below reduce, but cannot eliminate, this risk).

## 3. Threat model

| # | Threat | Vector | Control |
|---|---|---|---|
| T1 | Parser DoS (billion-laughs, deep nesting, quadratic blowup) | HTML/Markdown input | html5ever/pulldown-cmark are linear in input; explicit depth and node-count limits in the semantic builder; stress tests |
| T2 | Memory exhaustion | Huge inputs; pathological tables/nesting | Documented input size limits; bounded pipeline memory budget (PERFORMANCE.md §3); counting-allocator memory regression tests |
| T3 | Panic on malformed input | Package bytes fed to reader | Strict bounds-checked reader; no `unwrap` on untrusted paths; truncation/flip corpus; fuzz harness |
| T4 | Nondeterministic output (poisoning reproducible archives) | Anything influencing output order | Determinism suite (byte-identical across runs) |
| T5 | Font parser vulnerabilities | Hostile font files | `ttf-parser` is memory-safe (no `unsafe`); version pinned; dependency audit |
| T6 | Package integrity failure (silent corruption) | Storage/transport corruption | Per-section CRC-32 + SEAL hash tree; validated by `cull validate` and runtimes |
| T7 | Supply-chain compromise | Dependency registry | Small audited dependency set (DESIGN D10); lockfiles committed; `cargo audit` in CI; MSRV pinned |
| T8 | Source disclosure via package metadata | Source-derived content | Compiler emits only what the format defines; no comments, no source path, no timestamps; `source_digest` is a one-way hash, not source |

## 4. Hardening rules (enforced by review, tests, and CI)

1. No `unsafe` in the compiler pipeline except in a single audited module, if ever; each
   occurrence requires a written justification and a safety comment.
2. No panics reachable from untrusted input: `expect`/`unwrap`/indexing are forbidden on
   input-derived paths (clippy lints + review); errors are typed and precise.
3. All numeric arithmetic on untrusted lengths is checked (overflow-safe offset/length
   validation — see SPEC.md §limits).
4. Input size limits are enforced before work begins, and are documented (SPEC.md limits,
   PERFORMANCE.md budgets).
5. Determinism is a security property: T4 is tested by the determinism suite in CI.

## 5. Format security properties

See SPEC.md §security: readers must (a) validate the header, (b) validate every section
entry (bounds, overflow), (c) verify CRC-32 of decoded payloads, (d) enforce decode-size
limits, (e) never panic, (f) skip unknown section kinds (forward compatibility without
interpretation), (g) reject duplicate sections. The SEAL section provides a content hash
tree; signature support is a documented future extension (authenticity of packages at rest).

## 6. Supply chain

- Lockfiles (`Cargo.lock`) committed.
- CI runs `cargo audit`, `cargo deny` (licenses), and a pinned toolchain.
- Dependency additions require a DESIGN.md entry (D10) and this table's update.

## 7. Reporting

Security issues: see CONTRIBUTING.md. Coordinated disclosure; no private key material or
secrets ever in this repository.
