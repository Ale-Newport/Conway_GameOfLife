#!/usr/bin/env bash
# Headless per-scenario benchmarks with `emergence-cli bench` (release build).
# Each line of BENCHES is: <name> <target> <ticks> <warmup> [--param k=v ...].
# Results: target/bench-logs/scenarios/<name>.json plus a summary table in
# target/bench-logs/scenarios/summary.txt. Numbers depend on the machine and
# its load: quote them with the environment.txt written next to them.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

OUT="target/bench-logs/scenarios"
mkdir -p "$OUT"
cargo build --release -q -p simulation-cli
CLI="${CARGO_TARGET_DIR:-target}/release/emergence-cli"

{
    echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "host: $(uname -a)"
    if command -v sysctl > /dev/null 2>&1; then
        echo "cpu: $(sysctl -n machdep.cpu.brand_string 2> /dev/null || true), $(sysctl -n hw.ncpu 2> /dev/null || true) cores"
    fi
    echo "rustc: $(rustc --version)"
    echo "load: $(uptime)"
} > "$OUT/environment.txt"

NOTRACK=(--param track_ages=false --param track_activity=false)
BENCHES=(
    "life-512-dense conway/random-soup-torus 300 30 --param width=512 --param height=512 --param engine=dense ${NOTRACK[*]}"
    "life-512-bit conway/random-soup-torus 300 30 --param width=512 --param height=512 --param engine=bit ${NOTRACK[*]}"
    "life-512-sparse conway/random-soup-torus 300 30 --param topology=infinite --param width=512 --param height=512 ${NOTRACK[*]}"
    "life-2048-dense conway/random-soup-torus 100 10 --param width=2048 --param height=2048 --param engine=dense ${NOTRACK[*]}"
    "life-2048-bit conway/random-soup-torus 100 10 --param width=2048 --param height=2048 --param engine=bit ${NOTRACK[*]}"
    "life-2048-sparse conway/random-soup-torus 100 10 --param topology=infinite --param width=2048 --param height=2048 ${NOTRACK[*]}"
    "life-puffer-sparse conway/infinite-puffer-world 300 30"
    "hashlife-breeder-2pow10 conway/breeder-hashlife 50 5 --param generations_per_tick=1024"
    "hashlife-acorn hashlife/acorn 300 30"
    "metapixel-galaxy metapixel/metapixel.galaxy 4 1"
    "lenia-orbium continuous/lenia.orbium 300 30"
    "lenia-soup continuous/lenia.soup 200 20"
    "smoothlife-soup continuous/smoothlife.soup 200 20"
    "flow-lenia continuous/flow_lenia.demo 100 10"
    "voxel-5766-soup voxel/life-5766-soup 100 10"
    "voxel-clouds voxel/clouds-1 50 5"
    "ecosystem-netlogo ecosystem/wolf_sheep_grass 300 30"
    "ecosystem-meadow ecosystem/balanced_meadow 240 24"
    "ecosystem-savanna ecosystem/african_savanna 240 24"
    "ecosystem-rainforest ecosystem/tropical_rainforest 240 24"
    "evolution-speed evolution/speed_selection 300 30"
    "civilization-trade civilization/trade_routes 300 30"
    "custom-life-producers custom-life/producers-consumers 300 30"
)

for line in "${BENCHES[@]}"; do
    # shellcheck disable=SC2086
    set -- $line
    name="$1"
    target="$2"
    ticks="$3"
    warmup="$4"
    shift 4
    echo "==> $name ($target)"
    "$CLI" bench "$target" --ticks "$ticks" --warmup "$warmup" --json "$OUT/$name.json" "$@" \
        > "$OUT/$name.txt" 2>&1 || echo "    FAILED (see $OUT/$name.txt)" >&2
done
echo "results in $OUT/"
