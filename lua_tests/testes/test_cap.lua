-- 创建一个简单的测试模块
local f = io.open("test_capture.lua", "w")
f:write([[
local captured_tonumber = tonumber
local captured_print = print
local mystr = "hello"
captured_print("In module, captured_tonumber =", captured_tonumber)
captured_print("captured_tonumber('123') =", captured_tonumber("123"))

_ENV = nil  -- 关键

-- 返回一个使用捕获变量的函数
return function(x)
  captured_print("test func called with:", x)
  local n = captured_tonumber(x)
  captured_print("tonumber result:", n)
  return n
end
]])
f:close()
print("Created test_capture.lua")

package.path = ".\\?.lua;" .. package.path
local test_func = require("test_capture")
print("After require, test_func =", test_func)
print("Calling test_func('456'):")
local result = test_func("456")
print("result =", result)
