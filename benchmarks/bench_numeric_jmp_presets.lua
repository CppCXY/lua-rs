local iterations = tonumber(os.getenv("BENCH_ITERS")) or 5000000

print("=== Numeric Jmp Preset Benchmark ===")
print("Iterations:", iterations)

local start = os.clock()
local total = 0
local carry = 1
local step = 2
local limit = iterations * 2 + 1

while true do
    carry = carry + step
    if carry < limit then
        total = total + carry + step
    else
        total = total + carry
        break
    end
end

local elapsed = os.clock() - start
print(string.format("carry=%d total=%d %.3f seconds (%.2f M iters/sec)", carry, total, elapsed, iterations / elapsed / 1000000))