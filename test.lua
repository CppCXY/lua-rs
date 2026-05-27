--[[
binarytrees.lua — GC pressure via aggressive table allocation/discard.

Adapted from the Computer Language Benchmarks Game "binary-trees" Lua entry.
Builds, traverses, and discards binary trees at varying depths so the GC
sees a continuous stream of allocation + reclamation.

Measures: allocation throughput, minor-collection cost, table layout cost.

Workload: depth 14 (CLBG default is 21; we use 14 to keep the benchmark
under ~5 s in reference C Lua).

Deterministic: a checksum sums every leaf value seen during traversal.
]]
-- collectgarbage("stop")  -- avoid GC during setup
local function make_tree(depth)
    if depth == 0 then return { value = 1 } end
    return {
        left = make_tree(depth - 1),
        right = make_tree(depth - 1),
        value = depth,
    }
end

local function check_tree(node)
    if not node.left then return node.value end
    return node.value + check_tree(node.left) + check_tree(node.right)
end

local max_depth = 14
local min_depth = 4

local long_lived = make_tree(max_depth)
local checksum = 0

for d = min_depth, max_depth, 2 do
    local iterations = 1 << (max_depth - d + min_depth)
    local subtotal = 0
    for _ = 1, iterations do
        subtotal = subtotal + check_tree(make_tree(d))
    end
    checksum = checksum + subtotal
end

checksum = checksum + check_tree(long_lived)

assert(checksum == 4622192, "binarytrees checksum mismatch: got " .. checksum)
io.write("binarytrees.lua OK: checksum=", checksum, "\n")