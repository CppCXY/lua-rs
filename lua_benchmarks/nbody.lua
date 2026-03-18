local loops = tonumber(arg and arg[1]) or 500000
local pi = 3.141592653589793
local solar_mass = 4 * pi * pi
local days_per_year = 365.24

local bodies = {
    {
        x = 0.0, y = 0.0, z = 0.0,
        vx = 0.0, vy = 0.0, vz = 0.0,
        mass = solar_mass,
    },
    {
        x = 4.84143144246472090e+00,
        y = -1.16032004402742839e+00,
        z = -1.03622044471123109e-01,
        vx = 1.66007664274403694e-03 * days_per_year,
        vy = 7.69901118419740425e-03 * days_per_year,
        vz = -6.90460016972063023e-05 * days_per_year,
        mass = 9.54791938424326609e-04 * solar_mass,
    },
    {
        x = 8.34336671824457987e+00,
        y = 4.12479856412430479e+00,
        z = -4.03523417114321381e-01,
        vx = -2.76742510726862411e-03 * days_per_year,
        vy = 4.99852801234917238e-03 * days_per_year,
        vz = 2.30417297573763929e-05 * days_per_year,
        mass = 2.85885980666130812e-04 * solar_mass,
    },
    {
        x = 1.28943695621391310e+01,
        y = -1.51111514016986312e+01,
        z = -2.23307578892655734e-01,
        vx = 2.96460137564761618e-03 * days_per_year,
        vy = 2.37847173959480950e-03 * days_per_year,
        vz = -2.96589568540237556e-05 * days_per_year,
        mass = 4.36624404335156298e-05 * solar_mass,
    },
    {
        x = 1.53796971148509165e+01,
        y = -2.59193146099879641e+01,
        z = 1.79258772950371181e-01,
        vx = 2.68067772490389322e-03 * days_per_year,
        vy = 1.62824170038242295e-03 * days_per_year,
        vz = -9.51592254519715870e-05 * days_per_year,
        mass = 5.15138902046611451e-05 * solar_mass,
    },
}

local function offset_momentum()
    local px, py, pz = 0.0, 0.0, 0.0
    for i = 1, #bodies do
        local body = bodies[i]
        px = px + body.vx * body.mass
        py = py + body.vy * body.mass
        pz = pz + body.vz * body.mass
    end
    local sun = bodies[1]
    sun.vx = -px / solar_mass
    sun.vy = -py / solar_mass
    sun.vz = -pz / solar_mass
end

local function advance(dt)
    for i = 1, #bodies - 1 do
        local body_i = bodies[i]
        for j = i + 1, #bodies do
            local body_j = bodies[j]
            local dx = body_i.x - body_j.x
            local dy = body_i.y - body_j.y
            local dz = body_i.z - body_j.z

            local distance_sq = dx * dx + dy * dy + dz * dz
            local distance = math.sqrt(distance_sq)
            local mag = dt / (distance_sq * distance)

            body_i.vx = body_i.vx - dx * body_j.mass * mag
            body_i.vy = body_i.vy - dy * body_j.mass * mag
            body_i.vz = body_i.vz - dz * body_j.mass * mag
            body_j.vx = body_j.vx + dx * body_i.mass * mag
            body_j.vy = body_j.vy + dy * body_i.mass * mag
            body_j.vz = body_j.vz + dz * body_i.mass * mag
        end
    end

    for i = 1, #bodies do
        local body = bodies[i]
        body.x = body.x + dt * body.vx
        body.y = body.y + dt * body.vy
        body.z = body.z + dt * body.vz
    end
end

local function energy()
    local e = 0.0
    for i = 1, #bodies do
        local body_i = bodies[i]
        e = e + 0.5 * body_i.mass * (body_i.vx * body_i.vx + body_i.vy * body_i.vy + body_i.vz * body_i.vz)
        for j = i + 1, #bodies do
            local body_j = bodies[j]
            local dx = body_i.x - body_j.x
            local dy = body_i.y - body_j.y
            local dz = body_i.z - body_j.z
            local distance = math.sqrt(dx * dx + dy * dy + dz * dz)
            e = e - (body_i.mass * body_j.mass) / distance
        end
    end
    return e
end

offset_momentum()
local before = energy()
for _ = 1, loops do
    advance(0.01)
end
local after = energy()
print(string.format("before=%.9f after=%.9f loops=%d", before, after, loops))