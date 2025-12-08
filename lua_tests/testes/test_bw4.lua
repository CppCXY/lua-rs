require "bwcoercion"

local numbits = 64

a = 0xF0000000 << 32
print("a = 0xF0000000 << 32:", a)
b = 0xCC000000 << 32
c = 0xAA000000 << 32
d = 0xFD000000 << 32
print("a | b ~ c & d =", a | b ~ c & d)
print("0xF4000000 << 32 =", 0xF4000000 << 32)
print("a | b ~ c & d == 0xF4000000 << 32:", a | b ~ c & d == 0xF4000000 << 32)

-- constant folding tests
print("Testing constant folding:")
print("-1 >> math.maxinteger =", -1 >> math.maxinteger)
print("-1 >> math.mininteger =", -1 >> math.mininteger)
print("-1 << math.maxinteger =", -1 << math.maxinteger)
print("-1 << math.mininteger =", -1 << math.mininteger)

print("-1 >> 1 =", -1 >> 1)
print("(1 << (numbits - 1)) - 1 =", (1 << (numbits - 1)) - 1)
print("-1 >> 1 == (1 << (numbits - 1)) - 1:", -1 >> 1 == (1 << (numbits - 1)) - 1)
print("1 << 31 =", 1 << 31)
print("1 << 31 == 0x80000000:", 1 << 31 == 0x80000000)
