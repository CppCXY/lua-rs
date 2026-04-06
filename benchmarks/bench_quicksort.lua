-- Benchmark: Quicksort on numeric arrays
-- This is meant to approximate a more realistic mixed workload:
-- recursive function calls, a hot partition loop, table reads/writes,
-- comparisons, branches, and numeric arithmetic all in the same routine.

local iterations = 300
local array_size = 256

print("=== Quicksort Benchmark ===")
print("Iterations:", iterations)
print("Array size:", array_size)

local seed = 0x12345678

local function next_random()
    seed = (1103515245 * seed + 12345) % 2147483648
    return seed
end

local function build_source_array(n)
    local out = {}
    for i = 1, n do
        out[i] = next_random() % 100000
    end
    return out
end

local source = build_source_array(array_size)

local function copy_array(src)
    local dst = {}
    for i = 1, #src do
        dst[i] = src[i]
    end
    return dst
end

local function insertion_sort(a, left, right)
    for i = left + 1, right do
        local value = a[i]
        local j = i - 1
        while j >= left and a[j] > value do
            a[j + 1] = a[j]
            j = j - 1
        end
        a[j + 1] = value
    end
end

local function partition(a, left, right)
    local mid = (left + right) // 2
    local pivot = a[mid]
    local i = left
    local j = right

    while i <= j do
        while a[i] < pivot do
            i = i + 1
        end

        while a[j] > pivot do
            j = j - 1
        end

        if i <= j then
            local tmp = a[i]
            a[i] = a[j]
            a[j] = tmp
            i = i + 1
            j = j - 1
        end
    end

    return i, j
end

local function quicksort(a, left, right)
    while left < right do
        if right - left <= 12 then
            insertion_sort(a, left, right)
            return
        end

        local i, j = partition(a, left, right)

        if j - left < right - i then
            if left < j then
                quicksort(a, left, j)
            end
            left = i
        else
            if i < right then
                quicksort(a, i, right)
            end
            right = j
        end
    end
end

local function checksum(a)
    local acc = 0
    for i = 1, #a do
        acc = (acc + a[i] * i) % 2147483647
    end
    return acc
end

local function is_sorted(a)
    for i = 2, #a do
        if a[i - 1] > a[i] then
            return false
        end
    end
    return true
end

local start = os.clock()
local sum = 0

for iter = 1, iterations do
    local work = copy_array(source)
    quicksort(work, 1, #work)

    if not is_sorted(work) then
        error("quicksort produced unsorted output")
    end

    sum = (sum + checksum(work)) % 2147483647
end

local elapsed = os.clock() - start
local total_values = iterations * array_size

print(string.format("Quicksort: %.3f seconds (%.2f K values/sec)", elapsed, total_values / elapsed / 1000))
print("Checksum:", sum)