#!/usr/bin/env bash
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Optional --target <triple> flag for cross-compilation.
TARGET=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) TARGET="$2"; shift 2 ;;
    *) echo "Unknown argument: $1"; exit 1 ;;
  esac
done

VERSION="$(grep -E '^version *= *"' Cargo.toml | head -1 | sed -E 's/^version *= *"([^"]+)".*/\1/')"
if [[ -n "$TARGET" ]]; then
  PLAT="$(echo "$TARGET" | tr -c 'A-Za-z0-9_' '_')"
else
  HOST="$(rustc -vV | sed -n 's/^host: //p')"
  PLAT="$(echo "$HOST" | tr -c 'A-Za-z0-9_' '_')"
fi

if [[ -n "$TARGET" ]]; then
  cargo build --release --target "$TARGET"
else
  cargo build --release
fi

EXE="rtunes"
[[ "$(uname -s)" == MINGW* ]] || [[ "$(uname -s)" == *_NT* ]] && EXE="rtunes.exe"

if [[ -n "$TARGET" ]]; then
  BIN="target/$TARGET/release/$EXE"
else
  BIN="target/release/$EXE"
fi
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
