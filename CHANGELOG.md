# Changelog

All notable changes to this repository are recorded here, in reverse chronological order.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

### Planned (Phase 2 — Compiler pipeline)

- Semantic Graph, HTML/Markdown front ends, Chunk Graph and transforms, MSDF glyph atlas
  generation, compression, `cull compile` CLI, full test pyramid and documentation.
