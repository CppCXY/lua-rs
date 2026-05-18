param(
	[ValidateSet("debug", "release")]
	[string]$Profile = "debug",

	[string]$Script = "all.lua",

	[switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

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

$exe = if ($Profile -eq "debug") {
	"../../target/debug/lua.exe"
}
else {
	"../../target/release/lua.exe"
}

Push-Location "lua_tests/testes"
try {
	& $exe $Script
	exit $LASTEXITCODE
}
finally {
	Pop-Location
}