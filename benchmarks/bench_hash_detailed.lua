-- Detailed Hash Table Performance Analysis
print("=== Hash Table Detailed Benchmark ===")
print()

-- Test 1: Small hash tables (1000 entries)
print("Test 1: Small hash table (1000 entries)")
local start = os.clock()
local t = {}
for i = 1, 1000 do
    t["key" .. i] = i
end
local elapsed = os.clock() - start
print(string.format("  Insert: %.4f seconds", elapsed))

start = os.clock()
local sum = 0
for i = 1, 1000 do
    sum = sum + (t["key" .. i] or 0)
end
elapsed = os.clock() - start
print(string.format("  Lookup: %.4f seconds, sum=%d", elapsed, sum))

-- Test 2: Medium hash tables (10k entries)
print("\nTest 2: Medium hash table (10k entries)")
start = os.clock()
t = {}
for i = 1, 10000 do
    t["key" .. i] = i
end
elapsed = os.clock() - start
print(string.format("  Insert: %.4f seconds (%.2f K ops/sec)", elapsed, 10 / elapsed))

start = os.clock()
sum = 0
for i = 1, 10000 do
    sum = sum + (t["key" .. i] or 0)
end
elapsed = os.clock() - start
print(string.format("  Lookup: %.4f seconds (%.2f K ops/sec)", elapsed, 10 / elapsed))

-- Test 3: Different key types
print("\nTest 3: Integer keys (10k)")
start = os.clock()
t = {}
for i = 1, 10000 do
    t[i * 1000] = i  -- Non-sequential integer keys
end
elapsed = os.clock() - start
print(string.format("  Insert: %.4f seconds", elapsed))

start = os.clock()
sum = 0
for i = 1, 10000 do
    sum = sum + (t[i * 1000] or 0)
end
elapsed = os.clock() - start
print(string.format("  Lookup: %.4f seconds", elapsed))

-- Test 4: Mixed key types
print("\nTest 4: Mixed keys (5k string + 5k int)")
start = os.clock()
t = {}
for i = 1, 5000 do
    t["str" .. i] = i
    t[i * 100] = i
end
elapsed = os.clock() - start
print(string.format("  Insert: %.4f seconds", elapsed))

-- Test 5: Hash collisions (worst case)
print("\nTest 5: Sequential string keys (1k)")
start = os.clock()
t = {}
for i = 1, 1000 do
    t["k" .. string.format("%04d", i)] = i
end
elapsed = os.clock() - start
print(string.format("  Insert: %.4f seconds", elapsed))

-- Test 6: Iteration performance
print("\nTest 6: pairs iteration (10k entries)")
t = {}
for i = 1, 10000 do
    t["key" .. i] = i
end

start = os.clock()
sum = 0
for k, v in pairs(t) do
    sum = sum + v
end
elapsed = os.clock() - start
print(string.format("  Iteration: %.4f seconds, sum=%d", elapsed, sum))

-- Test 7: Table growth
print("\nTest 7: Progressive growth (0 -> 10k)")
start = os.clock()
t = {}
for i = 1, 10000 do
    t["k" .. i] = i
end
elapsed = os.clock() - start
print(string.format("  Progressive insert: %.4f seconds (%.2f K ops/sec)", elapsed, 10 / elapsed))

print("\n=== Analysis Complete ===")
