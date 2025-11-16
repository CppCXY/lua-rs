-- ipairs iteration test
local t = {}
for i = 1, 1000000 do
    t[i] = i
end

local start = os.clock()
local sum = 0
for i = 1, 100 do
    for idx, val in ipairs(t) do
        sum = sum + val
    end
end
local elapsed = os.clock() - start
print(string.format("ipairs iteration (100x1000000): %.3f seconds, sum=%d", elapsed, sum))
