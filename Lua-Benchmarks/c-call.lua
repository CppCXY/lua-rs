-- c-call.lua
-- Benchmark: Overhead of calling Host C functions
-- Realism: High for Game Engines (calling API functions per frame)

local N = tonumber(arg and arg[1]) or 100
local iterations = N * 10000

local sub = string.sub
local s = "helloworld"
local count = 0

-- string.sub is a C-function in standard Lua. 
-- In LuaJIT it might be fast, but it still measures the boundary cost.
for i = 1, iterations do
    -- Calling a C function repeatedly
    sub(s, 1, 1)
    sub(s, 2, 2)
    sub(s, 3, 3)
    sub(s, 4, 4)
    sub(s, 5, 5)
end

io.write("C-Calls done: ", iterations * 5, "\n")
