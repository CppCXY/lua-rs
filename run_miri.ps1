#!/usr/bin/env pwsh

param(
    [string]$Package = "luars",
    [string]$TestFilter = "miri_",
    [string]$Toolchain = "nightly",
    [string]$MiriFlags = "-Zmiri-disable-stacked-borrows -Zmiri-permissive-provenance",
    [switch]$SkipSetup,
    [switch]$NoCapture
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $repoRoot

$miriTempDir = Join-Path $repoRoot ".miri-tmp"
New-Item -ItemType Directory -Force $miriTempDir | Out-Null

$originalTemp = $env:TEMP
$originalTmp = $env:TMP
$originalMiriFlags = $env:MIRIFLAGS

try {
    $env:TEMP = (Resolve-Path $miriTempDir).Path
    $env:TMP = $env:TEMP
    $env:MIRIFLAGS = $MiriFlags

    Write-Host "Repo root: $repoRoot" -ForegroundColor Cyan
    Write-Host "TEMP/TMP: $($env:TEMP)" -ForegroundColor DarkGray
    Write-Host "MIRIFLAGS: $($env:MIRIFLAGS)" -ForegroundColor DarkGray

    if (-not $SkipSetup) {
        Write-Host "Installing Miri component..." -ForegroundColor Yellow
        rustup component add miri --toolchain $Toolchain

        Write-Host "Preparing Miri sysroot..." -ForegroundColor Yellow
        cargo +$Toolchain miri setup
    }

    $cargoArgs = @(
        "+$Toolchain",
        "miri",
        "test",
        "-p", $Package,
        "--lib"
    )

    if ($TestFilter) {
        $cargoArgs += $TestFilter
    }

    $cargoArgs += "--"

    if ($NoCapture) {
        $cargoArgs += "--nocapture"
    }

    Write-Host "Running: cargo $($cargoArgs -join ' ')" -ForegroundColor Green
    & cargo @cargoArgs

    if ($LASTEXITCODE -ne 0) {
        throw "Miri test run failed with exit code $LASTEXITCODE"
    }
}
finally {
    $env:TEMP = $originalTemp
    $env:TMP = $originalTmp
    $env:MIRIFLAGS = $originalMiriFlags
}