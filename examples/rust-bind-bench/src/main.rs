// Benchmark: Rust object binding performance in Lua
//
// Tests the performance of:
// 1. Constructor calls (Type.new)
// 2. Field reads (obj.x)
// 3. Field writes (obj.x = val)
// 4. Method calls with &self
// 5. Method calls with &mut self
// 6. Method call returning Self (userdata construction)
// 7. Method call returning Result<T, E>
// 8. Mixed workload (realistic game loop pattern)
// 9. Baseline Lua table comparison

use luars::lua_vm::{LuaVM, SafeOption};
use luars::{LuaResult, LuaUserData, Stdlib, lua_methods};
use std::fmt;

// ---- Benchmark types ----

#[derive(LuaUserData, PartialEq)]
#[lua_impl(Display, PartialEq)]
struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl fmt::Display for Vec2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Vec2({}, {})", self.x, self.y)
    }
}

#[lua_methods]
impl Vec2 {
    pub fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }

    pub fn length(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn length_sq(&self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    pub fn dot(&self, ox: f64, oy: f64) -> f64 {
        self.x * ox + self.y * oy
    }

    pub fn scale(&mut self, factor: f64) {
        self.x *= factor;
        self.y *= factor;
    }

    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
    }

    pub fn normalize(&mut self) -> f64 {
        let len = self.length();
        if len > 0.0 {
            self.x /= len;
            self.y /= len;
        }
        len
    }

    pub fn add(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        Vec2 {
            x: x1 + x2,
            y: y1 + y2,
        }
    }
}

#[derive(LuaUserData)]
struct Counter {
    pub value: i64,
}

#[lua_methods]
impl Counter {
    pub fn new(initial: i64) -> Self {
        Counter { value: initial }
    }

    pub fn increment(&mut self) {
        self.value += 1;
    }

    pub fn get(&self) -> i64 {
        self.value
    }

    pub fn add(&mut self, n: i64) {
        self.value += n;
    }

    pub fn checked_divide(&self, divisor: i64) -> Result<i64, String> {
        if divisor == 0 {
            Err("division by zero".to_string())
        } else {
            Ok(self.value / divisor)
        }
    }
}

fn main() {
    if let Err(e) = run_benchmarks() {
        eprintln!("Benchmark error: {:?}", e);
        std::process::exit(1);
    }
}

fn run_benchmarks() -> LuaResult<()> {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    let state = vm.main_state();
    state.register_type_of::<Vec2>("Vec2")?;
    state.register_type_of::<Counter>("Counter")?;

    state.execute(BENCH_LUA)?;
    Ok(())
}

const BENCH_LUA: &str = r#"
local iterations = 1000000

print("=== Rust Binding Performance Benchmark ===")
print("Iterations:", iterations)
print()

-- Helper
local function bench(name, n, fn_body)
    local start = os.clock()
    fn_body()
    local elapsed = os.clock() - start
    local ops = n / elapsed / 1000
    print(string.format("  %-40s %.3f s  (%8.2f K ops/sec)", name, elapsed, ops))
    return elapsed
end

-- ==========================================================
-- 1. Constructor performance
-- ==========================================================
print("--- Constructor ---")

bench("Vec2.new(x, y)", iterations, function()
    local v
    for i = 1, iterations do
        v = Vec2.new(1.0, 2.0)
    end
end)

bench("Lua table {x=, y=}", iterations, function()
    local v
    for i = 1, iterations do
        v = {x = 1.0, y = 2.0}
    end
end)

print()

-- ==========================================================
-- 2. Field read performance
-- ==========================================================
print("--- Field Read ---")
local v = Vec2.new(3.0, 4.0)
local lt = {x = 3.0, y = 4.0}

bench("ud.x (field read)", iterations, function()
    local s = 0.0
    for i = 1, iterations do
        s = s + v.x
    end
end)

bench("table.x (field read)", iterations, function()
    local s = 0.0
    for i = 1, iterations do
        s = s + lt.x
    end
end)

print()

-- ==========================================================
-- 3. Field write performance
-- ==========================================================
print("--- Field Write ---")

bench("ud.x = val (field write)", iterations, function()
    for i = 1, iterations do
        v.x = i
    end
end)

bench("table.x = val (field write)", iterations, function()
    for i = 1, iterations do
        lt.x = i
    end
end)

print()

-- ==========================================================
-- 4. Method call (no args, return value)
-- ==========================================================
print("--- Method Call (&self, return f64) ---")

bench("v:length()", iterations, function()
    local s = 0.0
    for i = 1, iterations do
        s = s + v:length()
    end
end)

-- Lua function equivalent
local function lua_length(t)
    return (t.x * t.x + t.y * t.y) ^ 0.5
end

bench("lua_length(t) (pure Lua)", iterations, function()
    local s = 0.0
    for i = 1, iterations do
        s = s + lua_length(lt)
    end
end)

print()

-- ==========================================================
-- 5. Method call with args
-- ==========================================================
print("--- Method Call (&self, 2 args) ---")

bench("v:dot(ox, oy)", iterations, function()
    local s = 0.0
    for i = 1, iterations do
        s = s + v:dot(1.0, 0.0)
    end
end)

local function lua_dot(t, ox, oy)
    return t.x * ox + t.y * oy
end

bench("lua_dot(t, ox, oy) (pure Lua)", iterations, function()
    local s = 0.0
    for i = 1, iterations do
        s = s + lua_dot(lt, 1.0, 0.0)
    end
end)

print()

-- ==========================================================
-- 6. Mutating method call (&mut self)
-- ==========================================================
print("--- Mutating Method Call (&mut self) ---")

bench("v:scale(factor)", iterations, function()
    v.x = 1.0; v.y = 1.0
    for i = 1, iterations do
        v:scale(1.000001)
    end
end)

bench("v:translate(dx, dy)", iterations, function()
    v.x = 0.0; v.y = 0.0
    for i = 1, iterations do
        v:translate(0.001, 0.001)
    end
end)

print()

-- ==========================================================
-- 7. Counter: simple &mut self, no return
-- ==========================================================
print("--- Simple Mutation (Counter) ---")

local c = Counter.new(0)

bench("c:increment()", iterations, function()
    for i = 1, iterations do
        c:increment()
    end
end)

bench("c:add(n)", iterations, function()
    for i = 1, iterations do
        c:add(1)
    end
end)

bench("c.value (field read)", iterations, function()
    local s = 0
    for i = 1, iterations do
        s = s + c.value
    end
end)

print()

-- ==========================================================
-- 8. Result<T, E> method (error path cold)
-- ==========================================================
print("--- Result<T, E> Return (ok path) ---")

c = Counter.new(100)

bench("c:checked_divide(2)", iterations, function()
    local s = 0
    for i = 1, iterations do
        s = s + c:checked_divide(2)
    end
end)

print()

-- ==========================================================
-- 9. Constructor + method chain (returning Self)
-- ==========================================================
print("--- Static Returning Self ---")

bench("Vec2.add(x1,y1,x2,y2)", iterations, function()
    local r
    for i = 1, iterations do
        r = Vec2.add(1.0, 2.0, 3.0, 4.0)
    end
end)

print()

-- ==========================================================
-- 10. Mixed realistic workload
-- ==========================================================
print("--- Mixed Workload (game loop style) ---")

local mix_iters = iterations / 2

bench("construct + field + method + mutate", mix_iters, function()
    for i = 1, mix_iters do
        local p = Vec2.new(i * 0.01, i * 0.02)
        local d = p:length()
        p:scale(1.0 / (d + 0.001))
        local _ = p.x + p.y
    end
end)

-- Pure Lua equivalent
local function lua_new_vec(x, y) return {x=x, y=y} end
local function lua_vec_length(v) return (v.x*v.x + v.y*v.y)^0.5 end
local function lua_vec_scale(v, f) v.x = v.x*f; v.y = v.y*f end

bench("pure Lua table equivalent", mix_iters, function()
    for i = 1, mix_iters do
        local p = lua_new_vec(i * 0.01, i * 0.02)
        local d = lua_vec_length(p)
        lua_vec_scale(p, 1.0 / (d + 0.001))
        local _ = p.x + p.y
    end
end)

print()
print("=== Benchmark Complete ===")
"#;
