-- 测试原始的 toint 函数
local function toint_orig(x)
  x = tonumber(x)   -- 重新赋值
  if not x then
    return false
  end
  return math.tointeger(x)
end

-- 测试我的版本
local function toint_new(x)
  local n = tonumber(x)   -- 新变量
  if not n then
    return false
  end
  return math.tointeger(n)
end

print("Testing toint_orig:")
print("  toint_orig('5') =", toint_orig("5"))
print("  toint_orig('abc') =", toint_orig("abc"))

print("Testing toint_new:")
print("  toint_new('5') =", toint_new("5"))
print("  toint_new('abc') =", toint_new("abc"))
