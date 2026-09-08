-- baseline: pure VM dispatch, loop of arithmetic
local t0 = os.clock()
local s = 0
for i = 1, 500000000 do s = s + i end
print(string.format('arith loop 500M: %.3f', os.clock() - t0))
