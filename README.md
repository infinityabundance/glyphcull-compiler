# glyphcull-compiler

The GlyphCull compiler. Translates HTML and Markdown into compiled `.cull` document packages.

```
HTML / Markdown
        ↓
Semantic Graph
        ↓
Chunk Graph
        ↓
Transform Pipeline
        ↓
Compression
        ↓
Glyph Atlas Generation (MSDF)
        ↓
Compiled .cull Package
```

This repository owns the **`.cull` package format**: the specification (the contract) and its
reference implementation (`glyphcull-format`), plus the compiler pipeline that emits packages.

## Relationship to other repositories

```
glyphcull-compiler  ──emits──▶  .cull package  ──consumed by──▶  glyphcull-runtime-js
                                                              └─▶  glyphcull-runtime-rs
```

The compiler never depends on runtime implementation. The runtimes never know how compilation
occurred. The package format is the contract. Nothing else.

## The format contract

- Byte-level specification: [`docs/format/SPEC.md`](docs/format/SPEC.md) — canonical, versioned.
- Terminology standard: [`docs/format/GLOSSARY.md`](docs/format/GLOSSARY.md).
- Reference implementation: `crates/glyphcull-format` (container, section codecs, validation).

## Compiler guarantees

- **Determinism**: identical input + identical toolchain ⇒ byte-identical packages. No
  timestamps, no unordered iteration, no randomness.
- **Correctness**: strict, typed, validated models at every stage; invalid input is rejected
  with precise diagnostics, never silently mangled.
- **Separation**: the compiler owns translation (HTML/CSS/Markdown semantics, style
  resolution, layout-independent chunking, glyph atlases). The runtime owns execution
  (visibility, materialization, layout, rendering).

## CLI

`cull` (validate/inspect available; `compile` arrives in Phase 2):

- `cull validate document.cull` — structural + semantic validation; exit 0/1/2
- `cull inspect document.cull` — deterministic package diagnostics
- `cull compile <input.md|input.html> [--style style.css] [--fonts ...] -o document.cull` (Phase 2)

## Repository documents

`Architecture.md` · `DESIGN.md` · `ROADMAP.md` · `TESTING.md` · `PERFORMANCE.md` ·
`SECURITY.md` · `CONTRIBUTING.md` · `CHANGELOG.md`

## License

Apache-2.0. See [`LICENSE`](LICENSE). Bundled assets (fonts) carry their own licenses in
`NOTICE`.
