#!/usr/bin/env sh
# Regenerates committed golden fixtures (tests/fixtures/*.cull).
#
# Deliberate action: refuses to run on a dirty tree so that golden drift is always
# reviewed as a diff. Run from the repository root.
set -eu

if [ -n "$(git status --porcelain)" ]; then
    echo "error: working tree is dirty; commit or stash before regenerating goldens" >&2
    exit 1
fi

cargo test -p glyphcull-format --test golden_minimal regenerate_fixture -- --ignored

echo "golden fixtures regenerated; review the diff before committing"
