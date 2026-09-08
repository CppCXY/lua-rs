#!/bin/env lua

-- Configuration ---------------------------------------------------------------

-- List of binaries that will be tested
local binaries = {
    { 'lua-5.1.5', 'lua5.1' },
    { 'lua-5.2.4', 'lua5.2' },
    { 'lua-5.3.6', 'lua5.3' },
    { 'lua-5.4.6', 'lua5.4' },
    { 'lua-5.5.0', 'lua5.5' },
    { 'luars-rs', '../target/release/lua' },
    --{ 'luajit-2.1.174-interp', 'luajit -joff' },
    { 'luajit-2.1.174', 'luajit' },
}

-- List of tests
local tests_root = './'
local tests = {
    { 'brainfuck', 'brainfuck.lua 10' },
    { 'mem-access', 'mem-access.lua 150' },
    { 'oop-dots', 'oop-dots.lua 100' },
    { 'c-call', 'c-call.lua 550' },
    { 'ray', 'ray.lua 768' },
    { 'coro', 'coro-scheduler.lua 450' },
    { 'json', 'json-serializer.lua 55' },

    -- Ackermann is a synthetic deep recursion test. It mostly benchmarks stack limits/call overhead, irrelevant for real-world logic.
    -- { 'ack', 'ack.lua 3 10' },

    -- Uses Y-combinator (functional style) to calculate factorial. Non-idiomatic for Lua (creates excessive closures instead of loops).
    -- { 'fixpoint-fact', 'fixpoint-fact.lua 3000' },

    -- Too simplistic/redundant. Array/table access is better benchmarked by 'heapsort' and 'fannkuch'.
    -- { 'sieve', 'sieve.lua 5000' },

    { 'heapsort', 'heapsort.lua 10 150000' },
    { 'mandelbrot', 'mandel.lua' },
    { 'juliaset', 'qt.lua' },
    { 'queen', 'queen.lua 12' },
    { 'binary', 'binary-trees.lua 14' },
    { 'n-body', 'n-body.lua 800000' },
    { 'fannkuch', 'fannkuch-redux.lua 10' },
    { 'fasta', 'fasta.lua 2500000' },
    { 'k-nucleotide', 'k-nucleotide.lua < fasta900000.txt' },
    { 'regex-dna', 'regex-dna.lua < fasta900000.txt' },
    { 'spectral-norm', 'spectral-norm.lua 1000' },
}

-- Command line arguments ------------------------------------------------------

local nruns = 3
local supress_errors = true
local basename = 'results'

local usage = [[
usage: lua ]] .. arg[0] .. [[ [options]
options:
    --nruns <n>      number of times that each test is executed (default = 3)
    --no-supress     don't supress error messages from tests
    --output <name>  name of the benchmark output (default = 'results')
    --help           show this message
]]

local function parse_args()
    local function parse_error(msg)
        print('Error: ' .. msg .. '\n' .. usage)
        os.exit(1)
    end
    local function get_next_arg(i)
        if i + 1 > #arg then
            parse_error(arg[i] .. ' requires a value')
        end
        local v = arg[i + 1]
        arg[i + 1] = nil
        return v
    end
    for i = 1, #arg do
        if not arg[i] then goto continue end
        if arg[i] == '--nruns' then
            nruns = tonumber(get_next_arg(i))
            if not nruns or nruns < 1 then
                parse_error('nruns should be a number greater than 1')
            end
        elseif arg[i] == '--no-supress' then
            supress_errors = false
        elseif arg[i] == '--output' then
            basename = get_next_arg(i)
        elseif arg[i] == '--help' then
            print(usage)
            os.exit()
        else
            parse_error('invalid argument: ' .. arg[i])
        end
        ::continue::
    end
end

-- Implementation --------------------------------------------------------------

-- Run the command a single time and returns the time elapsed
local function measure(cmd)
    -- Use 'sh' explicitly to access the shell built-in 'time'.
    -- '-p' creates POSIX portable output (real/user/sys).
    -- We redirect the command's stdout to /dev/null, but keep stderr (where time prints)
    -- and merge it to stdout (2>&1) so Lua can read it.
    local time_cmd = 'sh -c "time -p ' ..  cmd .. ' > /dev/null" 2>&1'

    local handle = io.popen(time_cmd)
    local result = handle:read("*a")
    handle:close()

    -- Parse the output looking for "real <number>"
    -- Output format is usually:
    -- real 0.55
    -- user 0.50
    -- sys 0.04
    local time_elapsed = string.match(result, "real%s+([%d%.]+)")
    time_elapsed = tonumber(time_elapsed)

    if not time_elapsed then
        error('Invalid output for "' .. cmd .. '":\n' .. (result or 'nil'))
    end

    return time_elapsed
end

-- Run the command $nruns and return the fastest time
local function benchmark(cmd)
    local min = 999
    io.write('running "' .. cmd .. '"... ')
    for _ = 1, nruns do
        local time = measure(cmd)
        min = math.min(min, time)
    end
    io.write('done\n')
    return min
end

-- Create a matrix with n rows
local function create_matrix(n)
    local m = {}
    for i = 1, n do
        m[i] = {}
    end
    return m
end

-- Measure the time for each binary and test
-- Return a matrix with the result (test x binary)
local function run_all()
    local results = create_matrix(#tests)
    for i, test in ipairs(tests) do
        local test_path = tests_root .. test[2]
        for j, binary in ipairs(binaries) do
            local cmd = binary[2] .. ' ' .. test_path
            local ok, msg = pcall(function()
                results[i][j] = benchmark(cmd)
            end)
            if not ok and not supress_errors then
                io.write('error:\n' .. msg .. '\n---\n')
            end
        end
    end
    return results
end

-- Helper to clone matrix and apply calculation
local function get_derived_results(src_results, calc_func)
    local new_results = create_matrix(#tests)
    for i, line in ipairs(src_results) do
        local base = line[1]
        for j = 1, #binaries do
            -- Apply the calculation function (e.g., base / v)
            new_results[i][j] = calc_func(line[j], base)
        end
    end
    return new_results
end

-- Saves the results matrix to a specific filename
local function create_data_file(filename, results)
    local data = 'test\t'
    for _, binary in ipairs(binaries) do
        data = data .. binary[1] .. '\t'
    end
    data = data .. '\n'
    for i, test in ipairs(tests) do
        data = data .. test[1] .. '\t'
        for j, _ in ipairs(binaries) do
            -- Format numbers to avoid excessive decimals in text files
            local val = results[i][j]

            -- Handle cases where benchmark failed (val is nil)
            if val == nil then
                val = "NaN"
            elseif type(val) == "number" then
                val = string.format("%.4f", val)
            end

            data = data .. val .. '\t'
        end
        data = data .. '\n'
    end
    local f = io.open(filename, 'w')
    if f then
        f:write(data)
        f:close()
        print('Saved: ' .. filename)
    else
        print('Error saving: ' .. filename)
    end
end

local function setup()
    os.execute('luajit ' .. tests_root .. 'fasta.lua 900000 > fasta900000.txt')
end

local function teardown()
    os.execute('rm fasta900000.txt')
end


local function main()
    parse_args()
    setup()

    -- 1. Run benchmarks (Raw data)
    local results = run_all()
    teardown()

    -- 2. Save Raw Data
    create_data_file(basename .. '.dat', results)

    -- 3. Calculate and Save Normalized Data (lower is better, relative to first binary)
    local results_norm = get_derived_results(results, function(v, base)
        if not v or v == 0 then return 0 end
        if not base then return v end
        return v / base
    end)
    create_data_file(basename .. '-norm.dat', results_norm)

    -- 4. Calculate and Save Speedup Data (higher is better, relative to first binary)
    local results_speed = get_derived_results(results, function(v, base)
        if not v or v == 0 then return 0 end
        if not base then return v end
        return base / v
    end)
    create_data_file(basename .. '-speed.dat', results_speed)

    print('Benchmark complete. Run ./plots.sh '.. basename ..' to generate graphs.')
end

main()

