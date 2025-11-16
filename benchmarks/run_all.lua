-- Benchmark runner script
-- Run all benchmarks and compare with native Lua

print("======================================")
print("   LUA-RS PERFORMANCE BENCHMARKS")
print("======================================")
print()

-- List of benchmark files
local benchmarks = {
    "bench_arithmetic.lua",
    "bench_functions.lua", 
    "bench_tables.lua",
    "bench_strings.lua",
    "bench_control_flow.lua"
}

for _, bench in ipairs(benchmarks) do
    print("\n--- Running: " .. bench .. " ---")
    local success, err = pcall(function()
        dofile("benchmarks/" .. bench)
    end)
    
    if not success then
        print("ERROR:", err)
    end
end

print("\n======================================")
print("   BENCHMARKS COMPLETE")
print("======================================")
