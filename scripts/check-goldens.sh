#!/usr/bin/env sh
# Verifies that the committed compiler-pipeline golden fixtures match a fresh
# regeneration. CI gate: any diff fails the build.
set -eu

if [ -n "$(git status --porcelain)" ]; then
    echo "error: working tree is dirty; run check-goldens from a clean tree" >&2
    exit 1
fi

cargo test -p glyphcull-pipeline --test golden regenerate_fixture -- --ignored

if [ -n "$(git status --porcelain)" ]; then
    echo "error: golden fixtures are out of date; run scripts/regenerate-goldens.sh and review the diff" >&2
    git status --porcelain
    exit 1
fi

echo "golden fixtures are byte-exact"
