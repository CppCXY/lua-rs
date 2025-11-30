-- Benchmark: String library functions (extended)
local iterations = 100000

print("=== String Library Extended Benchmark ===")
print("Iterations:", iterations)

local test_str = "The quick brown fox jumps over the lazy dog"
local long_str = string.rep("abcdefghij", 1000)  -- 10000 chars

-- string.upper/lower
local start = os.clock()
for i = 1, iterations do
    local s = string.upper(test_str)
end
local elapsed = os.clock() - start
print(string.format("string.upper: %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

start = os.clock()
for i = 1, iterations do
    local s = string.lower(test_str)
end
elapsed = os.clock() - start
print(string.format("string.lower: %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- string.reverse
start = os.clock()
for i = 1, iterations do
    local s = string.reverse(test_str)
end
elapsed = os.clock() - start
print(string.format("string.reverse: %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- string.rep
start = os.clock()
for i = 1, iterations do
    local s = string.rep("a", 100)
end
elapsed = os.clock() - start
print(string.format("string.rep (100 chars): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- string.byte
start = os.clock()
local sum = 0
for i = 1, iterations do
    sum = sum + string.byte(test_str, 10)
end
elapsed = os.clock() - start
print(string.format("string.byte: %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- string.char
start = os.clock()
for i = 1, iterations do
    local s = string.char(65, 66, 67, 68, 69)
end
elapsed = os.clock() - start
print(string.format("string.char (5 chars): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- string.format (simple)
start = os.clock()
for i = 1, iterations do
    local s = string.format("%d", i)
end
elapsed = os.clock() - start
print(string.format("string.format (%%d): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- string.format (complex)
start = os.clock()
for i = 1, iterations do
    local s = string.format("Name: %s, Value: %d, Price: %.2f", "item", i, i * 1.5)
end
elapsed = os.clock() - start
print(string.format("string.format (complex): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- string.match
start = os.clock()
for i = 1, iterations do
    local s = string.match(test_str, "(%w+)")
end
elapsed = os.clock() - start
print(string.format("string.match (simple): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- string.match (complex pattern)
start = os.clock()
for i = 1, iterations do
    local s = string.match(test_str, "(%a+)%s+(%a+)%s+(%a+)")
end
elapsed = os.clock() - start
print(string.format("string.match (3 captures): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- string.gmatch (reduced iterations)
local gmatch_iters = iterations // 100
start = os.clock()
for i = 1, gmatch_iters do
    for word in string.gmatch(test_str, "%w+") do
        local x = word
    end
end
elapsed = os.clock() - start
print(string.format("string.gmatch: %.3f seconds (%.2f K ops/sec)", elapsed, gmatch_iters / elapsed / 1000))

-- string.gsub (simple replacement)
local gsub_iters = iterations // 10
start = os.clock()
for i = 1, gsub_iters do
    local s = string.gsub(test_str, "fox", "cat")
end
elapsed = os.clock() - start
print(string.format("string.gsub (simple): %.3f seconds (%.2f K ops/sec)", elapsed, gsub_iters / elapsed / 1000))

-- string.gsub (pattern replacement)
start = os.clock()
for i = 1, gsub_iters do
    local s = string.gsub(test_str, "%a+", "X")
end
elapsed = os.clock() - start
print(string.format("string.gsub (pattern): %.3f seconds (%.2f K ops/sec)", elapsed, gsub_iters / elapsed / 1000))

-- Long string operations
local long_iters = iterations // 100
start = os.clock()
for i = 1, long_iters do
    local s = string.sub(long_str, 1000, 2000)
end
elapsed = os.clock() - start
print(string.format("string.sub (long str): %.3f seconds (%.2f K ops/sec)", elapsed, long_iters / elapsed / 1000))

start = os.clock()
for i = 1, long_iters do
    local pos = string.find(long_str, "defgh", 5000, true)
end
elapsed = os.clock() - start
print(string.format("string.find (long str): %.3f seconds (%.2f K ops/sec)", elapsed, long_iters / elapsed / 1000))

-- String comparison
local str1 = "hello world"
local str2 = "hello world"
local str3 = "hello world!"
start = os.clock()
for i = 1, iterations do
    local eq = (str1 == str2)
    local neq = (str1 == str3)
end
elapsed = os.clock() - start
print(string.format("String equality: %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- String concatenation with ..
start = os.clock()
for i = 1, iterations do
    local s = "Hello" .. " " .. "World" .. "!"
end
elapsed = os.clock() - start
print(string.format("Concat (4 parts): %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))
