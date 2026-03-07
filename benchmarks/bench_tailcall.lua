-- Benchmark: Tail call performance
local iterations = 10000000

print("=== Tail Call Benchmark ===")
print("Iterations:", iterations)

-- Tail-recursive sum
local function tailsum(n, acc)
    if n <= 0 then return acc end
    return tailsum(n - 1, acc + n)
end

local start = os.clock()
local result = tailsum(iterations, 0)
local elapsed = os.clock() - start
print(string.format("Tail-recursive sum(%dM): %.3f seconds, result=%.0f", iterations/1000000, elapsed, result))
print(string.format("  Throughput: %.2f M calls/sec", iterations / elapsed / 1000000))

-- Tail-recursive countdown (minimal work per call)
local function countdown(n)
    if n <= 0 then return 0 end
    return countdown(n - 1)
end

start = os.clock()
result = countdown(iterations)
elapsed = os.clock() - start
print(string.format("Tail-recursive countdown(%dM): %.3f seconds", iterations/1000000, elapsed))
print(string.format("  Throughput: %.2f M calls/sec", iterations / elapsed / 1000000))

-- Mutual tail recursion (A calls B, B calls A)
local function is_even(n)
    if n == 0 then return true end
    return is_odd(n - 1)
end

function is_odd(n)
    if n == 0 then return false end
    return is_even(n - 1)
end

local count = 1000000
start = os.clock()
for i = 1, 10 do
    result = is_even(count)
end
elapsed = os.clock() - start
print(string.format("Mutual tail recursion (10x%dM): %.3f seconds", count/1000000, elapsed))
print(string.format("  Throughput: %.2f M calls/sec", 10 * count / elapsed / 1000000))
