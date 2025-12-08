package.loaded["bwcoercion"] = nil
require "bwcoercion"
print("Testing basic operations:")
print("'5' & '3' =", "5" & "3")
print("0xAA & 0xFF =", 0xAA & 0xFF)

-- From bitwise.lua line 27
local a = 0xF0.0
local b = 0xCC.0
local c = "0xAA.0"
local d = "0xFD.0"
print("a =", a, type(a))
print("b =", b, type(b))
print("c =", c, type(c))
print("d =", d, type(d))
print("Testing a | b ~ c & d:")
print(a | b ~ c & d)
