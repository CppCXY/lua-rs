local n = tonumber(arg and arg[1]) or 150

local function a(i, j)
    local ij = i + j - 1
    return 1.0 / ((ij * (ij - 1) / 2) + i)
end

local function multiply_av(x, out)
    for i = 1, n do
        local sum = 0.0
        for j = 1, n do
            sum = sum + a(i, j) * x[j]
        end
        out[i] = sum
    end
end

local function multiply_atv(x, out)
    for i = 1, n do
        local sum = 0.0
        for j = 1, n do
            sum = sum + a(j, i) * x[j]
        end
        out[i] = sum
    end
end

local function multiply_at_av(x, tmp, out)
    multiply_av(x, tmp)
    multiply_atv(tmp, out)
end

local u = {}
local v = {}
local tmp = {}
for i = 1, n do
    u[i] = 1.0
    v[i] = 0.0
    tmp[i] = 0.0
end

for _ = 1, 10 do
    multiply_at_av(u, tmp, v)
    multiply_at_av(v, tmp, u)
end

local v_bv = 0.0
local vv = 0.0
for i = 1, n do
    v_bv = v_bv + u[i] * v[i]
    vv = vv + v[i] * v[i]
end

print(string.format("spectral_norm=%.9f n=%d", math.sqrt(v_bv / vv), n))