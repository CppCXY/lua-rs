-- Benchmark: Coroutines
local iterations = 100000

print("=== Coroutines Benchmark ===")
print("Iterations:", iterations)

-- Coroutine create/resume/yield cycle
local start = os.clock()
for i = 1, iterations do
    local co = coroutine.create(function()
        coroutine.yield(1)
        return 2
    end)
    coroutine.resume(co)
    coroutine.resume(co)
end
local elapsed = os.clock() - start
print(string.format("Create/resume/yield: %.3f seconds (%.2f K cycles/sec)", elapsed, iterations / elapsed / 1000))

-- Repeated yield in single coroutine
local co = coroutine.create(function()
    for i = 1, iterations do
        coroutine.yield(i)
    end
end)

start = os.clock()
for i = 1, iterations do
    coroutine.resume(co)
end
elapsed = os.clock() - start
print(string.format("Repeated yield: %.3f seconds (%.2f K yields/sec)", elapsed, iterations / elapsed / 1000))

-- Producer-consumer pattern
local function producer()
    for i = 1, iterations do
        coroutine.yield(i)
    end
end

local function consumer(prod)
    local sum = 0
    while true do
        local ok, val = coroutine.resume(prod)
        if not ok or coroutine.status(prod) == "dead" then break end
        sum = sum + val
    end
    return sum
end

start = os.clock()
local prod = coroutine.create(producer)
local total = consumer(prod)
elapsed = os.clock() - start
print(string.format("Producer-consumer: %.3f seconds (%.2f K msgs/sec)", elapsed, iterations / elapsed / 1000))

-- coroutine.wrap (simplified - has bug with upvalue capture)
local wrap_iters = iterations / 10
start = os.clock()
for i = 1, wrap_iters do
    local f = coroutine.wrap(function()
        return 42
    end)
    local result = f()
end
elapsed = os.clock() - start
print(string.format("coroutine.wrap: %.3f seconds (%.2f K ops/sec)", elapsed, wrap_iters / elapsed / 1000))

-- Coroutine status checks
co = coroutine.create(function()
    for i = 1, 10 do
        coroutine.yield()
    end
end)

start = os.clock()
for i = 1, iterations do
    local status = coroutine.status(co)
end
elapsed = os.clock() - start
print(string.format("coroutine.status: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))
