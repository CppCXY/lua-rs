-- Benchmark: Math operations
local iterations = 5000000

print("=== Math Operations Benchmark ===")
print("Iterations:", iterations)

local pi = math.pi
local sqrt = math.sqrt
local sin = math.sin
local cos = math.cos
local floor = math.floor
local ceil = math.ceil
local abs = math.abs
local min = math.min
local max = math.max
local random = math.random

-- Integer operations
local start = os.clock()
local x = 0
for i = 1, iterations do
    x = (i * 3 + 7) % 1000
end
local elapsed = os.clock() - start
print(string.format("Integer mul/add/mod: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Float operations
start = os.clock()
local y = 0.0
for i = 1, iterations do
    y = (i * 1.5 + 0.7) / 2.3
end
elapsed = os.clock() - start
print(string.format("Float mul/add/div: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- math.sqrt
start = os.clock()
for i = 1, iterations do
    y = sqrt(i)
end
elapsed = os.clock() - start
print(string.format("math.sqrt: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- math.sin
start = os.clock()
for i = 1, iterations do
    y = sin(i * 0.001)
end
elapsed = os.clock() - start
print(string.format("math.sin: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- math.floor/ceil
start = os.clock()
for i = 1, iterations do
    x = floor(i * 1.7)
    x = ceil(i * 1.3)
end
elapsed = os.clock() - start
print(string.format("math.floor/ceil: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- math.min/max
start = os.clock()
for i = 1, iterations do
    x = min(i, 500000)
    x = max(i, 500000)
end
elapsed = os.clock() - start
print(string.format("math.min/max: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- math.abs
start = os.clock()
for i = 1, iterations do
    x = abs(i - 2500000)
end
elapsed = os.clock() - start
print(string.format("math.abs: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- math.random
math.randomseed(12345)
start = os.clock()
for i = 1, iterations do
    x = random(1, 100)
end
elapsed = os.clock() - start
print(string.format("math.random: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Bitwise operations (Lua 5.3+)
start = os.clock()
for i = 1, iterations do
    x = (i & 0xFF) | (i >> 4)
end
elapsed = os.clock() - start
print(string.format("Bitwise AND/OR/SHR: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Integer division
start = os.clock()
for i = 1, iterations do
    x = i // 7
end
elapsed = os.clock() - start
print(string.format("Integer division (//): %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Power operation
start = os.clock()
for i = 1, iterations do
    y = (i % 10 + 1) ^ 2
end
elapsed = os.clock() - start
print(string.format("Power (^2): %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))
