-- 检查 require 后 bwcoercion 内部变量
package.path = ".\\?.lua;" .. package.path

-- 在 require 之前，修改 bwcoercion 使其暴露 toint
-- 我们用 dofile 加载一个修改版
local code = io.open("bwcoercion.lua"):read("*a")
print("Original code loaded, length:", #code)

-- 测试 tonumber 和 tointeger
print("Testing tonumber('10'):", tonumber("10"))
print("Testing math.tointeger(10.0):", math.tointeger(10.0))

-- 关键：测试在 require 之后
require "bwcoercion"
print("After require bwcoercion")

-- 手动测试 checkargs 类似的逻辑
local function my_toint(x)
  local n = tonumber(x)
  if not n then return false end
  return math.tointeger(n)
end

print("my_toint('10'):", my_toint("10"))
print("my_toint('20'):", my_toint("20"))

-- 现在测试字符串 band
print("Testing '5' & '3':")
print(("5" & "3"))
