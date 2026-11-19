#!/usr/bin/env bash
# Builds the browser demo (apps/web) into apps/web/www/pkg.
#
# Needs the wasm32-unknown-unknown target and the wasm-bindgen CLI at the
# version pinned in Cargo.lock:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --locked --version "$(scripts/wasm_bindgen_version.sh)"
# Then serve the folder, e.g.: python3 -m http.server --directory apps/web/www
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

want="$(scripts/wasm_bindgen_version.sh)"
if ! command -v wasm-bindgen >/dev/null 2>&1; then
    echo "error: wasm-bindgen not found; run: cargo install wasm-bindgen-cli --locked --version $want" >&2
    exit 1
fi
have="$(wasm-bindgen --version | awk '{print $2}')"
if [[ "$have" != "$want" ]]; then
    echo "error: wasm-bindgen $have installed, Cargo.lock needs $want" >&2
    exit 1
fi

TARGET_DIR="$ROOT/target/wasm"
CARGO_TARGET_DIR="$TARGET_DIR" cargo build --locked --release --target wasm32-unknown-unknown -p emergence-web
wasm-bindgen --target web --no-typescript --out-dir apps/web/www/pkg \
    "$TARGET_DIR/wasm32-unknown-unknown/release/emergence_web.wasm"
echo "built apps/web/www/pkg ($(du -h apps/web/www/pkg/emergence_web_bg.wasm | cut -f1) wasm)"
