local tonumber, tointeger = tonumber, math.tointeger
local type, getmetatable, rawget, error = type, getmetatable, rawget, error
local strsub = string.sub
local print = print

_ENV = nil

local function toint (x)
  print("  toint x =", x, "type =", type(x))
  local n = tonumber(x)
  print("  tonumber result =", n)
  if not n then
    return false
  end
  local ti = tointeger(n)
  print("  tointeger result =", ti)
  return ti
end

local function trymt (x, y, mtname)
  if type(y) ~= "string" then
    local mt = getmetatable(y)
    local mm = mt and rawget(mt, mtname)
    if mm then
      return mm(x, y)
    end
  end
  error("attempt to '" .. strsub(mtname, 3) .. "' a " .. type(x) .. " with a " .. type(y), 4)
end

local function checkargs (x, y, mtname)
  print("checkargs:", x, y)
  local xi = toint(x)
  print("xi =", xi)
  local yi = toint(y)
  print("yi =", yi)
  if xi and yi then
    return xi, yi
  else
    return trymt(x, y, mtname), nil
  end
end

local smt = getmetatable("")

smt.__band = function (x, y)
  print("__band:", x, y)
  local x2, y2 = checkargs(x, y, "__band")
  print("after checkargs:", x2, y2)
  return y2 and x2 & y2 or x2
end
