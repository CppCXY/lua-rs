# Performance comparison script for Lua-RS vs Native Lua

param(
    [switch]$NoColor,
    [switch]$JitStats
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
    "bench_quicksort.lua",
    "bench_strings.lua",
    "bench_string_lib.lua",
    "bench_math.lua",
    "bench_metatables.lua",
    "bench_oop.lua",
    "bench_coroutines.lua",
    "bench_errors.lua"
)

$nativeLua = if ($env:NATIVE_LUA) { $env:NATIVE_LUA } else { "lua" }
$luarsBinary = ".\target\release\lua.exe"

function Write-ColorHost {
    param(
        [string]$Message,
        [string]$Color = "White"
    )

    if ($NoColor) {
        Write-Output $Message
    } else {
        Write-Host $Message -ForegroundColor $Color
    }
}

function Ensure-LuarsBinary {
    param(
        [string]$Executable,
        [switch]$WithJit
    )

    if ($WithJit) {
        Write-ColorHost "Building lua-rs release binary with jit feature..." "Yellow"
        cargo build --release -p luars_interpreter --bin lua --features jit
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to build lua-rs JIT-enabled release binary."
        }
        return
    }

    if (-not (Test-Path $Executable)) {
        Write-ColorHost "Building lua-rs release binary..." "Yellow"
        cargo build --release -p luars_interpreter --bin lua
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to build lua-rs release binary."
        }
    }
}

function Invoke-BenchmarkRuntime {
    param(
        [string]$Executable,
        [string]$ScriptPath,
        [hashtable]$ExtraEnv = @{}
    )

    $previousEnv = @{}
    foreach ($key in $ExtraEnv.Keys) {
        $previousEnv[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $ExtraEnv[$key])
    }

    try {
        $rawOutput = & $Executable $ScriptPath 2>&1
        $output = @($rawOutput | ForEach-Object { [string]$_ })
        $exitCode = $LASTEXITCODE
    } finally {
        foreach ($key in $ExtraEnv.Keys) {
            [Environment]::SetEnvironmentVariable($key, $previousEnv[$key])
        }
    }

    if ($exitCode -ne 0) {
        throw "Benchmark failed for $ScriptPath with exit code $exitCode`n$($output | Out-String)"
    }

    [pscustomobject]@{
        Output = @($output)
        ExitCode = $exitCode
    }
}

function Write-OutputLines {
    param([object[]]$Lines)

    foreach ($line in $Lines) {
        Write-Output ([string]$line)
    }
}

function Convert-StatKey {
    param([string]$Key)

    $parts = ($Key -split '[^A-Za-z0-9]+' | Where-Object { $_ })
    if ($parts.Count -eq 0) {
        return $Key
    }

    return (($parts | ForEach-Object {
        if ($_.Length -eq 0) {
            return $_
        }

        $_.Substring(0, 1).ToUpperInvariant() + $_.Substring(1)
    }) -join '')
}

function Parse-StatValue {
    param([string]$Value)

    $trimmed = $Value.Trim()
    $number = 0
    if ([int]::TryParse($trimmed, [ref]$number)) {
        return $number
    }

    return $trimmed
}

function Get-JitStatsFromOutput {
    param([object[]]$Lines)

    $stats = [ordered]@{}
    $inStats = $false

    foreach ($lineObject in $Lines) {
        $line = [string]$lineObject
        if (-not $inStats) {
            if ($line -eq "JIT Stats:") {
                $inStats = $true
            }
            continue
        }

        if ([string]::IsNullOrWhiteSpace($line)) {
            break
        }

        if ($line -match '^-\s+([^:]+):\s+(.+)$') {
            $normalizedKey = Convert-StatKey $matches[1]
            $stats[$normalizedKey] = Parse-StatValue $matches[2]
            continue
        }

        break
    }

    if ($stats.Count -eq 0) {
        return $null
    }

    [pscustomobject]$stats
}

function Get-TopAbortReason {
    param([psobject]$Stats)

    $abortFields = @(
        @{ Name = "UnsupportedOpcode"; Value = $Stats.AbortUnsupportedOpcode },
        @{ Name = "BackedgeMismatch"; Value = $Stats.AbortBackedgeMismatch },
        @{ Name = "ForwardJump"; Value = $Stats.AbortForwardJump },
        @{ Name = "MissingBranchAfterGuard"; Value = $Stats.AbortMissingBranchAfterGuard },
        @{ Name = "PcOutOfBounds"; Value = $Stats.AbortPcOutOfBounds },
        @{ Name = "EmptyLoopBody"; Value = $Stats.AbortEmptyLoopBody },
        @{ Name = "TraceTooLong"; Value = $Stats.AbortTraceTooLong }
    )

    $top = $abortFields |
        Sort-Object -Property Value -Descending |
        Select-Object -First 1

    if ($null -eq $top -or $top.Value -eq 0) {
        return "none"
    }

    return "$($top.Name)=$($top.Value)"
}

function Get-TopUnsupportedOpcode {
    param([psobject]$Stats)

    if ($null -eq $Stats.TopUnsupportedOpcode -or [string]::IsNullOrWhiteSpace([string]$Stats.TopUnsupportedOpcode)) {
        return "none"
    }

    return [string]$Stats.TopUnsupportedOpcode
}

function Write-JitStatsSummary {
    param(
        [object[]]$Rows,
        [string]$ReportPath
    )

    if ($Rows.Count -eq 0) {
        Write-ColorHost "No JIT stats were captured." "Yellow"
        return
    }

    Write-Output ""
    Write-ColorHost "JIT Summary (Lua-RS)" "Cyan"
    $Rows |
        Sort-Object -Property RecordAborts, BlacklistedSlots -Descending |
        Select-Object Benchmark, RecordedTraces, RecordAborts, BlacklistedSlots, HelperPlanDispatches, HelperPlanCalls, HelperPlanMetamethods, TopAbortReason, TopUnsupportedOpcode |
        Format-Table -AutoSize |
        Out-String -Width 240 |
        Write-Output

    $reportDir = Split-Path -Parent $ReportPath
    if (-not (Test-Path $reportDir)) {
        New-Item -ItemType Directory -Path $reportDir | Out-Null
    }

    $payload = [pscustomobject]@{
        generated_at = (Get-Date).ToString("o")
        rows = $Rows
    }
    $payload | ConvertTo-Json -Depth 5 | Set-Content -Path $ReportPath
    Write-ColorHost "Saved JIT summary to $ReportPath" "Gray"
}

Ensure-LuarsBinary -Executable $luarsBinary -WithJit:$JitStats

Write-Output ""
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "  Lua-RS vs Native Lua Performance" "Cyan"
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "Native Lua: $nativeLua" "Gray"
Write-Output ""

$jitRows = New-Object System.Collections.Generic.List[object]

foreach ($bench in $benchmarks) {
    Write-Output ""
    Write-ColorHost ">>> $bench <<<" "Yellow"
    Write-Output ""

    Write-ColorHost "--- Lua-RS ---" "Magenta"
    $extraEnv = @{}
    if ($JitStats) {
        $extraEnv["LUARS_JIT_STATS"] = "1"
    }
    $luarsResult = Invoke-BenchmarkRuntime -Executable $luarsBinary -ScriptPath "benchmarks\$bench" -ExtraEnv $extraEnv
    Write-OutputLines -Lines $luarsResult.Output

    if ($JitStats) {
        $parsedStats = Get-JitStatsFromOutput -Lines $luarsResult.Output
        if ($null -ne $parsedStats) {
            $jitRows.Add([pscustomobject]@{
                Benchmark = $bench
                TraceHeadersSeen = $parsedStats.TraceHeadersSeen
                RecordAttempts = $parsedStats.RecordAttempts
                RecordedTraces = $parsedStats.RecordedTraces
                RecordAborts = $parsedStats.RecordAborts
                AbortEmptyLoopBody = $parsedStats.AbortEmptyLoopBody
                AbortPcOutOfBounds = $parsedStats.AbortPcOutOfBounds
                AbortUnsupportedOpcode = $parsedStats.AbortUnsupportedOpcode
                TopUnsupportedOpcode = Get-TopUnsupportedOpcode $parsedStats
                AbortMissingBranchAfterGuard = $parsedStats.AbortMissingBranchAfterGuard
                AbortForwardJump = $parsedStats.AbortForwardJump
                AbortBackedgeMismatch = $parsedStats.AbortBackedgeMismatch
                AbortTraceTooLong = $parsedStats.AbortTraceTooLong
                BlacklistHits = $parsedStats.BlacklistHits
                TraceEnterChecks = $parsedStats.TraceEnterChecks
                TraceEnterHits = $parsedStats.TraceEnterHits
                HelperPlanDispatches = $parsedStats.HelperPlanDispatches
                HelperPlanSteps = $parsedStats.HelperPlanSteps
                HelperPlanGuards = $parsedStats.HelperPlanGuards
                HelperPlanCalls = $parsedStats.HelperPlanCalls
                HelperPlanMetamethods = $parsedStats.HelperPlanMetamethods
                TraceSlots = $parsedStats.TraceSlots
                RecordedSlots = $parsedStats.RecordedSlots
                CompiledSlots = $parsedStats.CompiledSlots
                BlacklistedSlots = $parsedStats.BlacklistedSlots
                TopAbortReason = Get-TopAbortReason $parsedStats
            }) | Out-Null
        }
    }

    Write-Output ""
    Write-ColorHost "--- Native Lua ---" "Green"
    $nativeResult = Invoke-BenchmarkRuntime -Executable $nativeLua -ScriptPath "benchmarks\$bench"
    Write-OutputLines -Lines $nativeResult.Output

    Write-Output ""
    Write-Output "----------------------------------------"
}

if ($JitStats) {
    $timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
    Write-JitStatsSummary -Rows $jitRows.ToArray() -ReportPath ".\benchmark_reports\jit-stats-summary-$timestamp.json"
}

Write-Output ""
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "  Comparison Complete!" "Cyan"
Write-ColorHost "========================================" "Cyan"
Write-Output ""