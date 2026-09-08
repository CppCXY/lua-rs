-- oop-dots.lua
-- Benchmark: Table field access (Hash/String keys) and Metatable Method Dispatch
-- Realism: High (Simulates Game Objects, Scene Graphs, ECS)

local setmetatable = setmetatable

-- 1. Class Definition (Standard Lua OOP pattern)
local Dot = {}
Dot.__index = Dot

-- Constructor
function Dot.new(x, y, z)
   local self = {
      -- Data is stored in hash part (string keys)
      x = x, 
      y = y, 
      z = z,
      vx = 1.5,
      vy = -0.5,
      vz = 0.1,
      life = 1000.0,
      id = "dot_instance" -- String processing overhead
   }
   return setmetatable(self, Dot)
end

-- Method Call overhead check
function Dot:move()
   -- Extensive reading/writing of fields
   -- In PUC Lua: Hashing string keys "x", "vx", etc. every time
   -- In LuaJIT: Should optimize to pointer offsets
   self.x = self.x + self.vx
   self.y = self.y + self.vy
   self.z = self.z + self.vz
   
   -- Simple logic to keep numbers bounded
   if self.x > 1000 or self.x < -1000 then self.vx = -self.vx end
   if self.y > 1000 or self.y < -1000 then self.vy = -self.vy end
   
   self.life = self.life - 0.01
end

function Dot:normalize()
   -- Math stress mixed with object access
   local mag = (self.x^2 + self.y^2 + self.z^2)^0.5
   self.x = self.x / mag
   self.y = self.y / mag
   self.z = self.z / mag
end

-- 2. Simulation
local N = tonumber(arg and arg[1]) or 100
local iterations = N * 100

-- Create a "Scene" of objects
local dots = {}
local num_dots = 1000 -- Fixed pool size

for i = 1, num_dots do
   dots[i] = Dot.new(i * 0.1, i * -0.2, i * 0.5)
end

-- 3. Hot Loop
local check_sum = 0
for iter = 1, iterations do
   for i = 1, num_dots do
      local d = dots[i]
      -- Method dispatch "__index" lookup stress
      d:move()
      if iter % 100 == 0 then
         d:normalize()
      end
   end
end

-- Validate result
for i = 1, num_dots do
   check_sum = check_sum + dots[i].x + dots[i].y
end

io.write(string.format("Dots: %d, Iterations: %d\n", num_dots, iterations))
io.write(string.format("Check Sum: %0.5f\n", check_sum))
