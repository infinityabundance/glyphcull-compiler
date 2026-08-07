# Release receipts — glyphcull-compiler

Every published crate must be traceable to its **source commit, version, package
hash, build/test/conformance commands, dry-run result, and release timestamp**.
This directory is that evidence: committed receipts, the schema, and the scripts
that generate and validate them.

## Layout

```
release/
  README.md                     this document
  templates/
    receipt.template.json       the receipt schema (placeholder values)
  scripts/
    generate-release-receipt.sh   write release/receipts/<package>-<version>.json
    check-release-receipts.sh     validate the committed receipts (--fast CI gate,
                                  --full release procedure)
    release-dry-run.sh            assemble every crate in release order
  receipts/                     the committed receipts (one per published crate)
```

## The receipt contract

```json
{
  "project": "glyphcull",
  "repository": "…origin url…",
  "package": "…crate name…",
  "version": "…manifest version…",
  "git_commit": "…40 hex…",
  "git_tree_clean": true,
  "source_archive_hash": "…sha256…",
  "package_archive_hash": "…sha256…",
  "toolchain": { "rust": "…", "cargo": "…", "node": "…", "npm": "…" },
  "commands": { "build": "…", "test": "…", "conformance": "…", "package_dry_run": "…" },
  "results": { "build": "pass|fail|not-run", "test": "…", "conformance": "…", "package_dry_run": "pass|fail" },
  "release_timestamp": "2026-08-07T00:00:00Z"
}
```

| Field | Meaning | Deterministic |
|---|---|---|
| `project` / `repository` | The project and the origin this receipt was generated in | yes |
| `package` / `version` | The crate and its manifest version at the recorded commit | yes |
| `git_commit` | The exact commit the package was assembled from | varies per release (recorded) |
| `git_tree_clean` | Receipts are only generated on a clean tree (`true`) | yes |
| `source_archive_hash` | SHA-256 over every blob id + path of the recorded commit's tree (`git ls-tree -r`), C-sorted — deterministic given the commit, independent of HEAD or git version | yes (given the commit) |
| `package_archive_hash` | SHA-256 of the real `cargo package` tarball (`target/package/<pkg>-<ver>.crate`) | yes (given the commit; cargo packages are byte-reproducible) |
| `toolchain` | The compiler/tool versions at generation time | recorded (varies by machine) |
| `commands` | The exact commands the receipt's results refer to | yes |
| `results` | `pass` / `fail` when the gate ran, `not-run` otherwise (full gates need `GLYPHCULL_RECEIPT_FULL=1`) | recorded |
| `release_timestamp` | UTC generation time — the only wall-clock field; it never enters `.cull` output | varies (allowed metadata) |

Everything except `release_timestamp`, `toolchain`, `git_commit`, and the two hashes
is schema-fixed; the check script verifies every deterministic field.

## Release order (enforced)

Publish crates in dependency order — each crate's manifest depends only on
earlier crates, so crates.io resolves at publish time:

```text
glyphcull-format → glyphcull-semantic → glyphcull-chunk → glyphcull-atlas
→ glyphcull-pipeline → glyphcull-cli → glyphcull (umbrella)
```

`release-dry-run.sh` iterates this exact order (it fails on the first crate that
does not assemble), and `check-release-receipts.sh` requires every crate in the
order to have a receipt.

## Usage

```sh
# Assemble every crate in release order (the dry run; nothing is published).
release/scripts/release-dry-run.sh

# Generate a receipt for one crate (dirty-tree-refusing).
release/scripts/generate-release-receipt.sh glyphcull-format

# …and re-run the full gates so the receipt records real results:
GLYPHCULL_RECEIPT_FULL=1 release/scripts/generate-release-receipt.sh glyphcull-format

# Validate the committed receipts.
release/scripts/check-release-receipts.sh --fast   # CI gate
release/scripts/check-release-receipts.sh --full   # release procedure (recomputes
                                                   # every package hash from its commit)
```

CI runs `check-release-receipts.sh --fast` on every push/PR: schema, filename,
commit existence, source-tree-hash honesty, clean-tree claim, and completeness.
The `--full` mode is the manual release gate — it re-derives every package
archive hash from the recorded commit, proving reproducibility.

## Workflow

1. Finish the change; commit + push; the tree must be clean.
2. `release/scripts/release-dry-run.sh` — every crate assembles.
3. `GLYPHCULL_RECEIPT_FULL=1 release/scripts/generate-release-receipt.sh <crate>`
   for each crate in release order (the dry run already ran; the full gate is
   the release-time evidence).
4. `release/scripts/check-release-receipts.sh --full` — all receipts valid,
   hashes reproduce.
5. Commit the receipts, push, then `cargo publish -p <crate>` in release order.
6. After the final publish, the receipts + this README are already in the repo —
   a subsequent release generates new receipts at the new commit.
