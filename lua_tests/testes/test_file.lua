local f = io.open("test_output.txt", "w")
f:write("Starting test\n")

package.loaded["bwcoercion"] = nil
f:write("Loading bwcoercion\n")
local ok, err = pcall(function() require "bwcoercion" end)
f:write("require result: " .. tostring(ok) .. " " .. tostring(err) .. "\n")

if ok then
  f:write("Testing string band\n")
  local ok2, result = pcall(function() return "5" & "3" end)
  f:write("band result: " .. tostring(ok2) .. " " .. tostring(result) .. "\n")
end

f:close()
print("Done")
