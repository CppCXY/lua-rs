param(
	[ValidateSet("debug", "release")]
	[string]$Profile = "debug",

	[string]$Script = "all.lua",

	[switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $repoRoot

if (-not $SkipBuild) {
	if ($Profile -eq "debug") {
		cargo build -p luars_interpreter
	}
	else {
		cargo build -p luars_interpreter --release
	}
}

if ($Profile -eq "debug") {
	if (-not $env:LUARS_MAIN_STACK_SIZE_MB) {
		$env:LUARS_MAIN_STACK_SIZE_MB = "128"
	}
	if (-not $env:LUARS_MAX_CALL_DEPTH) {
		$env:LUARS_MAX_CALL_DEPTH = "1024"
	}
	if (-not $env:LUARS_MAX_C_STACK_DEPTH) {
		$env:LUARS_MAX_C_STACK_DEPTH = "200"
	}
}

$exeName = if ($IsWindows) {
	"lua.exe"
}
else {
	"lua"
}

$exe = if ($Profile -eq "debug") {
	Join-Path $repoRoot "target/debug/$exeName"
}
else {
	Join-Path $repoRoot "target/release/$exeName"
}

if (-not (Test-Path $exe)) {
	throw "Lua interpreter not found at '$exe'. Build may have failed or produced the binary at a different path."
}

Push-Location "lua_tests/testes"
try {
	& $exe $Script
	exit $LASTEXITCODE
}
finally {
	Pop-Location
}