#!/usr/bin/env pwsh
# Performance comparison script for Lua-RS vs Native Lua

$benchmarks = @(
    "bench_arithmetic.lua",
    "bench_functions.lua",
    "bench_tables.lua",
    "bench_strings.lua",
    "bench_control_flow.lua"
)

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Lua-RS vs Native Lua Performance" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

foreach ($bench in $benchmarks) {
    Write-Host ""
    Write-Host ">>> $bench <<<" -ForegroundColor Yellow
    Write-Host ""
    
    Write-Host "--- Lua-RS ---" -ForegroundColor Magenta
    & ".\target\release\lua.exe" "benchmarks\$bench"
    
    Write-Host ""
    Write-Host "--- Native Lua ---" -ForegroundColor Green
    & lua "benchmarks\$bench"
    
    Write-Host ""
    Write-Host "----------------------------------------"
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Comparison Complete!" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "See PERFORMANCE_REPORT.md for detailed analysis" -ForegroundColor Yellow
