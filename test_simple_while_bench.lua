-- Test simple while loop with constant comparison (should use LTI)
local start = os.clock()
local total = 0
for run = 1, 5 do
    local i = 0
    while i < 10000000 do
        i = i + 1
    end
    total = total + i
end
local elapsed = os.clock() - start
local iterations = 10000000 * 5
local speed = iterations / elapsed / 1000000
print("Simple while (constant comparison):")
print(string.format("Total iterations: %d", iterations))
print(string.format("Time: %.3f seconds", elapsed))
print(string.format("Speed: %.2f M ops/sec", speed))
print(string.format("Verify: total = %d", total))
