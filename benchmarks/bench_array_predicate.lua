-- Benchmark: counted-array predicate loops
-- Focused JIT sample for guarded numeric-for traces such as:
-- ADDI/MMBINI, GETTABLE, GETTABLE, LT, JMP, FORLOOP.

local iterations = 20000
local array_size = 512

print("=== Array Predicate Benchmark ===")
print("Iterations:", iterations)
print("Array size:", array_size)

local function build_sorted_array(n)
    local out = {}
    for i = 1, n do
        out[i] = i * 2
    end
    return out
end

local function build_unsorted_array(n)
    local out = build_sorted_array(n)
    out[n // 2] = out[n // 2 + 1] + 1
    return out
end

local function is_non_decreasing(a)
    for i = 2, #a do
        if a[i - 1] > a[i] then
            return false
        end
    end
    return true
end

local sorted = build_sorted_array(array_size)
local unsorted = build_unsorted_array(array_size)

if not is_non_decreasing(sorted) then
    error("sorted fixture is invalid")
end

if is_non_decreasing(unsorted) then
    error("unsorted fixture is invalid")
end

local start = os.clock()
local checks = 0

for iter = 1, iterations do
    if not is_non_decreasing(sorted) then
        error("predicate check failed")
    end
    checks = checks + 1
end

local elapsed = os.clock() - start
local total_values = iterations * array_size

print(string.format("Sorted predicate loop: %.3f seconds (%.2f M values/sec)", elapsed, total_values / elapsed / 1000000))
print("Checks:", checks)