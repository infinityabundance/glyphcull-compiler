# Contributing — glyphcull-compiler

GlyphCull is built as infrastructure that critical systems may depend on for decades.
Contributions are held to the highest bar: correctness, determinism, documentation,
and tests.

## 1. Getting started

- Toolchain: Rust stable (MSRV pinned in `rust-toolchain.toml`; currently 1.85+), `cargo fmt`,
  `cargo clippy`, `cargo test`, `cargo bench`.
- Read in order: `README.md` → `Architecture.md` → `DESIGN.md` → `docs/format/SPEC.md` →
  `docs/format/GLOSSARY.md` → `TESTING.md` → `PERFORMANCE.md` → `SECURITY.md`.

## 2. Standards

- **Terminology**: use the canonical terminology (GLOSSARY.md). The codebase reads like a
  graphics engine, never like a browser. "Decode paragraph" is forbidden; "materialize
  chunk" is required. Reviewers enforce this.
- **Determinism**: no timestamps, no randomness, no unordered iteration in any pipeline
  path. If a change can affect output bytes, the determinism suite must pass.
- **No placeholders**: no `TODO`, `FIXME`, `todo!()`, `unimplemented!()`, or dead code.
  Incomplete work does not merge.
- **Small reviews**: each change is one logical unit with its tests and documentation in the
  same commit. Docs and code land together.
- **Dependencies**: no new dependency without a DESIGN.md entry (D10) and SECURITY.md
  supply-chain update.
- **No `unsafe`**: unless the SECURITY.md hardening rules are satisfied (written
  justification + safety comment + review).

## 3. Development workflow

1. Branch from `main` (e.g., `phase2/atlas-msdf`).
2. Implement with tests-first where practical: property tests for invariants, unit tests for
   behavior, golden tests for output bytes.
3. Run the full gate locally:
   `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all && cargo bench -- --quick` (perf smoke).
4. Update documentation in the same change.
5. Open a PR; describe the change, the evidence (measurements), and the documentation delta.

### CI

The GitHub Actions workflow (`.github/workflows/ci.yml`) runs the same gate from a clean
checkout on every push to `main` and every pull request:

- `cargo fmt --check`, `cargo clippy --all-targets --all-features`
- `cargo build --workspace --all-targets`
- `cargo test --all --all-targets`
- `cargo doc --all --no-deps` (the `missing_docs = deny` lint keeps every public item
documented — the API-expansion guard)
- `cargo bench --all -- --test` (bench smoke)
- `cargo package --workspace --dry-run` (every crate packages cleanly — the release gate)
- `./scripts/check-golden.sh && ./scripts/check-goldens.sh` (byte-exact golden invariants)
- `./scripts/ci-smoke.sh` (the CLI end to end: Markdown + HTML compile → `cull validate` →
  `cull inspect` → double-compile determinism)

CI is the release gate: a workflow must be green before a crate is published (`cargo
publish`), and `cargo package --dry-run` must pass for the exact tree being published.

## 4. Review requirements

- Behavior changes must include before/after evidence (measurements, diffs of golden bytes).
- Golden fixture regeneration is a deliberate act: the regeneration script refuses to run on
  a dirty tree, and the diff is reviewed byte-by-byte.
- Any change to `SPEC.md` requires the corresponding reader/writer change in the same PR and
  a version policy decision (see SPEC.md §versioning).

## 5. Bug reports

- Include: input that triggers the bug (minimized), expected vs actual behavior, toolchain
  versions, and (if available) `cull validate` output.
- Every accepted bug gets a regression test in the same PR as the fix.

## 6. Security

- Do not open public issues for security vulnerabilities; follow the disclosure path in
  SECURITY.md.

## 7. License

Contributions are licensed under Apache-2.0 (see LICENSE). By contributing you agree to
these terms.
