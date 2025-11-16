-- Benchmark: Function calls
local iterations = 1000000

print("=== Function Call Benchmark ===")
print("Iterations:", iterations)

-- Simple function call
local function add(a, b)
    return a + b
end

local start = os.clock()
local sum = 0
for i = 1, iterations do
    sum = add(i, 5)
end
local elapsed = os.clock() - start
print(string.format("Simple function call: %.3f seconds (%.2f M calls/sec)", elapsed, iterations / elapsed / 1000000))

-- Recursive function (Fibonacci)
local function fib(n)
    if n <= 1 then return n end
    return fib(n-1) + fib(n-2)
end

start = os.clock()
local result = fib(25)
elapsed = os.clock() - start
print(string.format("Recursive fib(25): %.3f seconds, result=%d", elapsed, result))

-- Vararg function
local function vararg_sum(...)
    local sum = 0
    for i, v in ipairs({...}) do
        sum = sum + v
    end
    return sum
end

start = os.clock()
for i = 1, iterations do
    sum = vararg_sum(1, 2, 3, 4, 5)
end
elapsed = os.clock() - start
print(string.format("Vararg function: %.3f seconds (%.2f M calls/sec)", elapsed, iterations / elapsed / 1000000))
