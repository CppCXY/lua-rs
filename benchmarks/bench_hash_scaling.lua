-- Medium Hash Table Test
print("=== Medium Hash Table Performance ===")

local sizes = {10000, 20000, 30000, 50000, 100000}

for _, size in ipairs(sizes) do
    print(string.format("\nInserting %d entries:", size))
    
    local start = os.clock()
    local t = {}
    for i = 1, size do
        t["key" .. i] = i
    end
    local elapsed = os.clock() - start
    
    print(string.format("  Time: %.4f seconds (%.2f K ops/sec)", 
        elapsed, size / elapsed / 1000))
end

print("\n=== Complete ===")
