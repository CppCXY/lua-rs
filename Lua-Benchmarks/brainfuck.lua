-- brainfuck.lua
-- Benchmark: Bytecode interpretation simulation
-- Realism: Virtual Machines inside Lua, complex state formatting

-- Using "Square Numbers" algorithm. 
local code = [=[
++++[>+++++<-]>[<+++++>-]+<+[
    >[>+>+<<-]++>>[<<+>>-]>>>[-]++>[-]+
    >>>+[[-]++++++>>>]<<<[[<++++++++<++>>-]+<.<[>----<-]<]
    <<[>>>>>[>>>[-]+++++++++<[>-<-]+++++++++>[-[<->-]+[<<<]]<[>+<-]>]<<-]<<-
]
]=]

-- Changed: Removed 'or 1'. Default to 100 if no arg provided.
-- User can now tune this: 'lua brainfuck.lua 1000'
local N = tonumber(arg and arg[1]) or 100

-- Replaced 'run' with the logic from the GitHub implementation you provided
-- Adapted to be silent (no io.write) and non-interactive (no io.read) for benchmarking
local function run(str)
    local pointer = 1
    local stack = {}
    local loops = {}
    local skipping = 0
    local pos = 0
    local str_len = #str

    repeat
        pos = pos + 1
        -- Optimization: using cache for str length doesn't help much with string.sub, but logic follows provided snippet
        local symbol = string.sub(str, pos, pos)
        
        stack[pointer] = stack[pointer] or 0

        if skipping == 0 then
            if symbol == '>' then
                pointer = pointer + 1
            elseif symbol == '<' then
                if pointer > 1 then pointer = pointer - 1 end
            elseif symbol == '+' then
                stack[pointer] = (stack[pointer] + 1) % 256 -- Added modulo 256 for safety logic
            elseif symbol == '-' then
                stack[pointer] = (stack[pointer] - 1) % 256
            elseif symbol == '.' then
                -- Silent mode for benchmark: do nothing
            elseif symbol == ',' then
                -- Input mode: Simulate EOF (0) to avoid blocking
                stack[pointer] = 0
            end
        end

        if symbol == '[' then
            table.insert(loops, pos)
            if (stack[pointer] == 0) and (skipping == 0) then
                skipping = #loops
            end
        elseif symbol == ']' then
            if stack[pointer] ~= 0 then
                pos = loops[#loops]
            else
                if skipping == #loops then skipping = 0 end
                table.remove(loops)
            end
        end
    until pos >= str_len
end

-- Execution
-- Changed: Straight loop, no hidden multipliers. 
-- If you put N=100, it runs 100 times.
for _ = 1, N do
    run(code)
end

io.write(string.format("Brainfuck VM finished (Ran %d times)\n", N))

