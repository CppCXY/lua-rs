-- measure empty C function call overhead (baseline for resume's C-layer)
local function nop() end
local t0 = os.clock()
for i = 1, 10000000 do nop() end
print(string.format('10M Lua->C empty call: %.3f', os.clock() - t0))
