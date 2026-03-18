local n = tonumber(arg and arg[1]) or 2000000
local sum1 = 0.0
local sum2 = 0.0
local sum3 = 0.0
local sum4 = 0.0
local sum5 = 0.0
local sum6 = 0.0
local sum7 = 0.0
local alt = 1.0

for i = 1, n do
    local k = i
    local k2 = k * k
    local k3 = k2 * k
    local sink = math.sin(k)
    sum1 = sum1 + (2.0 / 3.0) ^ (k - 1)
    sum2 = sum2 + k ^ -0.5
    sum3 = sum3 + 1.0 / (k * (k + 1.0))
    sum4 = sum4 + 1.0 / (k3 * sink * sink)
    sum5 = sum5 + 1.0 / k3
    sum6 = sum6 + 1.0 / k2
    sum7 = sum7 + alt / (2.0 * k - 1.0)
    alt = -alt
end

print(string.format(
    "partial_sums=%.9f,%.9f,%.9f,%.9f,%.9f,%.9f,%.9f n=%d",
    sum1,
    sum2,
    sum3,
    sum4,
    sum5,
    sum6,
    sum7,
    n
))