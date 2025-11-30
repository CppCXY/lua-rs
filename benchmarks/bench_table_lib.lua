-- Benchmark: Table library functions
local iterations = 100000

print("=== Table Library Benchmark ===")
print("Iterations:", iterations)

-- table.insert (at end)
local t = {}
local start = os.clock()
for i = 1, iterations do
    table.insert(t, i)
end
local elapsed = os.clock() - start
print(string.format("table.insert (end): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- table.insert (at position)
t = {1, 2, 3, 4, 5}
start = os.clock()
for i = 1, iterations / 10 do
    table.insert(t, 3, i)
    table.remove(t, 3)  -- Keep size stable
end
elapsed = os.clock() - start
print(string.format("table.insert (middle): %.3f seconds (%.2f K ops/sec)", elapsed, (iterations / 10) / elapsed / 1000))

-- table.remove (from end)
t = {}
for i = 1, iterations do t[i] = i end
start = os.clock()
for i = 1, iterations do
    table.remove(t)
end
elapsed = os.clock() - start
print(string.format("table.remove (end): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- table.concat
t = {}
for i = 1, 1000 do t[i] = "item" .. i end
start = os.clock()
for i = 1, iterations / 10 do
    local s = table.concat(t, ",")
end
elapsed = os.clock() - start
print(string.format("table.concat (1000 items): %.3f seconds (%.2f K ops/sec)", elapsed, (iterations / 10) / elapsed / 1000))

-- table.sort (already sorted)
t = {}
for i = 1, 1000 do t[i] = i end
start = os.clock()
for i = 1, iterations / 100 do
    table.sort(t)
end
elapsed = os.clock() - start
print(string.format("table.sort (sorted): %.3f seconds (%.2f K ops/sec)", elapsed, (iterations / 100) / elapsed / 1000))

-- table.sort (reverse order)
start = os.clock()
for i = 1, iterations / 100 do
    for j = 1, 1000 do t[j] = 1001 - j end
    table.sort(t)
end
elapsed = os.clock() - start
print(string.format("table.sort (reversed): %.3f seconds (%.2f K ops/sec)", elapsed, (iterations / 100) / elapsed / 1000))

-- table.sort (random order)
math.randomseed(12345)
start = os.clock()
for i = 1, iterations / 100 do
    for j = 1, 1000 do t[j] = math.random(1, 10000) end
    table.sort(t)
end
elapsed = os.clock() - start
print(string.format("table.sort (random): %.3f seconds (%.2f K ops/sec)", elapsed, (iterations / 100) / elapsed / 1000))

-- table.sort with custom comparator
t = {}
for i = 1, 1000 do t[i] = {value = math.random(1, 10000)} end
start = os.clock()
for i = 1, iterations / 100 do
    table.sort(t, function(a, b) return a.value < b.value end)
end
elapsed = os.clock() - start
print(string.format("table.sort (custom cmp): %.3f seconds (%.2f K ops/sec)", elapsed, (iterations / 100) / elapsed / 1000))

-- table.move
local src = {}
for i = 1, 1000 do src[i] = i end
local dst = {}
start = os.clock()
for i = 1, iterations / 10 do
    table.move(src, 1, 1000, 1, dst)
end
elapsed = os.clock() - start
print(string.format("table.move (1000 items): %.3f seconds (%.2f K ops/sec)", elapsed, (iterations / 10) / elapsed / 1000))

-- table.unpack
t = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10}
start = os.clock()
for i = 1, iterations do
    local a, b, c, d, e = table.unpack(t, 1, 5)
end
elapsed = os.clock() - start
print(string.format("table.unpack (5 values): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- table.pack
start = os.clock()
for i = 1, iterations do
    local t = table.pack(1, 2, 3, 4, 5)
end
elapsed = os.clock() - start
print(string.format("table.pack (5 values): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- # operator (array length)
t = {}
for i = 1, 10000 do t[i] = i end
start = os.clock()
local len = 0
for i = 1, iterations do
    len = #t
end
elapsed = os.clock() - start
print(string.format("# operator (10k array): %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))
