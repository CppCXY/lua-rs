local threads = {}
local t0 = os.clock()
for i = 1, 450000 do
  threads[i] = { a = i, b = i + 1, c = i + 2 }
end
local t1 = os.clock()
collectgarbage('collect')
local t2 = os.clock()
print(string.format('alloc 450k: %.3f  fullgc: %.3f', t1 - t0, t2 - t1))
