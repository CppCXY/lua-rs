-- Benchmark: numeric backward-jmp loops
-- Focused JIT sample for head/tail guarded compare+jmp loop traces.

local iterations = 20000
local inner = 512

print("=== Numeric Jmp Loop Benchmark ===")
print("Iterations:", iterations)
print("Inner count:", inner)

local start = os.clock()
local total = 0

for outer = 1, iterations do
    local i = 0
    local acc = 0
    while i < inner do
        acc = acc + i
        i = i + 1
    end
    total = total + acc
end

local elapsed = os.clock() - start
print(string.format("While compare loop: %.3f seconds (%.2f M inner iters/sec)", elapsed, iterations * inner / elapsed / 1000000))

start = os.clock()
local total_repeat = 0

for outer = 1, iterations do
    local i = 0
    local acc = 0
    repeat
        acc = acc + i
        i = i + 1
    until i >= inner
    total_repeat = total_repeat + acc
end

elapsed = os.clock() - start
print(string.format("Repeat compare loop: %.3f seconds (%.2f M inner iters/sec)", elapsed, iterations * inner / elapsed / 1000000))
print("Checks:", total, total_repeat)