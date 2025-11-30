-- Benchmark: Multiple return values and select()
local iterations = 1000000

print("=== Multiple Returns & Select Benchmark ===")
print("Iterations:", iterations)

-- Single return
local function single_return()
    return 1
end

local start = os.clock()
for i = 1, iterations do
    local a = single_return()
end
local elapsed = os.clock() - start
print(string.format("Single return: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Multiple returns (3)
local function triple_return()
    return 1, 2, 3
end

start = os.clock()
for i = 1, iterations do
    local a, b, c = triple_return()
end
elapsed = os.clock() - start
print(string.format("Triple return: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Multiple returns (10)
local function many_return()
    return 1, 2, 3, 4, 5, 6, 7, 8, 9, 10
end

start = os.clock()
for i = 1, iterations do
    local a, b, c, d, e, f, g, h, i, j = many_return()
end
elapsed = os.clock() - start
print(string.format("10 returns: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Discarding extra returns
start = os.clock()
for i = 1, iterations do
    local a = many_return()  -- only capture first
end
elapsed = os.clock() - start
print(string.format("Discard extra returns: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- select('#', ...)
local function count_args(...)
    return select('#', ...)
end

start = os.clock()
for i = 1, iterations do
    local n = count_args(1, 2, 3, 4, 5)
end
elapsed = os.clock() - start
print(string.format("select('#', ...): %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- select(n, ...)
local function get_nth(n, ...)
    return select(n, ...)
end

start = os.clock()
for i = 1, iterations do
    local x = get_nth(3, 1, 2, 3, 4, 5)
end
elapsed = os.clock() - start
print(string.format("select(3, ...): %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Vararg pass-through
local function passthrough(...)
    return ...
end

start = os.clock()
for i = 1, iterations do
    local a, b, c = passthrough(1, 2, 3)
end
elapsed = os.clock() - start
print(string.format("Vararg passthrough: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Vararg to table
local function args_to_table(...)
    return {...}
end

start = os.clock()
for i = 1, iterations do
    local t = args_to_table(1, 2, 3, 4, 5)
end
elapsed = os.clock() - start
print(string.format("Vararg to table: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- table.pack
start = os.clock()
for i = 1, iterations do
    local t = table.pack(1, 2, 3, 4, 5)
end
elapsed = os.clock() - start
print(string.format("table.pack: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- table.unpack
local tbl = {1, 2, 3, 4, 5}
start = os.clock()
for i = 1, iterations do
    local a, b, c, d, e = table.unpack(tbl)
end
elapsed = os.clock() - start
print(string.format("table.unpack: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Return value as table constructor arg
local function returns_for_table()
    return 1, 2, 3
end

start = os.clock()
for i = 1, iterations do
    local t = {returns_for_table()}
end
elapsed = os.clock() - start
print(string.format("Returns in table ctor: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Return value as function arg
local function sum3(a, b, c)
    return a + b + c
end

start = os.clock()
local sum = 0
for i = 1, iterations do
    sum = sum3(triple_return())
end
elapsed = os.clock() - start
print(string.format("Returns as func args: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))
