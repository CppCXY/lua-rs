-- Test while loop with SMALL constant (uses LTI instruction)
local start = os.clock()
local total = 0
for outer = 1, 1000000 do
    local i = 0
    while i < 100 do  -- Small constant, will use LTI
        i = i + 1
    end
    total = total + i
end
local elapsed = os.clock() - start
local iterations = 100 * 1000000
local speed = iterations / elapsed / 1000000
print("While with small constant (uses LTI):")
print(string.format("Total iterations: %d", iterations))
print(string.format("Time: %.3f seconds", elapsed))
print(string.format("Speed: %.2f M ops/sec", speed))
print(string.format("Verify: total = %d", total))
