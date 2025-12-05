#!/usr/bin/env bash
# Performance comparison script for Lua-RS vs Native Lua (Linux/macOS)

set -e

BENCHMARKS=(
    "bench_arithmetic.lua"
    "bench_control_flow.lua"
    "bench_locals.lua"
    
    "bench_functions.lua" 
    "bench_closures.lua"
    "bench_multiret.lua"

    "bench_tables.lua"
    "bench_table_lib.lua"
    "bench_iterators.lua"
    

    "bench_strings.lua"
    "bench_string_lib.lua"
    

    "bench_math.lua"

    "bench_metatables.lua"
    "bench_oop.lua"
    "bench_coroutines.lua"
    "bench_errors.lua"
)

# Parse arguments
NOCOLOR=false
for arg in "$@"; do
    case $arg in
        --nocolor|-n)
            NOCOLOR=true
            shift
            ;;
    esac
done

# Colors (disabled with --nocolor)
if [ "$NOCOLOR" = true ]; then
    CYAN=''
    YELLOW=''
    MAGENTA=''
    GREEN=''
    NC=''
else
    CYAN='\033[0;36m'
    YELLOW='\033[1;33m'
    MAGENTA='\033[0;35m'
    GREEN='\033[0;32m'
    NC='\033[0m'
fi

echo ""
echo -e "${CYAN}========================================${NC}"
echo -e "${CYAN}  Lua-RS vs Native Lua Performance${NC}"
echo -e "${CYAN}========================================${NC}"
echo ""

# Detect lua-rs binary location
LUARS_BIN="./target/release/lua"
if [ ! -f "$LUARS_BIN" ]; then
    echo "Building lua-rs in release mode..."
    cargo build --release
fi

# Detect native Lua
NATIVE_LUA=""
if command -v lua5.4 &> /dev/null; then
    NATIVE_LUA="lua5.4"
elif command -v lua &> /dev/null; then
    NATIVE_LUA="lua"
else
    echo "Warning: Native Lua not found. Only running lua-rs benchmarks."
fi

for bench in "${BENCHMARKS[@]}"; do
    echo ""
    echo -e "${YELLOW}>>> $bench <<<${NC}"
    echo ""
    
    echo -e "${MAGENTA}--- Lua-RS ---${NC}"
    "$LUARS_BIN" "benchmarks/$bench"
    
    if [ -n "$NATIVE_LUA" ]; then
        echo ""
        echo -e "${GREEN}--- Native Lua ---${NC}"
        "$NATIVE_LUA" "benchmarks/$bench"
    fi
    
    echo ""
    echo "----------------------------------------"
done

echo ""
echo -e "${CYAN}========================================${NC}"
echo -e "${CYAN}  Comparison Complete!${NC}"
echo -e "${CYAN}========================================${NC}"
echo ""
echo -e "${YELLOW}See PERFORMANCE_REPORT.md for detailed analysis${NC}"
