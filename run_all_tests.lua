-- Run each test file from all.lua individually to identify which pass/fail
-- Run from lua_tests/testes/ directory

local tests = {
  "gc.lua",
  "calls.lua",
  "strings.lua",
  "literals.lua",
  "tpack.lua",
  "attrib.lua",
  "gengc.lua",
  "locals.lua",
  "constructs.lua",
  "code.lua",
  "big.lua",
  "cstack.lua",
  "nextvar.lua",
  "pm.lua",
  "utf8.lua",
  "api.lua",
  "events.lua",
  "vararg.lua",
  "closure.lua",
  "coroutine.lua",
  "goto.lua",
  "errors.lua",
  "math.lua",
  "sort.lua",
  "bitwise.lua",
  "verybig.lua",
  "files.lua",
}

-- Set globals expected by test files
_soft = true    -- skip long tests
_port = true    -- skip non-portable tests
_nomsg = true   -- skip messages about missing tests

local passed = 0
local failed = 0
local errors = {}

for _, test in ipairs(tests) do
  io.write("Testing " .. test .. " ... ")
  io.flush()
  local ok, err = pcall(dofile, test)
  if ok then
    print("PASS")
    passed = passed + 1
  else
    print("FAIL: " .. tostring(err))
    failed = failed + 1
    errors[#errors+1] = test .. ": " .. tostring(err)
  end
end

print("\n========== RESULTS ==========")
print(string.format("Passed: %d / %d", passed, #tests))
print(string.format("Failed: %d / %d", failed, #tests))
if #errors > 0 then
  print("\nFailed tests:")
  for _, e in ipairs(errors) do
    print("  " .. e)
  end
end
