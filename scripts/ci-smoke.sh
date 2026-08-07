#!/usr/bin/env bash
# The compiler CI smoke (scripts/ci-smoke.sh): proves the CLI end to end on
# both source surfaces from a clean checkout —
#
#   1. Markdown compiles to a .cull package
#   2. HTML compiles to a .cull package
#   3. both packages validate (cull validate, exit 0)
#   4. both packages inspect deterministically (cull inspect, non-empty)
#   5. compilation is byte-deterministic (double-compile, cmp)
#
# Usage: scripts/ci-smoke.sh   (from the repository root)
# Requires: a stable Rust toolchain; the CLI is built here (cargo build).

set -euo pipefail
cd "$(dirname "$0")/.."

echo "== building the cull CLI"
cargo build -p glyphcull-cli

CULL="$(pwd)/target/debug/cull"
SMOKE="$(mktemp -d)"
trap 'rm -rf "$SMOKE"' EXIT

echo "== compiling Markdown"
"$CULL" compile tests/smoke/sample.md -o "$SMOKE/md.cull"
echo "== compiling HTML"
"$CULL" compile tests/smoke/sample.html -o "$SMOKE/html.cull"

echo "== validating both packages"
"$CULL" validate "$SMOKE/md.cull"
"$CULL" validate "$SMOKE/html.cull"

echo "== inspecting both packages (deterministic diagnostics)"
"$CULL" inspect "$SMOKE/md.cull" > "$SMOKE/md.inspect"
"$CULL" inspect "$SMOKE/html.cull" > "$SMOKE/html.inspect"
test -s "$SMOKE/md.inspect" || { echo "error: empty inspect output (md)"; exit 1; }
test -s "$SMOKE/html.inspect" || { echo "error: empty inspect output (html)"; exit 1; }
"$CULL" inspect "$SMOKE/md.cull" > "$SMOKE/md.inspect2"
cmp "$SMOKE/md.inspect" "$SMOKE/md.inspect2" \
  || { echo "error: inspect output is not deterministic"; exit 1; }

echo "== proving determinism (double-compile)"
"$CULL" compile tests/smoke/sample.md -o "$SMOKE/md2.cull"
"$CULL" compile tests/smoke/sample.html -o "$SMOKE/html2.cull"
cmp "$SMOKE/md.cull" "$SMOKE/md2.cull" || { echo "error: md compile not deterministic"; exit 1; }
cmp "$SMOKE/html.cull" "$SMOKE/html2.cull" || { echo "error: html compile not deterministic"; exit 1; }

echo "ci smoke OK: md + html compile, validate, inspect; output deterministic"
