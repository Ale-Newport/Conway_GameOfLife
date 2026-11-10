#!/usr/bin/env bash
# Run the Criterion benchmarks of every crate that has a benches/ folder and
# save each crate's output to target/bench-logs/<crate>.log.
#
# Usage: scripts/run_benchmarks.sh [extra cargo bench arguments after --]
#   e.g. scripts/run_benchmarks.sh -- --warm-up-time 2 --measurement-time 5
#
# Benchmarks are sensitive to machine load: record the hardware and the load
# with any result you quote (see CONTRIBUTING.md).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LOG_DIR="target/bench-logs"
mkdir -p "$LOG_DIR"

# Arguments after `--` are passed to the bench harness.
extra=()
if (($# > 0)); then
    if [[ "$1" == "--" ]]; then
        shift
    fi
    extra=("$@")
fi

package_name() {
    # Reads `name = "..."` from the [package] section of a Cargo.toml.
    awk '
        /^\[package\]/ { in_pkg = 1; next }
        /^\[/          { in_pkg = 0 }
        in_pkg && /^name[[:space:]]*=/ {
            gsub(/^name[[:space:]]*=[[:space:]]*"|".*$/, "")
            print
            exit
        }
    ' "$1"
}

crates=()
for dir in crates/*/ apps/*/; do
    dir="${dir%/}"
    if [[ -d "$dir/benches" && -f "$dir/Cargo.toml" ]]; then
        crates+=("$(package_name "$dir/Cargo.toml")")
    fi
done

if ((${#crates[@]} == 0)); then
    echo "no crate with a benches/ folder found" >&2
    exit 1
fi

echo "crates with benchmarks: ${crates[*]}"
{
    echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "host: $(uname -a)"
    echo "rustc: $(rustc --version)"
    echo "load: $(uptime)"
} > "$LOG_DIR/environment.txt"

failed=()
for crate in "${crates[@]}"; do
    log="$LOG_DIR/$crate.log"
    echo "==> cargo bench -p $crate (log: $log)"
    if ((${#extra[@]} > 0)); then
        cmd=(cargo bench -p "$crate" -- "${extra[@]}")
    else
        cmd=(cargo bench -p "$crate")
    fi
    if "${cmd[@]}" > "$log" 2>&1; then
        echo "    ok"
    else
        echo "    FAILED (see $log)" >&2
        failed+=("$crate")
    fi
done

if ((${#failed[@]} > 0)); then
    echo "failed: ${failed[*]}" >&2
    exit 1
fi
echo "logs written to $LOG_DIR/"
