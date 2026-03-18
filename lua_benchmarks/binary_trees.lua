local depth = tonumber(arg and arg[1]) or 14

local function bottom_up_tree(item, tree_depth)
    if tree_depth <= 0 then
        return { item = item }
    end
    local next_depth = tree_depth - 1
    return {
        item = item,
        left = bottom_up_tree(item * 2 - 1, next_depth),
        right = bottom_up_tree(item * 2, next_depth),
    }
end

local function item_check(node)
    local left = node.left
    if left == nil then
        return node.item
    end
    return node.item + item_check(left) - item_check(node.right)
end

local min_depth = 4
local max_depth = math.max(min_depth + 2, depth)
local stretch_depth = max_depth + 1

local stretch_tree = bottom_up_tree(0, stretch_depth)
local stretch_check = item_check(stretch_tree)

local long_lived_tree = bottom_up_tree(0, max_depth)
local parts = {
    string.format("stretch_depth=%d check=%d", stretch_depth, stretch_check),
}

for current_depth = min_depth, max_depth, 2 do
    local iterations = 2 ^ (max_depth - current_depth + min_depth)
    local check = 0
    for i = 1, iterations do
        check = check + item_check(bottom_up_tree(i, current_depth))
        check = check + item_check(bottom_up_tree(-i, current_depth))
    end
    parts[#parts + 1] = string.format(
        "depth=%d iterations=%d check=%d",
        current_depth,
        iterations * 2,
        check
    )
end

parts[#parts + 1] = string.format(
    "long_lived_depth=%d check=%d",
    max_depth,
    item_check(long_lived_tree)
)

print(table.concat(parts, " | "))