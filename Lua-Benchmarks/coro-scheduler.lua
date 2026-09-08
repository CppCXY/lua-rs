-- coro-scheduler.lua
-- Benchmark: Coroutine context switching and scheduling overhead
-- Realism: High (Simulates Game AI logic, Async Web Servers, Event Loops)

local co_create = coroutine.create
local co_resume = coroutine.resume
local co_yield  = coroutine.yield

-- A worker task that performs some calculations, 
-- yields control back to the scheduler, and repeats.
local function task_logic(id, steps)
    local count = 0
    for i = 1, steps do
        -- Simulate some work (simple arithmetic)
        count = count + id
        -- Yield control back to main thread, passing status
        co_yield(count)
    end
    return count
end

-- The Scheduler
-- Manages a queue of coroutines and runs them round-robin until all are finished.
local function run_scheduler(num_threads, steps_per_thread)
    local threads = {}
    local n_threads = 0
    
    -- 1. Mass creation (Spawn phase)
    for i = 1, num_threads do
        -- We pass the function and its arguments
        local co = co_create(task_logic)
        n_threads = n_threads + 1
        threads[n_threads] = { c = co, id = i, steps = steps_per_thread }
    end

    local final_sum = 0
    local alive_count = n_threads

    -- 2. Execution Loop (Context switching hell)
    while alive_count > 0 do
        local new_alive_count = 0
        local next_batch = {} -- Double buffering optimization to avoid table.remove

        for i = 1, alive_count do
            local task = threads[i]
            local co = task.c
            
            -- Resume the coroutine
            -- Pass arguments only on first call (Lua handles this logic, 
            -- but here we rely on the closure or first resume args if needed)
            local status, result
            
            if coroutine.status(co) == 'suspended' then
                if task.started then
                    status, result = co_resume(co)
                else
                    status, result = co_resume(co, task.id, task.steps)
                    task.started = true
                end
                
                if status then
                    if coroutine.status(co) == "dead" then
                        -- Task finished
                        final_sum = final_sum + (result or 0)
                    else
                        -- Task yielded, keep it for next frame
                        new_alive_count = new_alive_count + 1
                        next_batch[new_alive_count] = task
                    end
                else
                    error("Coroutine error: " .. tostring(result))
                end
            end
        end
        
        -- Swap buffers
        threads = next_batch
        alive_count = new_alive_count
    end
    
    return final_sum
end

-- Parameters
-- N is input from benchmark runner. 
-- We scale it to have reasonable runtimes.
local N = tonumber(arg and arg[1]) or 100
local num_coroutines = N * 100    -- e.g., 500 * 100 = 50,000 coroutines
local steps = 100                 -- Each coroutine yields 100 times

-- For the benchmark suite, we usually run larger N, let's adjust:
-- If input is ~20 (like binary trees), we need adjustment.
-- If input is ~5000 (like loops), we need adjustment.
-- Let's assume input is around 50-200.

if N < 50 then num_coroutines = N * 1000 end -- Scale up for small N inputs

-- Main Execution
-- io.write(string.format("Scheduler: %d threads, %d steps each\n", num_coroutines, steps))

local result = run_scheduler(num_coroutines, steps)

io.write(string.format("Final sum: %d\n", result))

