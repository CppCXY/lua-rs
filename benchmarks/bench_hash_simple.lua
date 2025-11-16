-- Simple Hash Table Test
print("=== Simple Hash Table Test ===")

print("\nTest 1: 1000 string keys")
local start = os.clock()
local t = {}
for i = 1, 1000 do
    t["key" .. i] = i
end
local elapsed = os.clock() - start
print(string.format("  Time: %.4f seconds", elapsed))

print("\nTest 2: 5000 string keys")
start = os.clock()
t = {}
for i = 1, 5000 do
    t["key" .. i] = i
end
elapsed = os.clock() - start
print(string.format("  Time: %.4f seconds", elapsed))

print("\nTest 3: Lookup 1000 keys")
start = os.clock()
local sum = 0
for i = 1, 1000 do
    sum = sum + (t["key" .. i] or 0)
end
elapsed = os.clock() - start
print(string.format("  Time: %.4f seconds, sum=%d", elapsed, sum))

print("\n=== Complete ===")
