#!/usr/bin/env bash

set -euo pipefail

QUICK=false
LUARS_BIN="./target/release/lua"
NATIVE_LUA="${NATIVE_LUA:-lua}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --quick|-q)
            QUICK=true
            shift
            ;;
        --luars)
            LUARS_BIN="$2"
            shift 2
            ;;
        --native)
            NATIVE_LUA="$2"
            shift 2
            ;;
        *)
            echo "Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

BENCH_NAMES=("fannkuch-redux" "binary-trees" "nbody" "spectral-norm" "mandelbrot" "partial-sums")
BENCH_FILES=("fannkuch_redux.lua" "binary_trees.lua" "nbody.lua" "spectral_norm.lua" "mandelbrot.lua" "partial_sums.lua")
BENCH_DEFAULT_ARGS=("9" "14" "500000" "150" "600" "2000000")
BENCH_QUICK_ARGS=("8" "12" "100000" "100" "300" "500000")

measure_benchmark() {
    local runtime_name="$1"
    local executable="$2"
    local script_path="$3"
    local arg="$4"

    local start_ns end_ns elapsed_ns elapsed_s output
    start_ns=$(date +%s%N)
    output=$("$executable" "$script_path" "$arg")
    end_ns=$(date +%s%N)
    elapsed_ns=$((end_ns - start_ns))
    elapsed_s=$(awk -v ns="$elapsed_ns" 'BEGIN { printf "%.3f", ns / 1000000000 }')

    printf "%s\t%s\t%s\n" "$runtime_name" "$elapsed_s" "$output"
}

get_result_line() {
    printf "%s\n" "$1" | awk 'NF { line = $0 } END { print line }'
}

results_equivalent() {
    local left="$1"
    local right="$2"
    if [[ "$left" == "$right" ]]; then
        return 0
    fi
    if [[ -z "$left" || -z "$right" ]]; then
        return 1
    fi
    [[ "$left" == *"$right"* || "$right" == *"$left"* ]]
}

if [[ ! -f "$LUARS_BIN" ]]; then
    echo "Building Lua-RS release binary..."
    cargo build --release
fi

echo
echo "=============================================="
echo "  Traditional Lua Benchmark Comparison"
echo "=============================================="
echo "Lua-RS: $LUARS_BIN"
echo "Native Lua: $NATIVE_LUA"
if [[ "$QUICK" == true ]]; then
    echo "Mode: quick"
else
    echo "Mode: full"
fi

for idx in "${!BENCH_NAMES[@]}"; do
    name="${BENCH_NAMES[$idx]}"
    file="${BENCH_FILES[$idx]}"
    if [[ "$QUICK" == true ]]; then
        arg="${BENCH_QUICK_ARGS[$idx]}"
    else
        arg="${BENCH_DEFAULT_ARGS[$idx]}"
    fi
    script_path="./lua_benchmarks/$file"

    echo
    echo ">>> $name (arg=$arg) <<<"

    luars_line=$(measure_benchmark "Lua-RS" "$LUARS_BIN" "$script_path" "$arg")
    native_line=$(measure_benchmark "Native Lua" "$NATIVE_LUA" "$script_path" "$arg")

    luars_time=$(echo "$luars_line" | cut -f2)
    native_time=$(echo "$native_line" | cut -f2)
    luars_output=$(echo "$luars_line" | cut -f3-)
    native_output=$(echo "$native_line" | cut -f3-)
    luars_result=$(get_result_line "$luars_output")
    native_result=$(get_result_line "$native_output")

    printf "Lua-RS     %8ss  %s\n" "$luars_time" "$luars_output"
    printf "Native Lua %8ss  %s\n" "$native_time" "$native_output"

    if ! results_equivalent "$luars_result" "$native_result"; then
        echo "Result mismatch detected between runtimes."
    else
        ratio=$(awk -v a="$luars_time" -v b="$native_time" 'BEGIN { if (b == 0) print "0.00"; else printf "%.2f", a / b }')
        printf "Ratio      %8sx Lua-RS / Native\n" "$ratio"
    fi
done

echo
echo "=============================================="
echo "  Benchmark Run Complete"
echo "=============================================="