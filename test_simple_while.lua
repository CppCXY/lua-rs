local start = os.clock()
for run = 1, 5 do
    local i = 0
    while i < 10000000 do
        i = i + 1
    end
end
local elapsed = os.clock() - start
local iterations = 10000000 * 5
print(string.format(\"Simple while (i < 10M): %.3f seconds (%.2f M ops/sec)\", elapsed, iterations/elapsed/1000000))
