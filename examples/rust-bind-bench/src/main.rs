use luars::{Lua, LuaResult, LuaUserData, SafeOption, Stdlib, lua_methods};
use std::fmt;

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
        Self { x, y }
    }

    pub fn length(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn dot(&self, ox: f64, oy: f64) -> f64 {
        self.x * ox + self.y * oy
    }

    pub fn scale(&mut self, factor: f64) {
        self.x *= factor;
        self.y *= factor;
    }
}

#[derive(LuaUserData)]
struct Counter {
    pub value: i64,
}

#[lua_methods]
impl Counter {
    pub fn new(initial: i64) -> Self {
        Self { value: initial }
    }

    pub fn increment(&mut self) {
        self.value += 1;
    }

    pub fn get(&self) -> i64 {
        self.value
    }
}

fn main() {
    if let Err(error) = run_benchmarks() {
        eprintln!("Benchmark error: {:?}", error);
        std::process::exit(1);
    }
}

fn run_benchmarks() -> LuaResult<()> {
    let mut lua = Lua::new(SafeOption::default());
    lua.load_stdlibs(Stdlib::All)?;
    lua.register_type::<Vec2>("Vec2")?;
    lua.register_type::<Counter>("Counter")?;
    lua.load(BENCH_LUA).set_name("rust_bind_bench.lua").exec()?;
    Ok(())
}

const BENCH_LUA: &str = r#"
local iterations = 200000

print("=== High-level luars binding benchmark ===")
print("iterations:", iterations)
print()

local function bench(name, n, fn_body)
    local start = os.clock()
    fn_body()
    local elapsed = os.clock() - start
    local ops = n / elapsed / 1000
    print(string.format("  %-30s %.3f s  (%8.2f K ops/sec)", name, elapsed, ops))
end

local v = Vec2.new(3.0, 4.0)
local c = Counter.new(0)

bench("userdata field read", iterations, function()
    local sum = 0.0
    for _ = 1, iterations do
        sum = sum + v.x
    end
end)

bench("userdata method call", iterations, function()
    local sum = 0.0
    for _ = 1, iterations do
        sum = sum + v:length()
    end
end)

bench("userdata mutation", iterations, function()
    for _ = 1, iterations do
        v:scale(1.000001)
    end
end)

bench("counter increment", iterations, function()
    for _ = 1, iterations do
        c:increment()
    end
end)

print()
print("counter:", c:get())
"#;
