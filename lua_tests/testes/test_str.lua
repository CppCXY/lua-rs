require "bwcoercion"

print("Testing string to int coercion:")
print('1. "0xffffffffffffffff" | 0 == -1:', "0xffffffffffffffff" | 0 == -1)
print('2. "0xfffffffffffffffe" & "-1" == -2:', "0xfffffffffffffffe" & "-1" == -2)
print('3. (" \\t-0xfffffffffffffffe\\n\\t" & "-1") == 2:')
local r3 = " \t-0xfffffffffffffffe\n\t" & "-1"
print("   result:", r3, "expected: 2")
print('4. ("   \\n  -45  \\t " >> "  -2  ") == -45 * 4:', ("   \n  -45  \t " >> "  -2  ") == -45 * 4)
print('5. "1234.0" << "5.0" == 1234 * 32:', "1234.0" << "5.0" == 1234 * 32)
print('6. "0xffff.0" ~ "0xAAAA" == 0x5555:', "0xffff.0" ~ "0xAAAA" == 0x5555)
print('7. ~"0x0.000p4" == -1:', ~"0x0.000p4" == -1)
print('8. ("7" .. 3) << 1 == 146:', ("7" .. 3) << 1 == 146)
print('9. 0xffffffff >> (1 .. "9") == 0x1fff:', 0xffffffff >> (1 .. "9") == 0x1fff)
print('10. 10 | (1 .. "9") == 27:', 10 | (1 .. "9") == 27)
