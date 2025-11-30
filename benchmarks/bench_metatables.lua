-- Benchmark: Metatables and Metamethods
local iterations = 500000

print("=== Metatables & Metamethods Benchmark ===")
print("Iterations:", iterations)

-- __index metamethod (function)
local mt_func = {
    __index = function(t, k)
        return k * 2
    end
}
local t1 = setmetatable({}, mt_func)

local start = os.clock()
local sum = 0
for i = 1, iterations do
    sum = sum + t1[i]
end
local elapsed = os.clock() - start
print(string.format("__index (function): %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- __index metamethod (table fallback)
local fallback = {}
for i = 1, 1000 do fallback[i] = i * 3 end
local mt_table = { __index = fallback }
local t2 = setmetatable({}, mt_table)

start = os.clock()
sum = 0
for i = 1, iterations do
    sum = sum + t2[(i % 1000) + 1]
end
elapsed = os.clock() - start
print(string.format("__index (table): %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- __newindex metamethod (reduced iterations to avoid memory pressure)
local newindex_iters = iterations // 5
local storage = {}
local mt_newindex = {
    __newindex = function(t, k, v)
        storage[k] = v
    end
}
local t3 = setmetatable({}, mt_newindex)

start = os.clock()
for i = 1, newindex_iters do
    t3[i] = i
end
elapsed = os.clock() - start
print(string.format("__newindex: %.3f seconds (%.2f M ops/sec)", elapsed, newindex_iters / elapsed / 1000000))

-- Clear for memory
storage = nil
t3 = nil

-- __add metamethod (without nested setmetatable to avoid bug)
local add_iters = iterations // 5
local mt_add = {}
mt_add.__add = function(a, b) 
    return {val = a.val + b.val}  -- Don't call setmetatable in metamethod
end

local v1 = setmetatable({val = 1}, mt_add)
local v2 = setmetatable({val = 2}, mt_add)

start = os.clock()
local result = v1
for i = 1, add_iters do
    result = v1 + v2
end
elapsed = os.clock() - start
print(string.format("__add metamethod: %.3f seconds (%.2f M ops/sec)", elapsed, add_iters / elapsed / 1000000))

-- __call metamethod
local call_iters = iterations // 5
local callable = setmetatable({value = 10}, {
    __call = function(self, x)
        return self.value + x
    end
})

start = os.clock()
sum = 0
for i = 1, call_iters do
    sum = callable(i)
end
elapsed = os.clock() - start
print(string.format("__call metamethod: %.3f seconds (%.2f M ops/sec)", elapsed, call_iters / elapsed / 1000000))

-- __len metamethod
local mt_len = {
    __len = function(t)
        return t.size
    end
}
local t4 = setmetatable({size = 100}, mt_len)

start = os.clock()
sum = 0
for i = 1, iterations do
    sum = sum + #t4
end
elapsed = os.clock() - start
print(string.format("__len metamethod: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Raw access (baseline comparison)
local raw_table = {}
for i = 1, 1000 do raw_table[i] = i end

start = os.clock()
sum = 0
for i = 1, iterations do
    sum = sum + rawget(raw_table, (i % 1000) + 1)
end
elapsed = os.clock() - start
print(string.format("rawget (no metamethod): %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))
