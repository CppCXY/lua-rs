#!/usr/bin/env pwsh

param(
    [switch]$Verbose,
    [switch]$StopOnError
)

$ErrorActionPreference = "Stop"

$luacPath = "lua_src\lua-5.5.0\build\Release\luac.exe"
$bytecode_dumpPath = "target\release\bytecode_dump.exe"
$testesDir = "lua_tests\testes"
$outputDir = "bytecode_comparison_output"

# 创建输出目录
if (-not (Test-Path $outputDir)) {
    New-Item -ItemType Directory -Path $outputDir | Out-Null
}

# 检查工具是否存在
if (-not (Test-Path $luacPath)) {
    Write-Error "luac.exe not found: $luacPath"
    exit 1
}

if (-not (Test-Path $bytecode_dumpPath)) {
    Write-Host "bytecode_dump.exe not found. Building..." -ForegroundColor Yellow
    cargo build --release --bin bytecode_dump
}

# 获取所有 .lua 文件
$luaFiles = Get-ChildItem -Path $testesDir -Filter "*.lua" | Sort-Object Name

Write-Host "Found $($luaFiles.Count) Lua files in $testesDir" -ForegroundColor Cyan
Write-Host ""

$totalFiles = 0
$passedFiles = 0
$failedFiles = 0
$skippedFiles = 0
$failedList = @()

foreach ($file in $luaFiles) {
    $totalFiles++
    $baseName = $file.BaseName
    $filePath = $file.FullName
    
    Write-Host "[$totalFiles/$($luaFiles.Count)] Testing: $($file.Name)" -NoNewline
    
    try {
        # 生成官方字节码
        $officialOutput = "$outputDir\${baseName}_official.txt"
        & $luacPath -l $filePath 2>&1 | Out-File -FilePath $officialOutput -Encoding utf8
        
        # 检查 luac 是否成功
        if ($LASTEXITCODE -ne 0) {
            Write-Host " [SKIP - luac failed]" -ForegroundColor Yellow
            $skippedFiles++
            continue
        }
        
        # 生成我们的字节码
        $ourOutput = "$outputDir\${baseName}_ours.txt"
        & $bytecode_dumpPath $filePath 2>&1 | Out-File -FilePath $ourOutput -Encoding utf8
        
        # 检查我们的工具是否成功
        if ($LASTEXITCODE -ne 0) {
            Write-Host " [FAIL - compilation error]" -ForegroundColor Red
            $failedFiles++
            $failedList += @{
                File = $file.Name
                Reason = "Compilation failed"
                OurOutput = $ourOutput
            }
            
            if ($StopOnError) {
                Write-Host ""
                Write-Host "Stopping due to error. Check: $ourOutput" -ForegroundColor Red
                exit 1
            }
            continue
        }
        
        # 读取并规范化输出进行比较
        # 现在我们的输出格式与官方一致：数字 [数字] 指令
        $officialLines = Get-Content $officialOutput | Where-Object { $_ -match '^\s*\d+\s+\[\d+\]\s+\w+' }
        $ourLines = Get-Content $ourOutput | Where-Object { $_ -match '^\s*\d+\s+\[\d+\]\s+\w+' }
        
        # 简单比较指令数量
        if ($officialLines.Count -ne $ourLines.Count) {
            Write-Host " [FAIL - instruction count mismatch: $($officialLines.Count) vs $($ourLines.Count)]" -ForegroundColor Red
            $failedFiles++
            $failedList += @{
                File = $file.Name
                Reason = "Instruction count mismatch: official=$($officialLines.Count), ours=$($ourLines.Count)"
                OfficialOutput = $officialOutput
                OurOutput = $ourOutput
            }
            
            if ($StopOnError) {
                Write-Host ""
                Write-Host "Official: $officialOutput" -ForegroundColor Yellow
                Write-Host "Ours: $ourOutput" -ForegroundColor Yellow
                exit 1
            }
            continue
        }
        
        # 详细比较每条指令
        $mismatch = $false
        for ($i = 0; $i -lt $officialLines.Count; $i++) {
            # 两边格式现在一致，都是：数字 [数字] 指令 参数 ; 注释
            $officialLine = $officialLines[$i] -replace '^\s*\d+\s+\[\d+\]\s+', '' -replace '\s+', ' ' -replace '\s*;.*$', ''
            $ourLine = $ourLines[$i] -replace '^\s*\d+\s+\[\d+\]\s+', '' -replace '\s+', ' ' -replace '\s*;.*$', ''
            
            # 规范化指令名称（移除注释和额外空格）
            $officialLine = $officialLine.Trim()
            $ourLine = $ourLine.Trim()
            
            if ($officialLine -ne $ourLine) {
                if (-not $mismatch) {
                    Write-Host " [FAIL - instruction mismatch at line $($i+1)]" -ForegroundColor Red
                    $mismatch = $true
                }
                
                if ($Verbose) {
                    Write-Host "  Line $($i+1):" -ForegroundColor Yellow
                    Write-Host "    Official: $officialLine" -ForegroundColor Gray
                    Write-Host "    Ours:     $ourLine" -ForegroundColor Gray
                }
            }
        }
        
        if ($mismatch) {
            $failedFiles++
            $failedList += @{
                File = $file.Name
                Reason = "Instruction mismatch"
                OfficialOutput = $officialOutput
                OurOutput = $ourOutput
            }
            
            if ($StopOnError) {
                Write-Host ""
                Write-Host "Use: code --diff `"$officialOutput`" `"$ourOutput`"" -ForegroundColor Yellow
                exit 1
            }
        } else {
            Write-Host " [PASS]" -ForegroundColor Green
            $passedFiles++
        }
        
    } catch {
        Write-Host " [ERROR - $($_.Exception.Message)]" -ForegroundColor Red
        $failedFiles++
        $failedList += @{
            File = $file.Name
            Reason = $_.Exception.Message
        }
        
        if ($StopOnError) {
            throw
        }
    }
}

Write-Host ""
Write-Host "==================== SUMMARY ====================" -ForegroundColor Cyan
Write-Host "Total files:   $totalFiles" -ForegroundColor White
Write-Host "Passed:        $passedFiles" -ForegroundColor Green
Write-Host "Failed:        $failedFiles" -ForegroundColor Red
Write-Host "Skipped:       $skippedFiles" -ForegroundColor Yellow
Write-Host "=================================================" -ForegroundColor Cyan

if ($failedFiles -gt 0) {
    Write-Host ""
    Write-Host "Failed files:" -ForegroundColor Red
    foreach ($failed in $failedList) {
        Write-Host "  - $($failed.File): $($failed.Reason)" -ForegroundColor Yellow
        if ($failed.OfficialOutput -and $failed.OurOutput) {
            Write-Host "    Diff: code --diff `"$($failed.OfficialOutput)`" `"$($failed.OurOutput)`"" -ForegroundColor Gray
        }
    }
    exit 1
} else {
    Write-Host ""
    Write-Host "All tests passed! 🎉" -ForegroundColor Green
    exit 0
}
