#!/usr/bin/env pwsh

param(
    [switch]$NoColor,
    [switch]$Quick,
    [string]$LuaRs = ".\\target\\release\\lua.exe",
    [string]$NativeLua = $(if ($env:NATIVE_LUA) { $env:NATIVE_LUA } else { "lua" })
)

$benchmarks = @(
    @{ Name = "fannkuch-redux"; File = "fannkuch_redux.lua"; DefaultArg = "9"; QuickArg = "8" },
    @{ Name = "binary-trees"; File = "binary_trees.lua"; DefaultArg = "14"; QuickArg = "12" },
    @{ Name = "nbody"; File = "nbody.lua"; DefaultArg = "500000"; QuickArg = "100000" },
    @{ Name = "spectral-norm"; File = "spectral_norm.lua"; DefaultArg = "150"; QuickArg = "100" },
    @{ Name = "mandelbrot"; File = "mandelbrot.lua"; DefaultArg = "600"; QuickArg = "300" },
    @{ Name = "partial-sums"; File = "partial_sums.lua"; DefaultArg = "2000000"; QuickArg = "500000" }
)

function Write-ColorHost {
    param([string]$Message, [string]$Color = "White")
    if ($NoColor) {
        Write-Output $Message
    } else {
        Write-Host $Message -ForegroundColor $Color
    }
}

function Measure-Benchmark {
    param(
        [string]$RuntimeName,
        [string]$Executable,
        [string]$ScriptPath,
        [string]$Argument
    )

    $output = $null
    $elapsed = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        $output = & $Executable $ScriptPath $Argument 2>&1
        $exitCode = $LASTEXITCODE
    } finally {
        $elapsed.Stop()
    }

    if ($exitCode -ne 0) {
        throw "$RuntimeName failed for $ScriptPath with exit code $exitCode`n$output"
    }

    return @{
        Runtime = $RuntimeName
        Seconds = $elapsed.Elapsed.TotalSeconds
        Output = ($output | Out-String).Trim()
    }
}

function Get-ResultLine {
    param([string]$Output)

    $lines = @($Output -split "`r?`n") | Where-Object {
        $_ -is [string] -and $_.Trim().Length -gt 0
    }
    if ($lines.Count -eq 0) {
        return ""
    }
    return ([string]$lines[$lines.Count - 1]).Trim()
}

function Test-EquivalentResult {
    param(
        [string]$Left,
        [string]$Right
    )

    if ($Left -eq $Right) {
        return $true
    }
    if ([string]::IsNullOrEmpty($Left) -or [string]::IsNullOrEmpty($Right)) {
        return $false
    }
    return $Left.Contains($Right) -or $Right.Contains($Left)
}

if (-not (Test-Path $LuaRs)) {
    Write-ColorHost "Building Lua-RS release binary..." "Yellow"
    cargo build --release
}

Write-Output ""
Write-ColorHost "==============================================" "Cyan"
Write-ColorHost "  Traditional Lua Benchmark Comparison" "Cyan"
Write-ColorHost "==============================================" "Cyan"
Write-ColorHost "Lua-RS: $LuaRs" "Gray"
Write-ColorHost "Native Lua: $NativeLua" "Gray"
Write-ColorHost ("Mode: " + ($(if ($Quick) { "quick" } else { "full" }))) "Gray"

foreach ($bench in $benchmarks) {
    $arg = if ($Quick) { $bench.QuickArg } else { $bench.DefaultArg }
    $scriptPath = Join-Path ".\lua_benchmarks" $bench.File

    Write-Output ""
    Write-ColorHost ">>> $($bench.Name) (arg=$arg) <<<" "Yellow"

    $luarsResult = Measure-Benchmark -RuntimeName "Lua-RS" -Executable $LuaRs -ScriptPath $scriptPath -Argument $arg
    Write-ColorHost ("Lua-RS     {0,8:N3}s  {1}" -f $luarsResult.Seconds, $luarsResult.Output) "Magenta"

    $nativeResult = Measure-Benchmark -RuntimeName "Native Lua" -Executable $NativeLua -ScriptPath $scriptPath -Argument $arg
    Write-ColorHost ("Native Lua {0,8:N3}s  {1}" -f $nativeResult.Seconds, $nativeResult.Output) "Green"

    $luarsLine = Get-ResultLine $luarsResult.Output
    $nativeLine = Get-ResultLine $nativeResult.Output

    if (-not (Test-EquivalentResult $luarsLine $nativeLine)) {
        Write-ColorHost "Result mismatch detected between runtimes." "Red"
    } else {
        $ratio = if ($nativeResult.Seconds -gt 0) { $luarsResult.Seconds / $nativeResult.Seconds } else { 0 }
        Write-ColorHost ("Ratio      {0,8:N2}x Lua-RS / Native" -f $ratio) "Cyan"
    }
}

Write-Output ""
Write-ColorHost "==============================================" "Cyan"
Write-ColorHost "  Benchmark Run Complete" "Cyan"
Write-ColorHost "==============================================" "Cyan"