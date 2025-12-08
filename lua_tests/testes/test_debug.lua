-- 复制 bwcoercion 的代码，添加调试
local tonumber_orig = tonumber
local tointeger_orig = math.tointeger
local print = print

local tonumber, tointeger = tonumber, math.tointeger
print("tonumber captured:", tonumber)
print("tointeger captured:", tointeger)
print("are they same?", tonumber == tonumber_orig, tointeger == tointeger_orig)

local type, getmetatable, rawget, error = type, getmetatable, rawget, error
local strsub = string.sub

_ENV = nil

-- 测试 tonumber 在 _ENV = nil 后是否仍然有效
local function test_tonumber()
  print("Testing tonumber after _ENV = nil")
  print("tonumber =", tonumber)
  print("tonumber('10') =", tonumber("10"))
end

test_tonumber()
