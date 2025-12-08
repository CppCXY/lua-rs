package.loaded["bwcoercion"] = nil
require "bwcoercion"

local numbits = string.packsize("j") * 8
print("numbits =", numbits)

print("Testing ~0 == -1:", ~0 == -1)
print("Testing (1 << (numbits - 1)) == math.mininteger:", (1 << (numbits - 1)) == math.mininteger)

local a, b, c, d
a = 0xFFFFFFFFFFFFFFFF
print("a = 0xFFFFFFFFFFFFFFFF:", a)
print("a == -1:", a == -1)
print("a & -1 == a:", a & -1 == a)
print("a & 35 == 35:", a & 35 == 35)

a = 0xF0F0F0F0F0F0F0F0
print("a = 0xF0F0F0F0F0F0F0F0:", a)
print("a | -1 == -1:", a | -1 == -1)
print("a ~ a == 0:", a ~ a == 0)
print("a ~ 0 == a:", a ~ 0 == a)
print("a ~ ~a == -1:", a ~ ~a == -1)
print("a >> 4 =", a >> 4)
print("~a =", ~a)
print("a >> 4 == ~a:", a >> 4 == ~a)
