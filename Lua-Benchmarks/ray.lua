-- ray.lua
-- A simple ray-tracer benchmark.
-- Tests: Floating point math, massive creation of short-lived tables (Vectors), function calls.
-- Realism: High for Game Development (simulates vector math libraries in Lua).

local sqrt, max, min = math.sqrt, math.max, math.min
local floor = math.floor

-- ----------------------------------------------------------------------------
-- Vector3 Library (Simulating typical GameDev usage)
-- ----------------------------------------------------------------------------
local function vec3(x, y, z)
    return { x = x, y = y, z = z }
end

local function v_add(a, b) return { x = a.x + b.x, y = a.y + b.y, z = a.z + b.z } end
local function v_sub(a, b) return { x = a.x - b.x, y = a.y - b.y, z = a.z - b.z } end
local function v_mul_s(a, s) return { x = a.x * s, y = a.y * s, z = a.z * s } end
local function v_dot(a, b) return a.x * b.x + a.y * b.y + a.z * b.z end

local function v_unit(a)
    local len = sqrt(a.x*a.x + a.y*a.y + a.z*a.z)
    return { x = a.x / len, y = a.y / len, z = a.z / len }
end

-- ----------------------------------------------------------------------------
-- Scene
-- ----------------------------------------------------------------------------
local function hit_sphere(center, radius, r_origin, r_dir)
    local oc = v_sub(r_origin, center)
    local a = v_dot(r_dir, r_dir)
    local b = 2.0 * v_dot(oc, r_dir)
    local c = v_dot(oc, oc) - radius * radius
    local discriminant = b*b - 4*a*c
    
    if discriminant < 0 then
        return -1.0
    else
        return (-b - sqrt(discriminant)) / (2.0 * a)
    end
end

local sphere_pos = vec3(0, 0, -1)

local function color(r_origin, r_dir)
    local t = hit_sphere(sphere_pos, 0.5, r_origin, r_dir)
    if t > 0.0 then
        local N = v_unit(v_sub(v_add(r_origin, v_mul_s(r_dir, t)), sphere_pos))
        return vec3(0.5 * (N.x + 1), 0.5 * (N.y + 1), 0.5 * (N.z + 1))
    end
    
    local unit_dir = v_unit(r_dir)
    t = 0.5 * (unit_dir.y + 1.0)
    
    -- Linear interpolation (Lerp)
    local white = vec3(1.0, 1.0, 1.0)
    local blue = vec3(0.5, 0.7, 1.0)
    
    -- (1.0 - t) * white + t * blue
    local p1 = v_mul_s(white, 1.0 - t)
    local p2 = v_mul_s(blue, t)
    return v_add(p1, p2)
end

-- ----------------------------------------------------------------------------
-- Main Loop
-- ----------------------------------------------------------------------------
local N = tonumber(arg and arg[1]) or 100 -- Default size (NxN pixels)
local quiet = (arg and arg[2] == "quiet")

local lower_left = vec3(-2.0, -1.0, -1.0)
local horizontal = vec3(4.0, 0.0, 0.0)
local vertical   = vec3(0.0, 2.0, 0.0)
local origin     = vec3(0.0, 0.0, 0.0)

if not quiet then
    io.write(string.format("P3\n%d %d\n255\n", N, N))
end

local checksum = 0

-- Render from top to bottom
for j = N-1, 0, -1 do
    for i = 0, N-1 do
        local u = i / N
        local v = j / N
        
        -- Ray direction = lower_left + u*horizontal + v*vertical - origin
        local h_u = v_mul_s(horizontal, u)
        local v_v = v_mul_s(vertical, v)
        local dir = v_add(lower_left, v_add(h_u, v_v))
        dir = v_sub(dir, origin)
        
        local col = color(origin, dir)
        
        local ir = floor(255.99 * col.x)
        local ig = floor(255.99 * col.y)
        local ib = floor(255.99 * col.z)
        
        -- Simple checksum to verify correctness without printing tons of text
        checksum = checksum + ir + ig + ib
    end
end

if not quiet then
    io.write(string.format("# Checksum: %d\n", floor(checksum)))
end
