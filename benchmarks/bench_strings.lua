-- Benchmark: String operations
local iterations = 100000

print("=== String Operations Benchmark ===")
print("Iterations:", iterations)

-- String concatenation
local start = os.clock()
for i = 1, iterations do
    local s = "hello" .. "world" .. tostring(i)
end
local elapsed = os.clock() - start
print(string.format("String concatenation: %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- String length
local str = "Hello, World! " .. string.rep("x", 1000)
start = os.clock()
for i = 1, iterations do
    local len = #str
end
elapsed = os.clock() - start
print(string.format("String length: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- String.sub
start = os.clock()
for i = 1, iterations do
    local sub = string.sub(str, 1, 10)
end
elapsed = os.clock() - start
print(string.format("string.sub: %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- String.find
start = os.clock()
for i = 1, iterations do
    local pos = string.find(str, "World")
end
elapsed = os.clock() - start
print(string.format("string.find: %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- String.gsub
start = os.clock()
for i = 1, 10000 do
    local result = string.gsub(str, "x", "y")
end
elapsed = os.clock() - start
print(string.format("string.gsub (10k): %.3f seconds", elapsed))
