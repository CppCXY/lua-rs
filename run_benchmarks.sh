#!/usr/bin/env bash
# Performance comparison script for Lua-RS vs Native Lua (Linux/macOS)

set -e

BENCHMARKS=(
    "bench_arithmetic.lua"
    "bench_functions.lua"
    "bench_tables.lua"
    "bench_strings.lua"
    "bench_control_flow.lua"
)

# Colors
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
MAGENTA='\033[0;35m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

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
