#!/usr/bin/env bash
# Lua-Benchmarks runner for Linux/macOS.
# By default compares native `lua` with this repository's release binary.
set -u

cd "$(dirname "$0")"

NRUNS="${NRUNS:-1}"
OUTPUT="${OUTPUT:-lua_bench_results}"
NATIVE_LUA="${NATIVE_LUA:-lua}"
LUARS_BIN="${LUARS_BIN:-../target/release/lua}"

if [ ! -x "$LUARS_BIN" ]; then
    echo "luars binary not found or not executable: $LUARS_BIN" >&2
    echo "Build it first with: cargo build --release" >&2
    exit 1
fi

BINARIES=("$NATIVE_LUA" "$LUARS_BIN")
BIN_NAMES=("native-lua" "luars")

TESTS=(
    "brainfuck|brainfuck.lua 10"
    "mem-access|mem-access.lua 150"
    "oop-dots|oop-dots.lua 100"
    "c-call|c-call.lua 550"
    "ray|ray.lua 768"
    "coro|coro-scheduler.lua 450"
    "json|json-serializer.lua 55"
    "heapsort|heapsort.lua 10 150000"
    "mandelbrot|mandel.lua"
    "juliaset|qt.lua"
    "queen|queen.lua 12"
    "binary|binary-trees.lua 14"
    "n-body|n-body.lua 800000"
    "fannkuch|fannkuch-redux.lua 10"
    "fasta|fasta.lua 2500000"
    "k-nucleotide|k-nucleotide.lua < fasta900000.txt"
    "regex-dna|regex-dna.lua < fasta900000.txt"
    "spectral-norm|spectral-norm.lua 1000"
)

FASTA_INPUT="fasta900000.txt"

if [ ! -f "$FASTA_INPUT" ]; then
    echo "Generating input file ($FASTA_INPUT) ..."
    "$NATIVE_LUA" fasta.lua 900000 > "$FASTA_INPUT"
fi

measure() {
    local cmd="$1"
    local start end
    start=$(date +%s%N)
    eval "$cmd" > /dev/null 2>&1
    local status=$?
    end=$(date +%s%N)
    if [ $status -ne 0 ]; then
        echo "FAILED" >&2
        return 1
    fi
    awk -v s="$start" -v e="$end" 'BEGIN { printf "%.6f", (e-s)/1000000000 }'
}

run_one() {
    local bin="$1"
    local testcmd="$2"
    local min=999999
    for ((i=0; i<NRUNS; i++)); do
        local t
        t=$(measure "$bin $testcmd")
        if [ $? -eq 0 ]; then
            min=$(awk -v a="$min" -v b="$t" 'BEGIN { if (b < a) print b; else print a }')
        fi
    done
    echo "$min"
}

echo "Binaries: ${BINARIES[*]}"
echo "Nruns:    $NRUNS"
echo ""

RESULTS=()
for testline in "${TESTS[@]}"; do
    name="${testline%%|*}"
    cmd="${testline#*|}"
    printf '%-16s ' "$name"
    row=()
    for ((b=0; b<${#BINARIES[@]}; b++)); do
        t=$(run_one "${BINARIES[$b]}" "$cmd")
        row+=("$t")
        printf '%-12s ' "$t"
    done
    echo
    RESULTS+=("$(IFS=' '; echo "${row[*]}")")
done

rm -f "$FASTA_INPUT"

# Save raw data
{
    printf 'test\t'
    for n in "${BIN_NAMES[@]}"; do printf '%s\t' "$n"; done
    printf '\n'
    idx=0
    for testline in "${TESTS[@]}"; do
        name="${testline%%|*}"
        printf '%s\t' "$name"
        row=(${RESULTS[$idx]})
        for ((j=0; j<${#BINARIES[@]}; j++)); do
            printf '%.4f\t' "${row[$j]}"
        done
        printf '\n'
        idx=$((idx+1))
    done
} > "$OUTPUT.dat"

# Save normalized and speed files
{
    printf 'test\t'
    for n in "${BIN_NAMES[@]}"; do printf '%s\t' "$n"; done
    printf '\n'
    idx=0
    for testline in "${TESTS[@]}"; do
        name="${testline%%|*}"
        printf '%s\t' "$name"
        row=(${RESULTS[$idx]})
        base="${row[0]}"
        for ((j=0; j<${#BINARIES[@]}; j++)); do
            v="${row[$j]}"
            if awk -v a="$base" -v b="$v" 'BEGIN { exit !(b != 0) }'; then
                printf '%.4f\t' "$v"
            else
                printf 'NaN\t'
            fi
        done
        printf '\n'
        idx=$((idx+1))
    done
} > "$OUTPUT-norm.dat"

{
    printf 'test\t'
    for n in "${BIN_NAMES[@]}"; do printf '%s\t' "$n"; done
    printf '\n'
    idx=0
    for testline in "${TESTS[@]}"; do
        name="${testline%%|*}"
        printf '%s\t' "$name"
        row=(${RESULTS[$idx]})
        base="${row[0]}"
        for ((j=0; j<${#BINARIES[@]}; j++)); do
            v="${row[$j]}"
            if awk -v a="$base" -v b="$v" 'BEGIN { exit !(a != 0 && b != 0) }'; then
                printf '%.4f\t' "$(awk -v a="$base" -v b="$v" 'BEGIN { print a/b }')"
            else
                printf 'NaN\t'
            fi
        done
        printf '\n'
        idx=$((idx+1))
    done
} > "$OUTPUT-speed.dat"

echo ""
echo "Saved: $OUTPUT.dat, $OUTPUT-norm.dat, $OUTPUT-speed.dat"
