require "bwcoercion"

local a, b, c, d = 0xF0, 0xCC, 0xAA, 0xFD
print("Testing a | b ~ c & d == 0xF4:", a | b ~ c & d == 0xF4)

a = 0xF0.0; b = 0xCC.0; c = "0xAA.0"; d = "0xFD.0"
print("a, b, c, d =", a, b, c, d)
print("a | b ~ c & d =", a | b ~ c & d)
print("Testing a | b ~ c & d == 0xF4:", a | b ~ c & d == 0xF4)

a = 0xF0000000; b = 0xCC000000
c = 0xAA000000; d = 0xFD000000
print("a | b ~ c & d =", a | b ~ c & d)
print("Testing a | b ~ c & d == 0xF4000000:", a | b ~ c & d == 0xF4000000)
print("~~a =", ~~a)
print("~a =", ~a)
print("-1 ~ a =", -1 ~ a)
print("~~a == a:", ~~a == a)
print("~a == -1 ~ a:", ~a == -1 ~ a)
print("-d =", -d)
print("~d + 1 =", ~d + 1)
print("-d == ~d + 1:", -d == ~d + 1)
