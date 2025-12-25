#!/usr/bin/env pwsh

param(
    [Parameter(Mandatory=$true)]
    [string]$LuaFile
)

$ErrorActionPreference = "Stop"

$baseName = [System.IO.Path]::GetFileNameWithoutExtension($LuaFile)
$luacPath = "lua_src\lua-5.5.0\build\Release\luac.exe"
$bytecode_dumpPath = "target\release\bytecode_dump.exe"

# 检查文件是否存在
if (-not (Test-Path $LuaFile)) {
    Write-Error "Lua file not found: $LuaFile"
    exit 1
}

if (-not (Test-Path $luacPath)) {
    Write-Error "luac.exe not found: $luacPath"
    exit 1
}

if (-not (Test-Path $bytecode_dumpPath)) {
    Write-Error "bytecode_dump.exe not found. Building..."
    cargo build --release --bin bytecode_dump
}

# 生成官方 Lua 字节码
Write-Host "Generating official Lua bytecode..." -ForegroundColor Green
& $luacPath -o "${baseName}_official.luac" $LuaFile

# 使用官方 luac 列出字节码
Write-Host "`n=== Official Lua Bytecode (luac -l) ===" -ForegroundColor Cyan
& $luacPath -l $LuaFile | Out-File -FilePath "${baseName}_official.txt" -Encoding utf8
Get-Content "${baseName}_official.txt"

# 使用我们的实现生成字节码
Write-Host "`n=== Our Implementation Bytecode ===" -ForegroundColor Cyan
& $bytecode_dumpPath $LuaFile | Out-File -FilePath "${baseName}_ours.txt" -Encoding utf8
Get-Content "${baseName}_ours.txt"

# 提示用户对比
Write-Host "`n=== Comparison ===" -ForegroundColor Yellow
Write-Host "Official output saved to: ${baseName}_official.txt"
Write-Host "Our output saved to: ${baseName}_ours.txt"
Write-Host "`nYou can compare them using:"
Write-Host "  code --diff ${baseName}_official.txt ${baseName}_ours.txt"
