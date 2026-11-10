#!/usr/bin/env bash
# Capture a smoke screenshot of the dashboard and of every laboratory.
#
# Builds the desktop application in release mode, then runs
#   emergence-lab --mode <slug> --smoke-screenshot docs/screenshots/<slug>.png --smoke-frames 180
# for each slug, each under `timeout 180`. The slugs are the ones of
# `AppMode::slug` in crates/interface/src/mode.rs.
#
# Usage: scripts/capture_screenshots.sh          (from anywhere in the checkout)
# Needs a display (the app opens a window) and `timeout` (GNU coreutils; on
# macOS install coreutils, which provides `timeout` or `gtimeout`).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if command -v timeout >/dev/null 2>&1; then
    TIMEOUT=timeout
elif command -v gtimeout >/dev/null 2>&1; then
    TIMEOUT=gtimeout
else
    echo "error: 'timeout' (or 'gtimeout') not found; install GNU coreutils" >&2
    exit 1
fi

SLUGS=(
    dashboard
    classic-conway
    ecosystem
    custom-life
    conway-within-conway
    cellular-automata
    evolution
    continuous-life
    3d-life
    civilizations
)

OUT_DIR="docs/screenshots"
FRAMES=180

echo "==> building emergence-lab (release)"
cargo build --release -p emergence-lab
BIN="$ROOT/target/release/emergence-lab"

mkdir -p "$OUT_DIR"
failed=()
for slug in "${SLUGS[@]}"; do
    out="$OUT_DIR/$slug.png"
    echo "==> $slug -> $out"
    if "$TIMEOUT" 180 "$BIN" --mode "$slug" --smoke-screenshot "$out" --smoke-frames "$FRAMES"; then
        echo "    ok"
    else
        status=$?
        echo "    FAILED (exit status $status)" >&2
        failed+=("$slug")
    fi
done

if ((${#failed[@]} > 0)); then
    echo "failed: ${failed[*]}" >&2
    exit 1
fi
echo "all screenshots written to $OUT_DIR/"
