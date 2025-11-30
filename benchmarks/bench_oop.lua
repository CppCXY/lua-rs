-- Benchmark: Object-Oriented Programming patterns
local iterations = 100000  -- Reduced for closure-heavy operations

print("=== OOP Patterns Benchmark ===")
print("Iterations:", iterations)

-- Simple class with methods
local Point = {}
Point.__index = Point

function Point.new(x, y)
    return setmetatable({x = x, y = y}, Point)
end

function Point:distance(other)
    local dx = self.x - other.x
    local dy = self.y - other.y
    return math.sqrt(dx*dx + dy*dy)
end

function Point:move(dx, dy)
    self.x = self.x + dx
    self.y = self.y + dy
end

-- Object creation
local start = os.clock()
for i = 1, iterations do
    local p = Point.new(i, i)
end
local elapsed = os.clock() - start
print(string.format("Object creation: %.3f seconds (%.2f K ops/sec)", elapsed, iterations / elapsed / 1000))

-- Method call (with self)
local p1 = Point.new(0, 0)
local p2 = Point.new(3, 4)
local method_iters = iterations * 5
start = os.clock()
for i = 1, method_iters do
    local d = p1:distance(p2)
end
elapsed = os.clock() - start
print(string.format("Method call (colon): %.3f seconds (%.2f K ops/sec)", elapsed, method_iters / elapsed / 1000))

-- Method call (dot notation)
start = os.clock()
for i = 1, method_iters do
    local d = Point.distance(p1, p2)
end
elapsed = os.clock() - start
print(string.format("Method call (dot): %.3f seconds (%.2f K ops/sec)", elapsed, method_iters / elapsed / 1000))

-- Inheritance (reduced iterations)
local inherit_iters = iterations / 5
local ColorPoint = setmetatable({}, {__index = Point})
ColorPoint.__index = ColorPoint

function ColorPoint.new(x, y, color)
    local self = setmetatable(Point.new(x, y), ColorPoint)
    self.color = color
    return self
end

function ColorPoint:getColor()
    return self.color
end

start = os.clock()
for i = 1, inherit_iters do
    local cp = ColorPoint.new(i, i, "red")
end
elapsed = os.clock() - start
print(string.format("Inherited object creation: %.3f seconds (%.2f K ops/sec)", elapsed, inherit_iters / elapsed / 1000))

-- Inherited method call
local cp = ColorPoint.new(0, 0, "blue")
start = os.clock()
for i = 1, method_iters do
    local d = cp:distance(p2)  -- calls Point:distance
end
elapsed = os.clock() - start
print(string.format("Inherited method call: %.3f seconds (%.2f K ops/sec)", elapsed, method_iters / elapsed / 1000))

-- Property access
local prop_iters = iterations * 5
start = os.clock()
local sum = 0
for i = 1, prop_iters do
    sum = sum + p1.x + p1.y
end
elapsed = os.clock() - start
print(string.format("Property access: %.3f seconds (%.2f K ops/sec)", elapsed, prop_iters / elapsed / 1000))

-- Property modification
start = os.clock()
for i = 1, prop_iters do
    p1.x = i
    p1.y = i
end
elapsed = os.clock() - start
print(string.format("Property modification: %.3f seconds (%.2f K ops/sec)", elapsed, prop_iters / elapsed / 1000))

-- Closure-based OOP (module pattern) - reduced due to GC overhead
local closure_iters = iterations / 10
local function createCounter(initial)
    local count = initial or 0
    return {
        increment = function() count = count + 1 end,
        decrement = function() count = count - 1 end,
        get = function() return count end
    }
end

start = os.clock()
for i = 1, closure_iters do
    local c = createCounter(0)
end
elapsed = os.clock() - start
print(string.format("Closure object creation: %.3f seconds (%.2f K ops/sec)", elapsed, closure_iters / elapsed / 1000))

local counter = createCounter(0)
start = os.clock()
for i = 1, method_iters do
    counter.increment()
end
elapsed = os.clock() - start
print(string.format("Closure method call: %.3f seconds (%.2f K ops/sec)", elapsed, method_iters / elapsed / 1000))

-- Prototype chain lookup (3 levels)
local A = {a_method = function(self) return 1 end}
A.__index = A

local B = setmetatable({b_method = function(self) return 2 end}, {__index = A})
B.__index = B

local C = setmetatable({c_method = function(self) return 3 end}, {__index = B})
C.__index = C

local obj = setmetatable({}, C)

start = os.clock()
for i = 1, method_iters do
    local x = obj:a_method()  -- 3 levels up
end
elapsed = os.clock() - start
print(string.format("Prototype chain (3 levels): %.3f seconds (%.2f K ops/sec)", elapsed, method_iters / elapsed / 1000))
