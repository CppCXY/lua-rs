-- json-serializer.lua
-- Benchmark: Serialization of complex nested tables (Arrays + Dictionaries)
-- Realism: High (Simulates API responses, Save files, Config generation)

local table_concat = table.concat
local table_insert = table.insert
local string_format = string.format
local type = type
local pairs = pairs
local ipairs = ipairs

-- Simple recursive JSON serializer
-- (Not full spec compliant, but functionally equivalent for benchmarks)
local function serialize(obj)
    local t = type(obj)
    if t == "number" then
        return tostring(obj)
    elseif t == "boolean" then
        return tostring(obj)
    elseif t == "string" then
        return string_format("%q", obj)
    elseif t == "table" then
        local parts = {}
        -- Detect if it's an array-like table (heuristic)
        local is_array = (#obj > 0)
        
        if is_array then
            table_insert(parts, "[")
            for i, v in ipairs(obj) do
                if i > 1 then table_insert(parts, ",") end
                table_insert(parts, serialize(v))
            end
            table_insert(parts, "]")
        else
            table_insert(parts, "{")
            local first = true
            -- Here we stress the Hash part of the table
            for k, v in pairs(obj) do
                if not first then table_insert(parts, ",") end
                table_insert(parts, string_format("%q:", tostring(k)))
                table_insert(parts, serialize(v))
                first = false
            end
            table_insert(parts, "}")
        end
        return table_concat(parts)
    else
        return "null"
    end
end

-- Generate synthetic data: A list of users with mixed data types
local function generate_data(count)
    local params = { "alpha", "beta", "gamma", "delta" }
    local data = {}
    for i = 1, count do
        local friends = {}
        for j = 1, 10 do
            table_insert(friends, {
                id = j,
                name = "Friend " .. j,
                active = (j % 2 == 0)
            })
        end
        
        table_insert(data, {
            id = "user_" .. i,
            index = i,
            guid = "a0eebc99-9c0b-4ef8-bb6d-" .. i,
            isActive = (i % 2 == 0),
            balance = i * 1234.56,
            age = 20 + (i % 50),
            eyeColor = "blue",
            name = {
                first = "John",
                last = "Doe " .. i
            },
            tags = { "lua", "benchmark", "json", "data" },
            friends = friends,
            meta = {
                login_count = i * 2,
                params = params -- Shared table reference
            }
        })
    end
    return data
end

-- Main
local N = tonumber(arg and arg[1]) or 100
local quiet = (arg and arg[2] == "quiet")

-- 1. Setup Phase (Allocate memory)
local complex_data = generate_data(1000) -- Fixed dataset size

-- 2. Kernel Phase (CPU intensive)
local res_len = 0
for i = 1, N do
    local json_str = serialize(complex_data)
    res_len = #json_str
end

if not quiet then
    io.write(string.format("Iterations: %d\n", N))
    io.write(string.format("Last JSON Size: %d bytes\n", res_len))
    io.write(string.format("Check: %s\n", complex_data[1].guid))
end
