local size = tonumber(arg and arg[1]) or 600
local inverse = 2.0 / size
local bytes = {}
local total = 0

for y = 0, size - 1 do
    local ci = y * inverse - 1.0
    local bit_num = 0
    local byte_acc = 0
    for x = 0, size - 1 do
        local zr = 0.0
        local zi = 0.0
        local cr = x * inverse - 1.5
        local escaped = 0
        for _ = 1, 50 do
            local zr2 = zr * zr
            local zi2 = zi * zi
            if zr2 + zi2 > 4.0 then
                escaped = 1
                break
            end
            zi = 2.0 * zr * zi + ci
            zr = zr2 - zi2 + cr
        end

        byte_acc = byte_acc * 2 + (1 - escaped)
        bit_num = bit_num + 1
        if bit_num == 8 then
            total = total + byte_acc
            bytes[#bytes + 1] = byte_acc
            bit_num = 0
            byte_acc = 0
        elseif x == size - 1 then
            byte_acc = byte_acc * (2 ^ (8 - bit_num))
            total = total + byte_acc
            bytes[#bytes + 1] = byte_acc
            bit_num = 0
            byte_acc = 0
        end
    end
end

print(string.format("mandelbrot_sum=%d bytes=%d size=%d", total, #bytes, size))