print("Testing with require:")
package.path = ".\\?.lua;" .. package.path
require "bwcoercion"

local smt = getmetatable("")
print("__band =", smt.__band)

-- 直接调用
print("Calling __band(\"5\", \"3\"):")
local r = smt.__band("5", "3")
print("r =", r)
