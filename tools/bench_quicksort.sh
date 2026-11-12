#!/usr/bin/env bash
# Compares two `bork` binaries (e.g. the pre-MIR baseline and the current build) on the quicksort programs:
# median wall time of `bork build` and of the program it produces, plus a check that both print the same output.
#
#   tools/bench_quicksort.sh BASE_BORK HEAD_BORK [runs]
#
# RFC 0006 budget: build time <= 1.5x the baseline, run time not slower.
set -euo pipefail
export LC_ALL=C

base=${1:?usage: $0 BASE_BORK HEAD_BORK [runs]}
head=${2:?usage: $0 BASE_BORK HEAD_BORK [runs]}
runs=${3:-7}
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Median of the wall time (seconds) of `"$@"` over $runs runs; stdout/stderr discarded.
median() {
    local times=() i start end
    for ((i = 0; i < runs; i++)); do
        start=$(date +%s.%N)
        "$@" >/dev/null 2>&1 || true
        end=$(date +%s.%N)
        times+=("$(echo "$end - $start" | bc -l)")
    done
    printf '%s\n' "${times[@]}" | sort -g | sed -n "$((runs / 2 + 1))p"
}

printf '%-26s %10s %10s %7s   %10s %10s %7s  %s\n' program base_build head_build ratio base_run head_run ratio output
for src in "$root"/tools/bench/sort_loop.bork "$root"/quicksort_simple.bork "$root"/quicksort.bork "$root"/quicksort_ref.bork "$root"/quicksort_rec.bork; do
    [ -f "$src" ] || continue
    name=$(basename "$src" .bork)
    "$base" build -o "$work/$name.base" "$src" 2>/dev/null || { echo "$name: baseline does not build"; continue; }
    "$head" build -o "$work/$name.head" "$src" 2>/dev/null || { echo "$name: head does not build"; continue; }
    same=same
    cmp -s <("$work/$name.base" 2>&1; echo "exit $?") <("$work/$name.head" 2>&1; echo "exit $?") || same=DIFFERENT
    bb=$(median "$base" build -o "$work/$name.b" "$src")
    hb=$(median "$head" build -o "$work/$name.h" "$src")
    br=$(median "$work/$name.base")
    hr=$(median "$work/$name.head")
    printf '%-26s %10.3f %10.3f %7.2f   %10.4f %10.4f %7.2f  %s\n' "$name" "$bb" "$hb" "$(echo "$hb / $bb" | bc -l)" "$br" "$hr" "$(echo "$hr / ($br + 0.00001)" | bc -l)" "$same"
done
