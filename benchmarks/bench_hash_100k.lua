-- Isolated Hash Table Test (100k)
print("=== Isolated Hash Table Test ===")
print("Inserting 100,000 entries...")

local start = os.clock()
local ht = {}
for i = 1, 100000 do
    ht["key" .. i] = i
end
local elapsed = os.clock() - start

print(string.format("Time: %.4f seconds", elapsed))
print(string.format("Rate: %.2f K ops/sec", 100 / elapsed))
