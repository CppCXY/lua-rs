local tonumber, tointeger = tonumber, math.tointeger
local type, getmetatable, rawget, error = type, getmetatable, rawget, error
local strsub = string.sub
local print = print

_ENV = nil

print("In module, tonumber =", tonumber)
print("In module, tointeger =", tointeger)
print("tonumber(\"10\") =", tonumber("10"))

local function toint (x)
  print("toint x =", x)
  local tn = tonumber(x)
  print("tonumber result =", tn)
  if not tn then
    return false
  end
  local ti = tointeger(tn)
  print("tointeger result =", ti)
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
  error("attempt to '" .. strsub(mtname, 3) ..
        "' a " .. type(x) .. " with a " .. type(y), 4)
end

local function checkargs (x, y, mtname)
  print("checkargs:", x, y, mtname)
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
  local x, y = checkargs(x, y, "__band")
  return y and x & y or x
end

return function()
  print("Testing band:")
  print("5" & "3")
end
