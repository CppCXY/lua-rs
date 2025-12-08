require "bwcoercion"
local numbits = 64

print("Testing bitwise.lua sections:")

print("1. ~0 == -1:", ~0 == -1)
print("2. (1 << (numbits - 1)) == math.mininteger:", (1 << (numbits - 1)) == math.mininteger)

local a = 0xFFFFFFFFFFFFFFFF
print("3. a == -1:", a == -1)
print("4. a & -1 == a:", a & -1 == a)
print("5. a & 35 == 35:", a & 35 == 35)

a = 0xF0F0F0F0F0F0F0F0
print("6. a | -1 == -1:", a | -1 == -1)
print("7. a >> 4 == ~a:", a >> 4 == ~a)

local a, b, c, d
a = 0xF0; b = 0xCC; c = 0xAA; d = 0xFD
print("8. a | b ~ c & d == 0xF4:", a | b ~ c & d == 0xF4)

a = 0xF0.0; b = 0xCC.0; c = "0xAA.0"; d = "0xFD.0"
print("9. string coercion:", a | b ~ c & d == 0xF4)

-- shift tests
print("10. -1 >> 1 == (1 << (numbits - 1)) - 1:", -1 >> 1 == (1 << (numbits - 1)) - 1)
print("11. -1 >> (numbits - 1) == 1:", -1 >> (numbits - 1) == 1)
print("12. -1 >> numbits == 0:", -1 >> numbits == 0)
print("13. -1 >> -numbits == 0:", -1 >> -numbits == 0)
print("14. -1 << numbits == 0:", -1 << numbits == 0)
print("15. -1 << -numbits == 0:", -1 << -numbits == 0)

print("16. 1 >> math.mininteger == 0:", 1 >> math.mininteger == 0)
print("17. 1 >> math.maxinteger == 0:", 1 >> math.maxinteger == 0)
print("18. 1 << math.mininteger == 0:", 1 << math.mininteger == 0)
print("19. 1 << math.maxinteger == 0:", 1 << math.maxinteger == 0)

print("20. (2^30 - 1) << 2^30 == 0:", (2^30 - 1) << 2^30 == 0)
print("21. (2^30 - 1) >> 2^30 == 0:", (2^30 - 1) >> 2^30 == 0)

print("22. 1 >> -3 == 1 << 3:", 1 >> -3 == 1 << 3)
print("23. 1000 >> 5 == 1000 << -5:", 1000 >> 5 == 1000 << -5)
