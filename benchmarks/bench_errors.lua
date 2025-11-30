-- Benchmark: Error handling (pcall/xpcall)
local iterations = 100000

print("=== Error Handling Benchmark ===")
print("Iterations:", iterations)

-- pcall with successful function
local function safe_add(a, b)
    return a + b
end

local start = os.clock()
for i = 1, iterations do
    local ok, result = pcall(safe_add, i, 5)
end
local elapsed = os.clock() - start
print(string.format("pcall (success): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- pcall with error
local function will_error()
    error("test error")
end

start = os.clock()
for i = 1, iterations do
    local ok, err = pcall(will_error)
end
elapsed = os.clock() - start
print(string.format("pcall (error): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- xpcall with error handler
local function error_handler(err)
    return "handled: " .. tostring(err)
end

start = os.clock()
for i = 1, iterations do
    local ok, result = xpcall(will_error, error_handler)
end
elapsed = os.clock() - start
print(string.format("xpcall (error): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- pcall overhead comparison (direct call vs pcall)
start = os.clock()
for i = 1, iterations do
    local result = safe_add(i, 5)
end
elapsed = os.clock() - start
print(string.format("Direct call (baseline): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- pcall with multiple return values
local function multi_return(a, b, c)
    return a + 1, b + 2, c + 3
end

start = os.clock()
for i = 1, iterations do
    local ok, r1, r2, r3 = pcall(multi_return, 1, 2, 3)
end
elapsed = os.clock() - start
print(string.format("pcall (multi-return): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- assert (success)
start = os.clock()
for i = 1, iterations do
    local x = assert(i, "should not fail")
end
elapsed = os.clock() - start
print(string.format("assert (success): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- Type checking with pcall
local function type_checked_add(a, b)
    if type(a) ~= "number" or type(b) ~= "number" then
        error("expected numbers")
    end
    return a + b
end

start = os.clock()
for i = 1, iterations do
    local ok, result = pcall(type_checked_add, i, 5)
end
elapsed = os.clock() - start
print(string.format("pcall (type check): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))
