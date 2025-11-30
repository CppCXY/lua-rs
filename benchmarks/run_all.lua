-- Benchmark runner script
-- Run all benchmarks and compare with native Lua

print("======================================")
print("   LUA-RS PERFORMANCE BENCHMARKS")
print("======================================")
print()

-- List of benchmark files (organized by category)
local benchmarks = {
    -- Core operations
    "bench_arithmetic.lua",
    "bench_control_flow.lua",
    "bench_locals.lua",
    
    -- Functions
    "bench_functions.lua", 
    "bench_closures.lua",
    "bench_multiret.lua",
    
    -- Tables
    "bench_tables.lua",
    "bench_table_lib.lua",
    "bench_iterators.lua",
    
    -- Strings
    "bench_strings.lua",
    "bench_string_lib.lua",
    
    -- Math
    "bench_math.lua",
    
    -- Advanced features
    "bench_metatables.lua",
    "bench_oop.lua",
    "bench_coroutines.lua",
    "bench_errors.lua",
}

local total_time = os.clock()

for _, bench in ipairs(benchmarks) do
    print("\n--- Running: " .. bench .. " ---")
    local success, err = pcall(function()
        dofile("benchmarks/" .. bench)
    end)
    
    if not success then
        print("ERROR:", err)
    end
end

local elapsed = os.clock() - total_time

print("\n======================================")
print("   BENCHMARKS COMPLETE")
print(string.format("   Total time: %.2f seconds", elapsed))
print("======================================")
