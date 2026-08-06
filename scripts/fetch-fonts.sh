#!/usr/bin/env sh
# Fetches the bundled Noto Sans font faces (assets/fonts/) from the official
# Noto Project release, then verifies the checksums.
#
# Determinism: the release tag is pinned; the fetched files are verified against
# scripts/font-checksums.sha256 byte-for-byte.
set -eu

TAG="NotoSans-v2.015"
URL="https://github.com/notofonts/latin-greek-cyrillic/releases/download/${TAG}/${TAG}.zip"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl -sL --max-time 300 -o "$TMP/noto.zip" "$URL"
cd "$TMP"
unzip -q -o noto.zip \
    "NotoSans/unhinted/ttf/NotoSans-Regular.ttf" \
    "NotoSans/unhinted/ttf/NotoSans-Bold.ttf" \
    "NotoSans/unhinted/ttf/NotoSans-Italic.ttf" \
    "NotoSans/unhinted/ttf/NotoSans-BoldItalic.ttf"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
for dest in "$ROOT/assets/fonts" "$ROOT/crates/glyphcull-atlas/assets/fonts" "$ROOT/crates/glyphcull-pipeline/assets/fonts"; do
    mkdir -p "$dest"
    for face in Regular Bold Italic BoldItalic; do
        cp "NotoSans/unhinted/ttf/NotoSans-${face}.ttf" "$dest/"
    done
    cp "OFL.txt" "$dest/" 2>/dev/null || true
done
DEST="$ROOT/assets/fonts"

cd "$DEST"
sha256sum -c "$(cd "$(dirname "$0")/.." && pwd)/scripts/font-checksums.sha256"
echo "fonts verified"
