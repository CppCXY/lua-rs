param(
    [Parameter(Mandatory = $true)]
    [string]$Exe,

    [Parameter(Mandatory = $true)]
    [string]$ScriptPath,

    [string[]]$ScriptArgs = @(),

    [string]$SetupCode = "collectgarbage('collect')",

    [int]$Repeats = 1,

    [string]$Label = "run"
)

$ErrorActionPreference = 'Stop'

function ConvertTo-LuaSingleQuoted {
    param([Parameter(Mandatory = $true)][string]$Text)

    $escaped = $Text.Replace('\', '\\').Replace("'", "\\'")
    return "'$escaped'"
}

function Build-LuaWrapper {
    param(
        [Parameter(Mandatory = $true)][string]$ResolvedScriptPath,
        [Parameter(Mandatory = $true)][string[]]$ResolvedScriptArgs,
        [Parameter(Mandatory = $true)][string]$ResolvedSetupCode
    )

    $scriptLiteral = ConvertTo-LuaSingleQuoted -Text $ResolvedScriptPath
    $argEntries = [System.Collections.Generic.List[string]]::new()
    $argEntries.Add("[-1] = 'lua'")
    $argEntries.Add("[0] = $scriptLiteral")

    for ($i = 0; $i -lt $ResolvedScriptArgs.Count; $i++) {
        $argLiteral = ConvertTo-LuaSingleQuoted -Text $ResolvedScriptArgs[$i]
        $argEntries.Add("[$($i + 1)] = $argLiteral")
    }

    @(
        $ResolvedSetupCode
        "arg = { $($argEntries -join ', ') }"
        "local ok, err = pcall(dofile, $scriptLiteral)"
        "local gc_count_kb = collectgarbage('count')"
        "collectgarbage('collect')"
        "local gc_count_after_full_kb = collectgarbage('count')"
        "io.stderr:write(string.format('GC_COUNT_KB=%.1f\n', gc_count_kb))"
        "io.stderr:write(string.format('GC_COUNT_AFTER_FULL_KB=%.1f\n', gc_count_after_full_kb))"
        "if not ok then"
        "  io.stderr:write('RUN_ERROR=' .. tostring(err) .. '\n')"
        "  os.exit(1)"
        "end"
    ) -join "`n"
}

function Measure-LuaRun {
    param(
        [Parameter(Mandatory = $true)][string]$RunExe,
        [Parameter(Mandatory = $true)][string]$WrapperCode,
        [Parameter(Mandatory = $true)][string]$RunLabel,
        [Parameter(Mandatory = $true)][int]$Iteration
    )

    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $RunExe
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    [void]$psi.ArgumentList.Add('-e')
    [void]$psi.ArgumentList.Add($WrapperCode)

    $proc = [System.Diagnostics.Process]::new()
    $proc.StartInfo = $psi

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $null = $proc.Start()

    $maxWorkingSet = 0L
    $maxPrivate = 0L
    $maxPaged = 0L
    $samples = 0

    do {
        $proc.Refresh()
        if ($proc.WorkingSet64 -gt $maxWorkingSet) { $maxWorkingSet = $proc.WorkingSet64 }
        if ($proc.PrivateMemorySize64 -gt $maxPrivate) { $maxPrivate = $proc.PrivateMemorySize64 }
        if ($proc.PagedMemorySize64 -gt $maxPaged) { $maxPaged = $proc.PagedMemorySize64 }
        $samples++
    } while (-not $proc.WaitForExit(1))

    $proc.Refresh()
    if ($proc.WorkingSet64 -gt $maxWorkingSet) { $maxWorkingSet = $proc.WorkingSet64 }
    if ($proc.PrivateMemorySize64 -gt $maxPrivate) { $maxPrivate = $proc.PrivateMemorySize64 }
    if ($proc.PagedMemorySize64 -gt $maxPaged) { $maxPaged = $proc.PagedMemorySize64 }

    $stdout = $proc.StandardOutput.ReadToEnd().Trim()
    $stderr = $proc.StandardError.ReadToEnd().Trim()
    $watch.Stop()

    $gcCountKb = $null
    if ($stderr -match 'GC_COUNT_KB=([0-9]+(?:\.[0-9]+)?)') {
        $gcCountKb = [double]$Matches[1]
    }

    $gcCountAfterFullKb = $null
    if ($stderr -match 'GC_COUNT_AFTER_FULL_KB=([0-9]+(?:\.[0-9]+)?)') {
        $gcCountAfterFullKb = [double]$Matches[1]
    }

    [pscustomobject]@{
        label = $RunLabel
        iteration = $Iteration
        exit_code = $proc.ExitCode
        elapsed_ms = [math]::Round($watch.Elapsed.TotalMilliseconds, 1)
        peak_working_set_mb = [math]::Round($maxWorkingSet / 1MB, 2)
        sampled_peak_private_mb = [math]::Round($maxPrivate / 1MB, 2)
        sampled_peak_paged_mb = [math]::Round($maxPaged / 1MB, 2)
        samples = $samples
        gc_count_kb = $gcCountKb
        gc_count_after_full_kb = $gcCountAfterFullKb
        stdout = $stdout
        stderr = $stderr
    }
}

$resolvedExe = (Resolve-Path $Exe).Path
$resolvedScript = (Resolve-Path $ScriptPath).Path.Replace('\', '/')
$wrapper = Build-LuaWrapper -ResolvedScriptPath $resolvedScript -ResolvedScriptArgs $ScriptArgs -ResolvedSetupCode $SetupCode

$results = [System.Collections.Generic.List[object]]::new()
for ($iteration = 1; $iteration -le $Repeats; $iteration++) {
    $results.Add((Measure-LuaRun -RunExe $resolvedExe -WrapperCode $wrapper -RunLabel $Label -Iteration $iteration))
}

$results | ConvertTo-Json -Depth 4
