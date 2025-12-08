require "bwcoercion"

print("Testing error cases:")

local st, msg = pcall(function () return 4 & "a" end)
print("1. 4 & 'a' fails:", not st)
print("   msg contains 'band':", msg and string.find(msg, "band") ~= nil)

local st, msg = pcall(function () return ~"a" end)
print("2. ~'a' fails:", not st)
print("   msg contains 'bnot':", msg and string.find(msg, "bnot") ~= nil)

-- out of range number
local st, msg = pcall(function () return "0xffffffffffffffff.0" | 0 end)
print("3. '0xffffffffffffffff.0' | 0 fails:", not st)

-- embedded zeros
local st, msg = pcall(function () return "0xffffffffffffffff\0" | 0 end)
print("4. embedded zero fails:", not st)
