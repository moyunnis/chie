#!/usr/bin/env bash
# Build chie and drop the binary somewhere on your PATH. Linux / macOS.
set -euo pipefail

cd "$(dirname "$0")"

if ! command -v cargo >/dev/null 2>&1; then
    echo "chie: cargo not found — install Rust from https://rustup.rs first." >&2
    exit 1
fi

echo "Building chie (release)…"
cargo build --release

BIN="target/release/chie"
DEST="${PREFIX:-$HOME/.local}/bin"
mkdir -p "$DEST"
install -m755 "$BIN" "$DEST/chie"

echo "Installed → $DEST/chie"
case ":$PATH:" in
    *":$DEST:"*) : ;;
    *) echo "Note: $DEST is not on your PATH — add it to your shell profile." ;;
esac
echo "Run 'chie --version' to check, then 'chie <file>' and press Ctrl+G."
