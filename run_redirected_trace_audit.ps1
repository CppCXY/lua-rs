#!/usr/bin/env pwsh

param(
    [switch]$NoColor,
    [string[]]$Benchmarks = @(
        "bench_quicksort.lua",
        "bench_control_flow.lua",
        "bench_iterators.lua",
        "bench_tables.lua"
    ),
    [string]$LuaRs = ".\\target\\release\\lua.exe",
    [switch]$SkipBuild,
    [switch]$OnlyFallbacks,
    [int]$TopFallbackTargets = 8,
    [switch]$SaveReport = $true
)

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
    param([string]$Executable)

    if ($SkipBuild -and (Test-Path $Executable)) {
        return
    }

    if (-not (Test-Path $Executable) -or -not $SkipBuild) {
        Write-ColorHost "Building lua-rs release binary with jit feature..." "Yellow"
        cargo build --release -p luars_interpreter --bin lua --features jit
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to build lua-rs JIT-enabled release binary."
        }
    }
}

function Invoke-BenchmarkRuntime {
    param(
        [string]$Executable,
        [string]$ScriptPath
    )

    $stdoutPath = [System.IO.Path]::GetTempFileName()
    $stderrPath = [System.IO.Path]::GetTempFileName()
    try {
        $process = Start-Process `
            -FilePath $Executable `
            -ArgumentList @("--jit-stats", "--jit-trace-report", $ScriptPath) `
            -NoNewWindow `
            -Wait `
            -PassThru `
            -RedirectStandardOutput $stdoutPath `
            -RedirectStandardError $stderrPath

        $output = @()
        if (Test-Path $stdoutPath) {
            $output += @(Get-Content -Path $stdoutPath -ErrorAction SilentlyContinue)
        }
        if (Test-Path $stderrPath) {
            $output += @(Get-Content -Path $stderrPath -ErrorAction SilentlyContinue)
        }

        if ($process.ExitCode -ne 0) {
            throw "Benchmark failed for $ScriptPath with exit code $($process.ExitCode)`n$($output | Out-String)"
        }

        return @($output | ForEach-Object { [string]$_ })
    } finally {
        Remove-Item -Path $stdoutPath, $stderrPath -ErrorAction SilentlyContinue
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

        if ($line -match '^\-\s+([^:]+):\s+(.+)$') {
            $stats[(Convert-StatKey $matches[1])] = Parse-StatValue $matches[2]
            continue
        }

        break
    }

    if ($stats.Count -eq 0) {
        return $null
    }

    return [pscustomobject]$stats
}

function Get-RedirectedSlots {
    param([object[]]$Lines)

    $slots = New-Object System.Collections.Generic.List[object]
    foreach ($lineObject in $Lines) {
        $line = [string]$lineObject
        if ($line -match '^\- chunk=(0x[0-9a-f]+) pc=(\d+) status=Redirected\(root_pc=(\d+)\)') {
            $slots.Add([pscustomobject]@{
                Chunk = $matches[1]
                Pc = [int]$matches[2]
                RootPc = [int]$matches[3]
                Key = ("{0}:{1}" -f $matches[1], $matches[2])
            }) | Out-Null
        }
    }

    return $slots.ToArray()
}

function Get-LinkedRootReentryTargets {
    param([object[]]$Lines)

    $targets = New-Object System.Collections.Generic.List[object]
    $inSection = $false
    foreach ($lineObject in $Lines) {
        $line = [string]$lineObject
        if (-not $inSection) {
            if ($line -eq "Linked root reentry by target header:") {
                $inSection = $true
            }
            continue
        }

        if ([string]::IsNullOrWhiteSpace($line)) {
            break
        }

        if ($line -match '^\- chunk=(0x[0-9a-f]+) pc=(\d+) attempts=(\d+) hits=(\d+) fallbacks=(\d+)$') {
            $targets.Add([pscustomobject]@{
                Chunk = $matches[1]
                Pc = [int]$matches[2]
                Attempts = [int]$matches[3]
                Hits = [int]$matches[4]
                Fallbacks = [int]$matches[5]
                Key = ("{0}:{1}" -f $matches[1], $matches[2])
            }) | Out-Null
            continue
        }

        break
    }

    return $targets.ToArray()
}

function Join-AuditSummary {
    param(
        [psobject]$Stats,
        [object[]]$RedirectedSlots,
        [object[]]$LinkedTargets
    )

    $redirectedLookup = @{}
    foreach ($slot in $RedirectedSlots) {
        $redirectedLookup[$slot.Key] = $slot
    }

    $fallbackTargets = @($LinkedTargets | Where-Object { $_.Fallbacks -gt 0 })
    $redirectedFallbackTargets = @($fallbackTargets | Where-Object { $redirectedLookup.ContainsKey($_.Key) })
    $nonRedirectedFallbackTargets = @($fallbackTargets | Where-Object { -not $redirectedLookup.ContainsKey($_.Key) })

    return [pscustomobject]@{
        LinkedRootReentryAttempts = [int]($Stats.LinkedRootReentryAttempts | ForEach-Object { $_ })
        LinkedRootReentryHits = [int]($Stats.LinkedRootReentryHits | ForEach-Object { $_ })
        LinkedRootReentryFallbacks = [int]($Stats.LinkedRootReentryFallbacks | ForEach-Object { $_ })
        RedirectedSlotCount = $RedirectedSlots.Count
        RedirectedFallbackTargetCount = $redirectedFallbackTargets.Count
        NonRedirectedFallbackTargetCount = $nonRedirectedFallbackTargets.Count
        RedirectedSlots = $RedirectedSlots
        FallbackTargets = $fallbackTargets
    }
}

Ensure-LuarsBinary -Executable $LuaRs

$rows = New-Object System.Collections.Generic.List[object]

Write-Output ""
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "  Redirected Trace Audit" "Cyan"
Write-ColorHost "========================================" "Cyan"
Write-ColorHost "Lua-RS: $LuaRs" "Gray"
Write-Output ""

foreach ($bench in $Benchmarks) {
    $scriptPath = Join-Path ".\benchmarks" $bench
    $output = Invoke-BenchmarkRuntime -Executable $LuaRs -ScriptPath $scriptPath
    $stats = Get-JitStatsFromOutput -Lines $output
    if ($null -eq $stats) {
        throw "Failed to parse JIT stats for $bench"
    }

    $redirectedSlots = Get-RedirectedSlots -Lines $output
    $linkedTargets = Get-LinkedRootReentryTargets -Lines $output
    $summary = Join-AuditSummary -Stats $stats -RedirectedSlots $redirectedSlots -LinkedTargets $linkedTargets

    if ($OnlyFallbacks -and $summary.LinkedRootReentryFallbacks -eq 0) {
        continue
    }

    Write-Output ""
    Write-ColorHost ">>> $bench <<<" "Yellow"

    $rows.Add([pscustomobject]@{
        Benchmark = $bench
        LinkedRootReentryAttempts = $summary.LinkedRootReentryAttempts
        LinkedRootReentryHits = $summary.LinkedRootReentryHits
        LinkedRootReentryFallbacks = $summary.LinkedRootReentryFallbacks
        RedirectedSlotCount = $summary.RedirectedSlotCount
        RedirectedFallbackTargetCount = $summary.RedirectedFallbackTargetCount
        NonRedirectedFallbackTargetCount = $summary.NonRedirectedFallbackTargetCount
        RedirectedSlots = $summary.RedirectedSlots
        FallbackTargets = $summary.FallbackTargets
    }) | Out-Null

    Write-ColorHost (
        "linked reentry: attempts={0} hits={1} fallbacks={2} | redirected slots={3}" -f
        $summary.LinkedRootReentryAttempts,
        $summary.LinkedRootReentryHits,
        $summary.LinkedRootReentryFallbacks,
        $summary.RedirectedSlotCount
    ) "Gray"

    foreach ($target in @($summary.FallbackTargets | Select-Object -First $TopFallbackTargets)) {
        $classification = if ($summary.RedirectedSlots.Where({ $_.Key -eq $target.Key }).Count -gt 0) {
            "redirected"
        } else {
            "non-redirected"
        }
        Write-ColorHost (
            "fallback target: chunk={0} pc={1} attempts={2} hits={3} fallbacks={4} [{5}]" -f
            $target.Chunk,
            $target.Pc,
            $target.Attempts,
            $target.Hits,
            $target.Fallbacks,
            $classification
        ) "DarkYellow"
    }
}

Write-Output ""
Write-ColorHost "Audit Summary" "Cyan"
if ($rows.Count -eq 0) {
    Write-ColorHost "No benchmarks matched the selected audit filter." "Yellow"
} else {
    $rows.ToArray() |
    Select-Object Benchmark, LinkedRootReentryAttempts, LinkedRootReentryHits, LinkedRootReentryFallbacks, RedirectedSlotCount, RedirectedFallbackTargetCount, NonRedirectedFallbackTargetCount |
    Format-Table -AutoSize |
    Out-String -Width 240 |
    Write-Output
}

if ($SaveReport -and $rows.Count -gt 0) {
    $timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $reportPath = ".\benchmark_reports\redirected-trace-audit-$timestamp.json"
    $payload = [pscustomobject]@{
        generated_at = (Get-Date).ToString("o")
        rows = $rows.ToArray()
    }
    $payload | ConvertTo-Json -Depth 6 | Set-Content -Path $reportPath
    Write-ColorHost "Saved redirected trace audit to $reportPath" "Gray"
}
