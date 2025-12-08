local debug = debug

-- load bwcoercion
package.path = ".\\?.lua;" .. package.path
require "bwcoercion"

-- get string metatable
local smt = getmetatable("")
print("smt.__band =", smt.__band)

-- manually call __band
print("Calling __band directly:")
local result = smt.__band("10", "20")
print("result =", result)
