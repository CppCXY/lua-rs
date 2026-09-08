param(
    [int]$Nruns = 3,
    [string]$Output = 'results',
    [switch]$ShowErrors,
    [switch]$NoPlots,
    [string[]]$Binaries = @('lua', 'luars'),
    [string[]]$TestsFilter = @()
)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

# Point the `luars` entry at this repository's release build.
$repoRoot = (Resolve-Path -LiteralPath '..').Path
$targetExe = if ($IsWindows) { 'lua.exe' } else { 'lua' }
$luarsBinary = Join-Path $repoRoot (Join-Path 'target' (Join-Path 'release' $targetExe))
if ($Binaries -contains 'luars') {
    $Binaries = @($Binaries | ForEach-Object { if ($_ -eq 'luars') { $luarsBinary } else { $_ } })
}

$Tests = @(
    @{ Name = 'brainfuck';      Cmd = 'brainfuck.lua 10' },
    @{ Name = 'mem-access';     Cmd = 'mem-access.lua 150' },
    @{ Name = 'oop-dots';       Cmd = 'oop-dots.lua 100' },
    @{ Name = 'c-call';         Cmd = 'c-call.lua 550' },
    @{ Name = 'ray';            Cmd = 'ray.lua 768' },
    @{ Name = 'coro';           Cmd = 'coro-scheduler.lua 450' },
    @{ Name = 'json';           Cmd = 'json-serializer.lua 55' },
    @{ Name = 'heapsort';       Cmd = 'heapsort.lua 10 150000' },
    @{ Name = 'mandelbrot';     Cmd = 'mandel.lua' },
    @{ Name = 'juliaset';       Cmd = 'qt.lua' },
    @{ Name = 'queen';          Cmd = 'queen.lua 12' },
    @{ Name = 'binary';         Cmd = 'binary-trees.lua 14' },
    @{ Name = 'n-body';         Cmd = 'n-body.lua 800000' },
    @{ Name = 'fannkuch';       Cmd = 'fannkuch-redux.lua 10' },
    @{ Name = 'fasta';          Cmd = 'fasta.lua 2500000' },
    @{ Name = 'k-nucleotide';   Cmd = 'k-nucleotide.lua < fasta900000.txt' },
    @{ Name = 'regex-dna';      Cmd = 'regex-dna.lua < fasta900000.txt' },
    @{ Name = 'spectral-norm';  Cmd = 'spectral-norm.lua 1000' }
)

if ($TestsFilter.Count -gt 0) {
    $Tests = @($Tests | Where-Object { $TestsFilter -contains $_.Name })
}

$FastaInput = Join-Path $PSScriptRoot 'fasta900000.txt'

function Measure-Run {
    param([string]$Command)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    cmd /c $Command
    $exit = $LASTEXITCODE
    $sw.Stop()
    return [pscustomobject]@{ Seconds = $sw.Elapsed.TotalSeconds; ExitCode = $exit }
}

function Measure-Benchmark {
    param([string]$Binary, [string]$TestFile, [string]$Args)
    $cmd = "`"$Binary $TestFile $Args > nul 2>&1`""
    $min = [double]::MaxValue
    Write-Host "running  $Binary $TestFile $Args ... " -NoNewline
    for ($i = 1; $i -le $Nruns; $i++) {
        $r = Measure-Run $cmd
        if ($r.ExitCode -ne 0) {
            if ($ShowErrors) {
                Write-Host ''
                Write-Host "error: binary '$Binary' failed on '$TestFile' (exit $($r.ExitCode))" -ForegroundColor Red
            }
        } elseif ($r.Seconds -lt $min) {
            $min = $r.Seconds
        }
    }
    Write-Host 'done'
    if ($min -eq [double]::MaxValue) { return $null }
    return $min
}

function Get-DerivedResults {
    param($Source, [scriptblock]$Calc)
    $new = @()
    for ($i = 0; $i -lt $Source.Count; $i++) {
        $line = $Source[$i]
        $base = $line[0]
        $out = @()
        for ($j = 0; $j -lt $line.Count; $j++) {
            $out += & $Calc $line[$j] $base
        }
        $new += , @($out)
    }
    return , $new
}

function Save-DataFile {
    param([string]$Filename, $Results)
    $lines = @()
    $lines += "test`t" + ($Binaries -join "`t")
    for ($i = 0; $i -lt $Tests.Count; $i++) {
        $row = $Tests[$i].Name
        foreach ($v in $Results[$i]) {
            if ($null -eq $v) { $row += "`tNaN" }
            else { $row += "`t" + $v.ToString('F4') }
        }
        $lines += $row
    }
    Set-Content -LiteralPath $Filename -Value $lines -Encoding ascii
    Write-Host "Saved: $Filename"
}

function Test-Pred {
    param([string]$P)
    return (Get-Command $P -ErrorAction SilentlyContinue) -ne $null
}

Write-Host "Binaries: $($Binaries -join ', ')"
Write-Host "Nruns:    $Nruns"
Write-Host ''

foreach ($b in $Binaries) {
    $found = if (Test-Path -LiteralPath $b -PathType Leaf) { $true } else { Test-Pred $b }
    if (-not $found) {
        Write-Warning "Binary '$b' not found - results will be NaN"
    }
}

if (-not (Test-Path -LiteralPath $FastaInput)) {
    Write-Host "Generating input file (fasta900000.txt) ..."
    $r = Measure-Run "`"$($Binaries[0]) fasta.lua 900000 > fasta900000.txt 2>&1`""
    if ($r.ExitCode -ne 0) {
        throw "Failed to generate fasta900000.txt with $($Binaries[0])"
    }
}

$results = @()
foreach ($t in $Tests) {
    $row = @()
    foreach ($b in $Binaries) {
        $row += Measure-Benchmark $b $t.Cmd
    }
    $results += , @($row)
}

Remove-Item -LiteralPath $FastaInput -ErrorAction SilentlyContinue

Save-DataFile "$Output.dat" $results

$norm = Get-DerivedResults $results { param($v, $base)
    if ($null -eq $v -or $v -eq 0) { return 0 }
    if ($null -eq $base) { return $v }
    return $v / $base
}
Save-DataFile "$Output-norm.dat" $norm

$speed = Get-DerivedResults $results { param($v, $base)
    if ($null -eq $v -or $v -eq 0) { return 0 }
    if ($null -eq $base) { return $v }
    return $base / $v
}
Save-DataFile "$Output-speed.dat" $speed

Write-Host ''
Write-Host '==== Results (seconds, lower is better) ===='
$header = '{0,-16}' -f 'test'
foreach ($b in $Binaries) { $header += ('{0,14}' -f $b) }
Write-Host $header
for ($i = 0; $i -lt $Tests.Count; $i++) {
    $row = '{0,-16}' -f $Tests[$i].Name
    foreach ($v in $results[$i]) {
        if ($null -eq $v) { $row += '{0,14}' -f 'NaN' }
        else { $row += ('{0,14:F4}' -f $v) }
    }
    Write-Host $row
}

if (-not $NoPlots -and (Test-Pred 'gnuplot')) {
    foreach ($file in @("$Output.dat", "$Output-norm.dat", "$Output-speed.dat")) {
        if (Test-Path -LiteralPath "plot.gpi") {
            $name = [System.IO.Path]::GetFileNameWithoutExtension($file)
            $ylabel = switch ([System.IO.Path]::GetFileName($file)) {
                "$Output-norm.dat" { 'Normalized time (lower is better)' }
                "$Output-speed.dat" { 'Speedup factor (higher is better)' }
                default { 'Elapsed time (sec)' }
            }
            gnuplot -e "datafile='$file'" -e "outfile='$name.png'" -e "ylabel='$ylabel'" -e "nbinaries=$($Binaries.Count)" plot.gpi
            Write-Host "Created $name.png"
        }
    }
} elseif (-not $NoPlots) {
    Write-Host 'gnuplot not found in PATH - skipping PNG generation.'
}

Write-Host ''
Write-Host 'Benchmark complete.'
