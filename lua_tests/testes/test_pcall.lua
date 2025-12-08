print("Clearing any cached modules")
package.loaded["bwcoercion"] = nil

print("Loading bwcoercion")
local ok, err = pcall(function() require "bwcoercion" end)
print("require result:", ok, err)

if ok then
  print("Testing string band")
  local ok2, err2 = pcall(function() return "5" & "3" end)
  print("band result:", ok2, err2)
end
