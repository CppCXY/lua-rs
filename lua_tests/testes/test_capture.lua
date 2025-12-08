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
