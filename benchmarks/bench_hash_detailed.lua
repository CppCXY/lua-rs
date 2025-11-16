-- Detailed hash table performance analysis
print("=== String Creation Test ===")
local start = os.clock()
local strings = {}
for i = 1, 100000 do
    strings[i] = "key" .. i
end
local elapsed = os.clock() - start
print(string.format("String creation (100k): %.3f seconds", elapsed))

-- Test with pre-created strings  
print("\n=== Hash Insertion (pre-created strings) ===")
start = os.clock()
local ht = {}
for i = 1, 100000 do
    ht[strings[i]] = i
end
elapsed = os.clock() - start
print(string.format("Hash insertion: %.3f seconds", elapsed))

-- Test retrieval
start = os.clock()
local sum = 0
for i = 1, 100000 do
    sum = sum + (ht[strings[i]] or 0)
end
elapsed = os.clock() - start
print(string.format("Hash retrieval: %.3f seconds, sum=%d", elapsed, sum))

-- Compare: inline concat + insert
print("\n=== Hash Insertion (inline concat) ===")
start = os.clock()
local ht2 = {}
for i = 1, 100000 do
    ht2["key" .. i] = i
end
elapsed = os.clock() - start
print(string.format("Inline concat + insert: %.3f seconds", elapsed))
