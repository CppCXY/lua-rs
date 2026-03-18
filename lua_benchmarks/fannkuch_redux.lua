local n = tonumber(arg and arg[1]) or 9

local function fannkuch(count)
    local perm = {}
    local perm1 = {}
    local count_arr = {}
    local max_flips = 0
    local checksum = 0
    local perm_count = 0
    local r = count

    for i = 1, count do
        perm1[i] = i
    end

    while true do
        while r ~= 1 do
            count_arr[r] = r
            r = r - 1
        end

        for i = 1, count do
            perm[i] = perm1[i]
        end

        local flips_count = 0
        local k = perm[1]
        while k ~= 1 do
            local half = math.floor(k / 2)
            for i = 1, half do
                perm[i], perm[k - i + 1] = perm[k - i + 1], perm[i]
            end
            flips_count = flips_count + 1
            k = perm[1]
        end

        if flips_count > max_flips then
            max_flips = flips_count
        end

        if perm_count % 2 == 0 then
            checksum = checksum + flips_count
        else
            checksum = checksum - flips_count
        end

        while true do
            if r == count then
                return checksum, max_flips
            end

            local perm0 = perm1[1]
            for i = 1, r do
                perm1[i] = perm1[i + 1]
            end
            perm1[r + 1] = perm0

            count_arr[r + 1] = count_arr[r + 1] - 1
            if count_arr[r + 1] > 0 then
                break
            end
            r = r + 1
        end

        perm_count = perm_count + 1
    end
end

local checksum, max_flips = fannkuch(n)
print(string.format("checksum=%d max_flips=%d n=%d", checksum, max_flips, n))