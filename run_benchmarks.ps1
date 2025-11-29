#!/usr/bin/env pwsh
# Performance comparison script for Lua-RS vs Native Lua

param(
    [switch]$NoColor
)

$benchmarks = @(
    "bench_arithmetic.lua",
    "bench_functions.lua",
    "bench_tables.lua",
    "bench_strings.lua",
    "bench_control_flow.lua"
)

# Detect Native Lua executable
$nativeLua = if ($env:NATIVE_LUA) { $env:NATIVE_LUA } else { "lua" }

# Helper function to write with optional color
function Write-ColorHost {
    param(
        [string]$Message,
        [string]$Color = "White"
    )
    if ($NoColor) {
        Write-Host $Message
    } else {
        Write-Host $Message -ForegroundColor $Color
    }
}

Write-Host ""
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "  Lua-RS vs Native Lua Performance" "Cyan"
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "Native Lua: $nativeLua" "Gray"
Write-Host ""

foreach ($bench in $benchmarks) {
    Write-Host ""
    Write-ColorHost ">>> $bench <<<" "Yellow"
    Write-Host ""
    
    Write-ColorHost "--- Lua-RS ---" "Magenta"
    & ".\target\release\lua.exe" "benchmarks\$bench"
    
    Write-Host ""
    Write-ColorHost "--- Native Lua ---" "Green"
    & $nativeLua "benchmarks\$bench"
    
    Write-Host ""
    Write-Host "----------------------------------------"
}

Write-Host ""
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "  Comparison Complete!" "Cyan"
Write-ColorHost "========================================" "Cyan"
Write-Host ""
Write-ColorHost "See PERFORMANCE_REPORT.md for detailed analysis" "Yellow"
