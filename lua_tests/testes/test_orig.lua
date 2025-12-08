print("Clearing any cached modules")
package.loaded["bwcoercion"] = nil
package.loaded["test_bw_mock"] = nil

print("Loading bwcoercion")
require "bwcoercion"

print("Testing string band")
local a, b = "5", "3"
print("a =", a, "b =", b)
local r = a & b
print("r =", r)
