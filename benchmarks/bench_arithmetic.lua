-- Benchmark: Arithmetic operations
local iterations = 10000000

print("=== Arithmetic Benchmark ===")
print("Iterations:", iterations)

-- Integer addition
local start = os.clock()
local sum = 0
for i = 1, iterations do
    sum = sum + i
end
local elapsed = os.clock() - start
print(string.format("Integer addition: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Floating point
start = os.clock()
local result = 1.0
for i = 1, iterations do
    result = result * 1.0000001
end
elapsed = os.clock() - start
print(string.format("Float multiplication: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Mixed operations
start = os.clock()
local x, y, z = 0, 0, 0
for i = 1, iterations do
    x = i + 5
    y = x * 2
    z = y - 3
end
elapsed = os.clock() - start
print(string.format("Mixed operations: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))
