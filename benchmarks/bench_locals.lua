-- Benchmark: Local vs Global variables
local iterations = 10000000

print("=== Local vs Global Benchmark ===")
print("Iterations:", iterations)

-- Global variable access
_G.global_var = 0
local start = os.clock()
for i = 1, iterations do
    global_var = global_var + 1
end
local elapsed = os.clock() - start
print(string.format("Global var access: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Local variable access
local local_var = 0
start = os.clock()
for i = 1, iterations do
    local_var = local_var + 1
end
elapsed = os.clock() - start
print(string.format("Local var access: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Upvalue access (from outer scope)
local upvalue_var = 0
local function upvalue_test()
    for i = 1, iterations do
        upvalue_var = upvalue_var + 1
    end
end
start = os.clock()
upvalue_test()
elapsed = os.clock() - start
print(string.format("Upvalue access: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Table field access (global)
_G.global_table = {value = 0}
start = os.clock()
for i = 1, iterations do
    global_table.value = global_table.value + 1
end
elapsed = os.clock() - start
print(string.format("Global table field: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Table field access (local)
local local_table = {value = 0}
start = os.clock()
for i = 1, iterations do
    local_table.value = local_table.value + 1
end
elapsed = os.clock() - start
print(string.format("Local table field: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Environment table lookup (_ENV)
start = os.clock()
for i = 1, iterations do
    local x = math.pi
end
elapsed = os.clock() - start
print(string.format("_ENV lookup (math.pi): %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Cached global function
local cached_floor = math.floor
start = os.clock()
for i = 1, iterations do
    local x = cached_floor(3.14)
end
elapsed = os.clock() - start
print(string.format("Cached global func: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))
