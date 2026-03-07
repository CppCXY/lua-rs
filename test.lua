-- Benchmark: Iterators (ipairs, pairs, custom)
local iterations = 10000

print("=== Iterators Benchmark ===")
print("Iterations:", iterations)

-- Setup test data
local array_1k = {}
for i = 1, 1000 do array_1k[i] = i end

local hash_1k = {}
for i = 1, 1000 do hash_1k["key" .. i] = i end

local mixed_1k = {}
for i = 1, 500 do mixed_1k[i] = i end
for i = 1, 500 do mixed_1k["key" .. i] = i end

-- ipairs iteration
local start = os.clock()
local sum = 0
for i = 1, iterations do
    for idx, val in ipairs(array_1k) do
        sum = sum + val
    end
end
local elapsed = os.clock() - start
print(string.format("ipairs (1000 items): %.3f seconds (%.2f K iters/sec)", elapsed, iterations / elapsed / 1000))

-- pairs on array
start = os.clock()
sum = 0
for i = 1, iterations do
    for k, v in pairs(array_1k) do
        sum = sum + v
    end
end
elapsed = os.clock() - start
print(string.format("pairs on array (1000): %.3f seconds (%.2f K iters/sec)", elapsed, iterations / elapsed / 1000))

-- pairs on hash table
start = os.clock()
sum = 0
for i = 1, iterations do
    for k, v in pairs(hash_1k) do
        sum = sum + v
    end
end
elapsed = os.clock() - start
print(string.format("pairs on hash (1000): %.3f seconds (%.2f K iters/sec)", elapsed, iterations / elapsed / 1000))

-- pairs on mixed table
start = os.clock()
sum = 0
for i = 1, iterations do
    for k, v in pairs(mixed_1k) do
        sum = sum + v
    end
end
elapsed = os.clock() - start
print(string.format("pairs on mixed (1000): %.3f seconds (%.2f K iters/sec)", elapsed, iterations / elapsed / 1000))

-- next() function
start = os.clock()
sum = 0
for i = 1, iterations do
    local k, v = next(array_1k)
    while k do
        sum = sum + v
        k, v = next(array_1k, k)
    end
end
elapsed = os.clock() - start
print(string.format("next() iteration (1000): %.3f seconds (%.2f K iters/sec)", elapsed, iterations / elapsed / 1000))

-- Numeric for (comparison baseline)
start = os.clock()
sum = 0
for i = 1, iterations do
    for j = 1, 1000 do
        sum = sum + array_1k[j]
    end
end
elapsed = os.clock() - start
print(string.format("Numeric for (1000): %.3f seconds (%.2f K iters/sec)", elapsed, iterations / elapsed / 1000))

-- Custom iterator (stateless)
local function stateless_iter(t, i)
    i = i + 1
    local v = t[i]
    if v then return i, v end
end

local function my_ipairs(t)
    return stateless_iter, t, 0
end

start = os.clock()
sum = 0
for i = 1, iterations do
    for idx, val in my_ipairs(array_1k) do
        sum = sum + val
    end
end
elapsed = os.clock() - start
print(string.format("Custom stateless iter: %.3f seconds (%.2f K iters/sec)", elapsed, iterations / elapsed / 1000))

-- Custom iterator (closure-based)
local function range(from, to)
    local i = from - 1
    return function()
        i = i + 1
        if i <= to then return i end
    end
end

start = os.clock()
sum = 0
for i = 1, iterations do
    for j in range(1, 100) do
        sum = sum + j
    end
end
elapsed = os.clock() - start
print(string.format("Closure iterator (100): %.3f seconds (%.2f K iters/sec)", elapsed, iterations / elapsed / 1000))

-- Generic for with multiple values
local function multi_iter(t)
    local i = 0
    local n = #t
    return function()
        i = i + 1
        if i <= n then
            return i, t[i], t[i] * 2
        end
    end
end

start = os.clock()
sum = 0
for i = 1, iterations do
    for idx, val, doubled in multi_iter(array_1k) do
        sum = sum + doubled
    end
end
elapsed = os.clock() - start
print(string.format("Multi-value iterator: %.3f seconds (%.2f K iters/sec)", elapsed, iterations / elapsed / 1000))
