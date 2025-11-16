-- Simple hash table insertion test
local start = os.clock()
local ht = {}
for i = 1, 100000 do
    ht["key" .. i] = i
end
local elapsed = os.clock() - start
print(string.format("Hash table insertion (100k): %.3f seconds", elapsed))

-- Test retrieval
start = os.clock()
local sum = 0
for i = 1, 100000 do
    sum = sum + (ht["key" .. i] or 0)
end
elapsed = os.clock() - start
print(string.format("Hash table retrieval (100k): %.3f seconds, sum=%d", elapsed, sum))
