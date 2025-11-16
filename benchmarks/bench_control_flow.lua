-- Benchmark: Control flow
local iterations = 10000000

print("=== Control Flow Benchmark ===")
print("Iterations:", iterations)

-- If-else branches
local start = os.clock()
local count = 0
for i = 1, iterations do
    if i % 2 == 0 then
        count = count + 1
    else
        count = count - 1
    end
end
local elapsed = os.clock() - start
print(string.format("If-else: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- While loop
start = os.clock()
local i = 0
local sum = 0
while i < iterations do
    sum = sum + i
    i = i + 1
end
elapsed = os.clock() - start
print(string.format("While loop: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Repeat-until loop
start = os.clock()
i = 0
sum = 0
repeat
    sum = sum + i
    i = i + 1
until i >= iterations
elapsed = os.clock() - start
print(string.format("Repeat-until: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

-- Nested loops
start = os.clock()
sum = 0
for i = 1, 1000 do
    for j = 1, 1000 do
        sum = sum + 1
    end
end
elapsed = os.clock() - start
print(string.format("Nested loops (1000x1000): %.3f seconds (%.2f M ops/sec)", elapsed, 1000000 / elapsed / 1000000))
