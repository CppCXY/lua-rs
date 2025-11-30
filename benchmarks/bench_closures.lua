-- Benchmark: Closures and Upvalues
local iterations = 1000000

print("=== Closures & Upvalues Benchmark ===")
print("Iterations:", iterations)

-- Closure creation
local start = os.clock()
for i = 1, iterations do
    local x = i
    local f = function() return x end
end
local elapsed = os.clock() - start
print(string.format("Closure creation: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Closure with upvalue read
local counter = 0
local function make_counter()
    local count = 0
    return function()
        count = count + 1
        return count
    end
end

local inc = make_counter()
start = os.clock()
for i = 1, iterations do
    inc()
end
elapsed = os.clock() - start
print(string.format("Upvalue read/write: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Multiple upvalues
local function make_adder(a, b, c)
    return function(x)
        return a + b + c + x
    end
end

local adder = make_adder(1, 2, 3)
start = os.clock()
local sum = 0
for i = 1, iterations do
    sum = adder(i)
end
elapsed = os.clock() - start
print(string.format("Multiple upvalues: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Nested closures
local function outer(x)
    return function(y)
        return function(z)
            return x + y + z
        end
    end
end

local f1 = outer(1)
local f2 = f1(2)
start = os.clock()
for i = 1, iterations do
    sum = f2(i)
end
elapsed = os.clock() - start
print(string.format("Nested closures: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))
