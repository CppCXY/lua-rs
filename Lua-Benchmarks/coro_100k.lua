local co_create = coroutine.create
local t0 = os.clock()
local function task() return 1 end
local n = 0
for i = 1, 100000 do
  local co = co_create(task)
  n = n + 1
end
print(string.format('create 100k: %.3f', os.clock() - t0))
print('gcinfo:', collectgarbage('count'))
