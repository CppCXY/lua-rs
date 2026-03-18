#!/usr/bin/env pwsh
# Performance comparison script for Lua-RS vs Native Lua

param(
    [switch]$NoColor
)

$benchmarks = @(
    "bench_arithmetic.lua",
    "bench_control_flow.lua",
    "bench_locals.lua",
    
    "bench_functions.lua", 
    "bench_closures.lua",
    "bench_multiret.lua",

    "bench_tables.lua",
    "bench_table_lib.lua",
    "bench_iterators.lua",
    

    "bench_strings.lua",
    "bench_string_lib.lua",
    

    "bench_math.lua",

    "bench_metatables.lua",
    "bench_oop.lua",
    "bench_coroutines.lua",
    "bench_errors.lua"
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
        Write-Output $Message
    } else {
        Write-Output $Message
        Write-Host $Message -ForegroundColor $Color
    }
}

function Invoke-BenchmarkRuntime {
    param(
        [string]$Executable,
        [string]$ScriptPath
    )

    $output = & $Executable $ScriptPath 2>&1
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        throw "Benchmark failed for $ScriptPath with exit code $exitCode`n$($output | Out-String)"
    }

    foreach ($line in @($output)) {
        Write-Output $line
    }
}

Write-Output ""
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "  Lua-RS vs Native Lua Performance" "Cyan"
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "Native Lua: $nativeLua" "Gray"
Write-Output ""

foreach ($bench in $benchmarks) {
    Write-Output ""
    Write-ColorHost ">>> $bench <<<" "Yellow"
    Write-Output ""
    
    Write-ColorHost "--- Lua-RS ---" "Magenta"
    Invoke-BenchmarkRuntime -Executable ".\target\release\lua.exe" -ScriptPath "benchmarks\$bench"
    
    Write-Output ""
    Write-ColorHost "--- Native Lua ---" "Green"
    Invoke-BenchmarkRuntime -Executable $nativeLua -ScriptPath "benchmarks\$bench"
    
    Write-Output ""
    Write-Output "----------------------------------------"
}

Write-Output ""
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "  Comparison Complete!" "Cyan"
Write-ColorHost "========================================" "Cyan"
Write-Output ""
Write-ColorHost "See PERFORMANCE_REPORT.md for detailed analysis" "Yellow"
