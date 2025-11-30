-- Benchmark: Table operations
local iterations = 1000000

print("=== Table Operations Benchmark ===")
print("Iterations:", iterations)

-- Array creation and access (reduced iterations due to GC overhead)
local array_iters = iterations // 100
local start = os.clock()
for i = 1, array_iters do
    local t = {1, 2, 3, 4, 5}
    local x = t[1] + t[5]
end
local elapsed = os.clock() - start
print(string.format("Array creation & access: %.3f seconds (%.2f M ops/sec)", elapsed, array_iters / elapsed / 1000000))

-- Table insertion
start = os.clock()
local t = {}
for i = 1, iterations do
    t[i] = i
end
elapsed = os.clock() - start
print(string.format("Table insertion: %.3f seconds (%.2f M inserts/sec)", elapsed, iterations / elapsed / 1000000))

-- Table iteration
start = os.clock()
local sum = 0
for i = 1, iterations do
    sum = sum + t[i]
end
elapsed = os.clock() - start
print(string.format("Table access: %.3f seconds (%.2f M accesses/sec)", elapsed, iterations / elapsed / 1000000))

-- Hash table operations
start = os.clock()
local ht = {}
for i = 1, 100000 do
    ht["key" .. i] = i
end
elapsed = os.clock() - start
print(string.format("Hash table insertion (100k): %.3f seconds", elapsed))

-- ipairs iteration (reduced)
local ipairs_iters = 10
start = os.clock()
sum = 0
for i = 1, ipairs_iters do
    for idx, val in ipairs(t) do
        sum = sum + val
    end
end
elapsed = os.clock() - start
print(string.format("ipairs iteration (%dx%d): %.3f seconds", ipairs_iters, iterations, elapsed))
