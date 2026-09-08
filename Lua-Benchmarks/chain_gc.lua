-- tables referencing each other (forces deep propagate)
local root = {}
local cur = root
for i = 1, 200000 do
  cur.next = {}
  cur = cur.next
end
local t0 = os.clock()
collectgarbage('collect')
print(string.format('chain 200k + full gc: %.3f', os.clock() - t0))
