-- mem-access.lua
-- Benchmark: Large memory array random access (Cache thrashing)
-- Realism: Large datasets, huge game worlds, database caches

local floor = math.floor
local random = math.random

local N = tonumber(arg and arg[1]) or 100
local size = N * 20000 -- e.g. 100 * 20000 = 2,000,000 elements

local t = {}
-- 1. Population (Linear write)
for i = 1, size do
    t[i] = i
end

local sum = 0
local steps = size * 2

-- 2. Random Access (The Cache Killer)
-- We use a simple LCG instead of math.random to ensure deterministic behavior across benchmarks
local seed = 12345
local function fast_rand(max)
    seed = (seed * 1664525 + 1013904223) % 4294967296
    return (seed % max) + 1
end

for i = 1, steps do
    local idx = fast_rand(size)
    sum = sum + t[idx]
end

io.write(string.format("Memory sum: %d (Size: %d)\n", sum, size))
