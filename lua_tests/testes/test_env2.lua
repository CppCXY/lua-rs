local tonumber = tonumber
local tointeger = math.tointeger
local print = print

_ENV = nil   -- 关键行！

local function toint (x)
  print("toint:", x)
  local n = tonumber(x)
  print("tonumber result:", n)
  if not n then
    return false
  end
  return tointeger(n)
end

print("Test:", toint("10"))
