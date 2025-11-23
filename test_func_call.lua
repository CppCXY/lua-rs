-- Simple function call test
local function add(a, b)
    return a + b
end

local sum = 0
for i = 1, 1000000 do
    sum = add(i, 5)
end
print(sum)
