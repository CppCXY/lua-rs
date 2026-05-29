-- Benchmark: GC behavior under allocation pressure

local pressure_iters = 100000
local hash_iters = 100000
local step_budget = 64
local step_arg = 0

local function print_gc_params()
    local names = { "pause", "stepmul", "stepsize", "minormul", "minormajor", "majorminor" }
    local parts = {}
    for _, name in ipairs(names) do
        local ok, value = pcall(collectgarbage, "param", name)
        if ok then
            parts[#parts + 1] = name .. "=" .. tostring(value)
        end
    end
    if #parts > 0 then
        print("GC params: " .. table.concat(parts, ", "))
    end
end

local function gc_count_kb()
    return collectgarbage("count")
end

local function build_table_garbage(count)
    for i = 1, count do
        local t = {
            i,
            i + 1,
            i + 2,
            ["key" .. i] = i,
            ["next" .. i] = i + 1,
        }
        local x = t[1] + t["key" .. i]
    end
end

local function run_hash_insert(count)
    local ht = {}
    for i = 1, count do
        ht["key" .. i] = i
    end
    return ht
end

local function run_full_collect(label)
    local before = gc_count_kb()
    local start = os.clock()
    collectgarbage("collect")
    local elapsed = os.clock() - start
    local after = gc_count_kb()
    print(string.format("%s: %.3f seconds (%.1f KB -> %.1f KB, reclaimed %.1f KB)", label, elapsed, before, after, before - after))
end

local function run_step_drain(label)
    local before = gc_count_kb()
    local start = os.clock()
    local completed = false
    local steps = 0
    while steps < step_budget do
        steps = steps + 1
        if collectgarbage("step", step_arg) then
            completed = true
            break
        end
    end
    local elapsed = os.clock() - start
    local after = gc_count_kb()
    print(string.format("%s: %.3f seconds (%d steps, completed=%s, %.1f KB -> %.1f KB, reclaimed %.1f KB)", label, elapsed, steps, tostring(completed), before, after, before - after))
end

print("=== GC Pressure Benchmark ===")
print(string.format("pressure_iters=%d, hash_iters=%d, step_budget=%d", pressure_iters, hash_iters, step_budget))
print_gc_params()

collectgarbage("collect")
collectgarbage("stop")
local base_kb = gc_count_kb()
local start = os.clock()
build_table_garbage(pressure_iters)
local build_elapsed = os.clock() - start
local pressure_kb = gc_count_kb()
print(string.format("Build pressure with GC stopped: %.3f seconds (%.1f KB -> %.1f KB, grew %.1f KB)", build_elapsed, base_kb, pressure_kb, pressure_kb - base_kb))
collectgarbage("restart")

run_step_drain("Drain pressure with collectgarbage('step')")
run_full_collect("Full collect after stepped drain")

collectgarbage("collect")
collectgarbage("stop")
build_table_garbage(pressure_iters)
collectgarbage("restart")
local before_hash_pressure = gc_count_kb()
start = os.clock()
local ht = run_hash_insert(hash_iters)
local hash_pressure_elapsed = os.clock() - start
local after_hash_pressure = gc_count_kb()
print(string.format("Hash insert under GC pressure: %.3f seconds (%.1f KB -> %.1f KB)", hash_pressure_elapsed, before_hash_pressure, after_hash_pressure))

run_full_collect("Full collect after pressure-path hash insert")

collectgarbage("collect")
local before_hash_collect = gc_count_kb()
start = os.clock()
ht = run_hash_insert(hash_iters)
local hash_collect_elapsed = os.clock() - start
local after_hash_collect = gc_count_kb()
print(string.format("Hash insert after explicit collect: %.3f seconds (%.1f KB -> %.1f KB)", hash_collect_elapsed, before_hash_collect, after_hash_collect))

collectgarbage("collect")
collectgarbage("stop")
local before_hash_stopped = gc_count_kb()
start = os.clock()
ht = run_hash_insert(hash_iters)
local hash_stopped_elapsed = os.clock() - start
local after_hash_stopped = gc_count_kb()
collectgarbage("restart")
print(string.format("Hash insert with GC stopped: %.3f seconds (%.1f KB -> %.1f KB)", hash_stopped_elapsed, before_hash_stopped, after_hash_stopped))
