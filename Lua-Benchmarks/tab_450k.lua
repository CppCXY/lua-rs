local threads = {}
local n = 0
local t0 = os.clock()
for i = 1, 450000 do
  n = n + 1
  threads[n] = { a = i, b = i + 1, c = i + 2 }
end
print(string.format('450k plain tables: %.3f', os.clock() - t0))
