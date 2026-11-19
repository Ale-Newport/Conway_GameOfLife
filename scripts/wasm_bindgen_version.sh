#!/usr/bin/env bash
# Prints the wasm-bindgen version pinned in Cargo.lock. The wasm-bindgen CLI
# must match it exactly.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$ROOT/Cargo.lock"
