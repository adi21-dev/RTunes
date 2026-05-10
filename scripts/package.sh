#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(grep -E '^version *= *"' Cargo.toml | head -1 | sed -E 's/^version *= *"([^"]+)".*/\1/')"
HOST="$(rustc -vV | sed -n 's/^host: //p')"
PLAT="$(echo "$HOST" | tr -c 'A-Za-z0-9_' '_')"

cargo build --release

EXE="rtunes"
[[ "$(uname -s)" == MINGW* ]] || [[ "$(uname -s)" == *_NT* ]] && EXE="rtunes.exe"

BIN="target/release/$EXE"
test -f "$BIN" || { echo "missing $BIN"; exit 1; }

DIST_NAME="rtunes-v${VERSION}-${PLAT}"
STAGE="dist/${DIST_NAME}"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp "$BIN" "$STAGE/$EXE"
[[ -f README.md ]] && cp README.md "$STAGE/README.md"

mkdir -p "$STAGE/deps"
cat >"$STAGE/deps/README.txt" <<'EOF'
Place yt-dlp and ffmpeg here (or install to PATH).
See README.md -> deps/ folder.
EOF

mkdir -p dist
ARCHIVE="dist/${DIST_NAME}.tar.gz"
tar -czf "$ARCHIVE" -C dist "$DIST_NAME"
echo "Staged: $STAGE"
echo "Archive: $ARCHIVE"
