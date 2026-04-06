local iterations = 10000000

print("=== Boolean Control Benchmark ===")
print("Iterations:", iterations)

local start = os.clock()
local acc = 0
local stop = false
for n = 1, iterations do
    local keep_running = true
    while keep_running do
        acc = acc + n
        keep_running = stop
    end
end
local elapsed = os.clock() - start
print(string.format("While boolean guard: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

start = os.clock()
acc = 0
local done_value = true
for n = 1, iterations do
    local done = false
    repeat
        acc = acc + n
        done = done_value
    until done
end
elapsed = os.clock() - start
print(string.format("Repeat boolean guard: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))

start = os.clock()
acc = 0
local flag = false
for n = 1, iterations do
    flag = not flag
    local selected = flag and n or 1
    acc = acc + selected
end
elapsed = os.clock() - start
print(string.format("Boolean select: %.3f seconds (%.2f M ops/sec)", elapsed, iterations / elapsed / 1000000))