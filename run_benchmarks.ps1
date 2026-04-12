# Performance comparison script for Lua-RS vs Native Lua

param(
    [switch]$NoColor,
    [switch]$JitStats,
    [string]$CompareBaseReport,
    [string]$CompareTargetReport,
    [switch]$CompareLatest
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
        $stdoutPath = [System.IO.Path]::GetTempFileName()
        $stderrPath = [System.IO.Path]::GetTempFileName()

        try {
            $process = Start-Process `
                -FilePath $Executable `
                -ArgumentList @($ScriptPath) `
                -NoNewWindow `
                -Wait `
                -PassThru `
                -RedirectStandardOutput $stdoutPath `
                -RedirectStandardError $stderrPath

            $stdoutLines = if (Test-Path $stdoutPath) {
                @(Get-Content -Path $stdoutPath -ErrorAction SilentlyContinue)
            } else {
                @()
            }
            $stderrLines = if (Test-Path $stderrPath) {
                @(Get-Content -Path $stderrPath -ErrorAction SilentlyContinue)
            } else {
                @()
            }
            $output = @($stdoutLines + $stderrLines | ForEach-Object { [string]$_ })
            $exitCode = $process.ExitCode
        } finally {
            Remove-Item -Path $stdoutPath, $stderrPath -ErrorAction SilentlyContinue
        }
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
        Select-Object Benchmark, RecordedTraces, RootNativeDispatches, RootInterpreterDispatches, SideNativeDispatches, SideInterpreterDispatches, NativeExitIndexResolveAttempts, NativeExitIndexResolveHits, NativeProfileGuardSteps, NativeProfileArithmeticHelpers, NativeProfileTableHelpers, NativeProfileUpvalueHelpers, NativeProfileShiftHelpers, HelperPlanDispatches, HelperPlanCalls, HelperPlanMetamethods, TopAbortReason, TopUnsupportedOpcode |
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

function Get-BenchmarkRuntimeSamples {
    param([object[]]$Lines)

    $samples = New-Object System.Collections.Generic.List[object]
    $pattern = '(?<seconds>\d+(?:\.\d+)?) seconds(?: \((?<throughput>\d+(?:\.\d+)?) (?<unit>[^)]+)\))?'

    foreach ($lineObject in $Lines) {
        $line = [string]$lineObject
        if ($line -notmatch $pattern) {
            continue
        }

        $sample = [ordered]@{
            Line = $line
            Seconds = [double]$matches.seconds
        }

        if ($matches.throughput) {
            $sample.Throughput = [double]$matches.throughput
        }

        if ($matches.unit) {
            $sample.ThroughputUnit = $matches.unit.Trim()
        }

        $samples.Add([pscustomobject]$sample) | Out-Null
    }

    return $samples.ToArray()
}

function Get-TotalRuntimeSeconds {
    param([object[]]$Samples)

    if ($null -eq $Samples -or $Samples.Count -eq 0) {
        return $null
    }

    $total = 0.0
    foreach ($sample in $Samples) {
        $total += [double]$sample.Seconds
    }

    return [math]::Round($total, 6)
}

function Load-JitSummaryReport {
    param([string]$Path)

    if (-not (Test-Path $Path)) {
        throw "Report not found: $Path"
    }

    return Get-Content -Path $Path -Raw | ConvertFrom-Json
}

function Get-LatestJitSummaryPaths {
    param([string]$Directory)

    if (-not (Test-Path $Directory)) {
        throw "Benchmark report directory not found: $Directory"
    }

    $reports = Get-ChildItem -Path $Directory -Filter 'jit-stats-summary-*.json' |
        Sort-Object -Property LastWriteTimeUtc -Descending

    if ($reports.Count -lt 2) {
        throw "Need at least two JIT summary reports in $Directory to compare latest results."
    }

    return @($reports[1].FullName, $reports[0].FullName)
}

function Get-RowValue {
    param(
        [psobject]$Row,
        [string]$PropertyName
    )

    if ($null -eq $Row) {
        return $null
    }

    $property = $Row.PSObject.Properties[$PropertyName]
    if ($null -eq $property) {
        return $null
    }

    return $property.Value
}

function Get-NumericRowValue {
    param(
        [psobject]$Row,
        [string]$PropertyName,
        [double]$DefaultValue = 0.0
    )

    $value = Get-RowValue -Row $Row -PropertyName $PropertyName
    if ($null -eq $value) {
        return $DefaultValue
    }

    return [double]$value
}

function Get-OptionalDelta {
    param(
        [object]$BaseValue,
        [object]$TargetValue
    )

    if ($null -eq $BaseValue -or $null -eq $TargetValue) {
        return $null
    }

    return [math]::Round(([double]$TargetValue - [double]$BaseValue), 6)
}

function Format-OptionalNumber {
    param(
        [object]$Value,
        [int]$Decimals = 3
    )

    if ($null -eq $Value) {
        return '-'
    }

    return ('{0:N' + $Decimals + '}') -f [double]$Value
}

function Format-DeltaValue {
    param([object]$Value)

    if ($null -eq $Value) {
        return '-'
    }

    if ($Value -is [double] -or $Value -is [single] -or $Value -is [decimal]) {
        return ('{0:+0.000;-0.000;0.000}' -f [double]$Value)
    }

    return ('{0:+0;-0;0}' -f [double]$Value)
}

function Format-PercentDelta {
    param(
        [object]$BaseValue,
        [object]$TargetValue,
        [switch]$InvertSign
    )

    if ($null -eq $BaseValue -or $null -eq $TargetValue) {
        return '-'
    }

    $base = [double]$BaseValue
    if ($base -eq 0) {
        return '-'
    }

    $percent = (([double]$TargetValue - $base) / $base) * 100.0
    if ($InvertSign) {
        $percent *= -1.0
    }

    return ('{0:+0.0;-0.0;0.0}%' -f $percent)
}

function Write-JitStatsComparison {
    param(
        [string]$BaseReportPath,
        [string]$TargetReportPath
    )

    $baseReport = Load-JitSummaryReport -Path $BaseReportPath
    $targetReport = Load-JitSummaryReport -Path $TargetReportPath

    $baseRows = @{}
    foreach ($row in @($baseReport.rows)) {
        $baseRows[$row.Benchmark] = $row
    }

    $targetRows = @{}
    foreach ($row in @($targetReport.rows)) {
        $targetRows[$row.Benchmark] = $row
    }

    $benchmarks = @($baseRows.Keys + $targetRows.Keys | Sort-Object -Unique)
    $runtimeRows = New-Object System.Collections.Generic.List[object]
    $jitRows = New-Object System.Collections.Generic.List[object]

    foreach ($benchmark in $benchmarks) {
        $baseRow = $baseRows[$benchmark]
        $targetRow = $targetRows[$benchmark]

        $baseLuaRsSeconds = Get-RowValue -Row $baseRow -PropertyName 'LuaRsTotalSeconds'
        $targetLuaRsSeconds = Get-RowValue -Row $targetRow -PropertyName 'LuaRsTotalSeconds'
        $baseNativeSeconds = Get-RowValue -Row $baseRow -PropertyName 'NativeTotalSeconds'
        $targetNativeSeconds = Get-RowValue -Row $targetRow -PropertyName 'NativeTotalSeconds'

        if ($null -ne $baseLuaRsSeconds -or $null -ne $targetLuaRsSeconds -or $null -ne $baseNativeSeconds -or $null -ne $targetNativeSeconds) {
            $runtimeRows.Add([pscustomobject]@{
                Benchmark = $benchmark
                BaseLuaRsSec = Format-OptionalNumber -Value $baseLuaRsSeconds
                TargetLuaRsSec = Format-OptionalNumber -Value $targetLuaRsSeconds
                DeltaLuaRsSec = Format-OptionalNumber -Value (Get-OptionalDelta -BaseValue $baseLuaRsSeconds -TargetValue $targetLuaRsSeconds)
                LuaRsPct = Format-PercentDelta -BaseValue $baseLuaRsSeconds -TargetValue $targetLuaRsSeconds -InvertSign
                BaseNativeSec = Format-OptionalNumber -Value $baseNativeSeconds
                TargetNativeSec = Format-OptionalNumber -Value $targetNativeSeconds
                NativePct = Format-PercentDelta -BaseValue $baseNativeSeconds -TargetValue $targetNativeSeconds -InvertSign
            }) | Out-Null
        }

        $jitRows.Add([pscustomobject]@{
            Benchmark = $benchmark
            DeltaRecordedTraces = Format-DeltaValue ((Get-NumericRowValue -Row $targetRow -PropertyName 'RecordedTraces') - (Get-NumericRowValue -Row $baseRow -PropertyName 'RecordedTraces'))
            DeltaRecordAborts = Format-DeltaValue ((Get-NumericRowValue -Row $targetRow -PropertyName 'RecordAborts') - (Get-NumericRowValue -Row $baseRow -PropertyName 'RecordAborts'))
            DeltaRootNative = Format-DeltaValue ((Get-NumericRowValue -Row $targetRow -PropertyName 'RootNativeDispatches') - (Get-NumericRowValue -Row $baseRow -PropertyName 'RootNativeDispatches'))
            DeltaSideNative = Format-DeltaValue ((Get-NumericRowValue -Row $targetRow -PropertyName 'SideNativeDispatches') - (Get-NumericRowValue -Row $baseRow -PropertyName 'SideNativeDispatches'))
            DeltaHelperDispatch = Format-DeltaValue ((Get-NumericRowValue -Row $targetRow -PropertyName 'HelperPlanDispatches') - (Get-NumericRowValue -Row $baseRow -PropertyName 'HelperPlanDispatches'))
            DeltaTableHelpers = Format-DeltaValue ((Get-NumericRowValue -Row $targetRow -PropertyName 'NativeProfileTableHelpers') - (Get-NumericRowValue -Row $baseRow -PropertyName 'NativeProfileTableHelpers'))
        }) | Out-Null
    }

    Write-Output ""
    Write-ColorHost "========================================" "Cyan"
    Write-ColorHost "  JIT Summary Comparison" "Cyan"
    Write-ColorHost "========================================" "Cyan"
    Write-ColorHost "Base:   $BaseReportPath" "Gray"
    Write-ColorHost "Target: $TargetReportPath" "Gray"

    if ($runtimeRows.Count -gt 0) {
        Write-Output ""
        Write-ColorHost "Runtime Delta" "Cyan"
        $runtimeRows |
            Sort-Object -Property Benchmark |
            Format-Table Benchmark, BaseLuaRsSec, TargetLuaRsSec, DeltaLuaRsSec, LuaRsPct, BaseNativeSec, TargetNativeSec, NativePct -AutoSize |
            Out-String -Width 220 |
            Write-Output
    }

    Write-Output ""
    Write-ColorHost "JIT Counter Delta" "Cyan"
    $jitRows |
        Sort-Object -Property Benchmark |
        Format-Table Benchmark, DeltaRecordedTraces, DeltaRecordAborts, DeltaRootNative, DeltaSideNative, DeltaHelperDispatch, DeltaTableHelpers -AutoSize |
        Out-String -Width 220 |
        Write-Output
}

if ($CompareLatest -or $CompareBaseReport -or $CompareTargetReport) {
    if ($CompareLatest) {
        $latestPaths = Get-LatestJitSummaryPaths -Directory '.\benchmark_reports'
        $CompareBaseReport = $latestPaths[0]
        $CompareTargetReport = $latestPaths[1]
    }

    if ([string]::IsNullOrWhiteSpace($CompareBaseReport) -or [string]::IsNullOrWhiteSpace($CompareTargetReport)) {
        throw 'Comparison requires both -CompareBaseReport and -CompareTargetReport, or use -CompareLatest.'
    }

    Write-JitStatsComparison -BaseReportPath $CompareBaseReport -TargetReportPath $CompareTargetReport
    return
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
        $luarsSamples = Get-BenchmarkRuntimeSamples -Lines $luarsResult.Output
        $nativeSamples = @()
        if ($null -ne $parsedStats) {
            $jitRows.Add([pscustomobject]@{
                Benchmark = $bench
                LuaRsTotalSeconds = Get-TotalRuntimeSeconds -Samples $luarsSamples
                LuaRsSampleCount = $luarsSamples.Count
                LuaRsSamples = @($luarsSamples)
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
                RootNativeDispatches = $parsedStats.RootNativeDispatches
                RootInterpreterDispatches = $parsedStats.RootInterpreterDispatches
                SideNativeDispatches = $parsedStats.SideNativeDispatches
                SideInterpreterDispatches = $parsedStats.SideInterpreterDispatches
                NativeExitIndexResolveAttempts = $parsedStats.NativeExitIndexResolveAttempts
                NativeExitIndexResolveHits = $parsedStats.NativeExitIndexResolveHits
                NativeProfileGuardSteps = $parsedStats.NativeProfileGuardSteps
                NativeProfileArithmeticHelpers = $parsedStats.NativeProfileArithmeticHelpers
                NativeProfileTableHelpers = $parsedStats.NativeProfileTableHelpers
                NativeProfileUpvalueHelpers = $parsedStats.NativeProfileUpvalueHelpers
                NativeProfileShiftHelpers = $parsedStats.NativeProfileShiftHelpers
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

    if ($JitStats -and $jitRows.Count -gt 0) {
        $nativeSamples = Get-BenchmarkRuntimeSamples -Lines $nativeResult.Output
        $lastRow = $jitRows[$jitRows.Count - 1]
        $lastRow | Add-Member -NotePropertyName NativeTotalSeconds -NotePropertyValue (Get-TotalRuntimeSeconds -Samples $nativeSamples) -Force
        $lastRow | Add-Member -NotePropertyName NativeSampleCount -NotePropertyValue $nativeSamples.Count -Force
        $lastRow | Add-Member -NotePropertyName NativeSamples -NotePropertyValue @($nativeSamples) -Force
    }

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