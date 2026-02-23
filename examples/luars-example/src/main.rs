// luars UserData Examples
//
// This file demonstrates the luars UserData API with four complete examples:
//   1. Vec2  — 2D vector with constructor, methods, and metamethods
//   2. AppConfig — readonly / skip / rename field attributes
//   3. Calculator — Result<T, E> returns and pcall error handling
//   4. Multi-type interaction — multiple UserData types working together

use luars::lua_vm::{LuaVM, SafeOption};
use luars::{LuaResult, LuaUserData, LuaUserdata, Stdlib, lua_methods};
use std::fmt;

// ---------------------------------------------------------------------------
// Example 1: Vec2 — 2D vector
// ---------------------------------------------------------------------------

#[derive(LuaUserData, PartialEq, PartialOrd)]
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
    /// Constructor
    pub fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }

    /// Zero vector
    pub fn zero() -> Self {
        Vec2 { x: 0.0, y: 0.0 }
    }

    /// Euclidean length
    pub fn length(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Normalize in-place, returns the original length
    pub fn normalize(&mut self) -> f64 {
        let len = self.length();
        if len > 0.0 {
            self.x /= len;
            self.y /= len;
        }
        len
    }

    /// Dot product with another vector (passed as components)
    pub fn dot(&self, other_x: f64, other_y: f64) -> f64 {
        self.x * other_x + self.y * other_y
    }

    /// Scale both components by a factor
    pub fn scale(&mut self, factor: f64) {
        self.x *= factor;
        self.y *= factor;
    }

    /// Internal helper — skipped from Lua, only callable from Rust
    #[lua(skip)]
    #[allow(dead_code)]
    pub fn raw_components(&self) -> (f64, f64) {
        (self.x, self.y)
    }
}

// ---------------------------------------------------------------------------
// Example 2: AppConfig — field attributes
// ---------------------------------------------------------------------------

#[derive(LuaUserData)]
struct AppConfig {
    /// Normal read-write field
    pub app_name: String,

    /// Read-only: Lua can read but not write
    #[lua(readonly)]
    pub version: i64,

    /// Renamed: exposed as "max_conn" in Lua
    #[lua(name = "max_conn")]
    pub max_connections: u32,

    /// Skipped: invisible to Lua even though it's `pub`
    #[lua(skip)]
    #[allow(dead_code)]
    pub internal_token: String,
}

#[lua_methods]
impl AppConfig {
    pub fn new(name: String, version: i64, max_conn: i64) -> Self {
        AppConfig {
            app_name: name,
            version,
            max_connections: max_conn as u32,
            internal_token: "secret-token".into(),
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "{} v{} (max {})",
            self.app_name, self.version, self.max_connections
        )
    }
}

// ---------------------------------------------------------------------------
// Example 3: Calculator — Result returns and error handling
// ---------------------------------------------------------------------------

#[derive(LuaUserData)]
struct Calculator {
    #[lua(readonly)]
    pub memory: f64,
}

#[lua_methods]
impl Calculator {
    pub fn new() -> Self {
        Calculator { memory: 0.0 }
    }

    pub fn add(&mut self, value: f64) {
        self.memory += value;
    }

    pub fn divide_by(&mut self, divisor: f64) -> Result<f64, String> {
        if divisor == 0.0 {
            Err("cannot divide by zero".into())
        } else {
            self.memory /= divisor;
            Ok(self.memory)
        }
    }

    pub fn sqrt(&self) -> Result<f64, String> {
        if self.memory < 0.0 {
            Err(format!(
                "cannot take sqrt of negative number: {}",
                self.memory
            ))
        } else {
            Ok(self.memory.sqrt())
        }
    }

    pub fn reset(&mut self) {
        self.memory = 0.0;
    }
}

// ---------------------------------------------------------------------------
// Example 4: Color — a second type for multi-type interaction
// ---------------------------------------------------------------------------

#[derive(LuaUserData)]
#[lua_impl(Display)]
struct Color {
    pub r: i64,
    pub g: i64,
    pub b: i64,
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rgb({}, {}, {})", self.r, self.g, self.b)
    }
}

#[lua_methods]
impl Color {
    pub fn new(r: i64, g: i64, b: i64) -> Self {
        Color { r, g, b }
    }

    pub fn red() -> Self {
        Color { r: 255, g: 0, b: 0 }
    }
    pub fn green() -> Self {
        Color { r: 0, g: 255, b: 0 }
    }
    pub fn blue() -> Self {
        Color { r: 0, g: 0, b: 255 }
    }

    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

// ---------------------------------------------------------------------------
// Main — run all examples
// ---------------------------------------------------------------------------

fn main() {
    example_vec2().expect("Vec2 example failed");
    example_config().expect("AppConfig example failed");
    example_calculator().expect("Calculator example failed");
    example_multi_type().expect("Multi-type example failed");
    example_push_existing().expect("Push existing example failed");
    example_borrowed_ref().expect("Borrowed ref example failed");
    println!("\nAll examples completed successfully!");
}

/// Example 1: Vec2 — constructor, fields, methods, metamethods
fn example_vec2() -> LuaResult<()> {
    println!("=== Example 1: Vec2 ===");

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    let state = vm.main_state();
    state.register_type_of::<Vec2>("Vec2")?;

    let results = state.execute(
        r#"
        -- Create vectors via constructors
        local v = Vec2.new(3, 4)
        local z = Vec2.zero()

        -- Read fields
        print("v = " .. tostring(v))         -- Vec2(3, 4)
        print("z = " .. tostring(z))         -- Vec2(0, 0)
        print("v.x =", v.x, "v.y =", v.y)   -- 3.0  4.0

        -- Call methods
        print("length =", v:length())        -- 5.0

        -- Mutating methods
        v:scale(2)
        print("after scale(2):", v.x, v.y)   -- 6.0  8.0

        -- Normalize
        local old_len = v:normalize()
        print("old length =", old_len)        -- 10.0
        print("normalized length =", v:length())

        -- Equality (via __eq metamethod)
        local a = Vec2.new(1, 2)
        local b = Vec2.new(1, 2)
        print("a == b:", a == b)              -- true

        return v.x
    "#,
    )?;

    println!("Rust received: {:?}", results[0].as_number());
    println!();
    Ok(())
}

#[allow(unused)]
/// Example 2: AppConfig — readonly, skip, name attributes
fn example_config() -> LuaResult<()> {
    println!("=== Example 2: AppConfig ===");

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    let state = vm.main_state();
    state.register_type_of::<AppConfig>("AppConfig")?;

    state.execute(
        r#"
        local cfg = AppConfig.new("MyApp", 3, 100)

        -- Read fields
        print("app_name =", cfg.app_name)     -- MyApp
        print("version =", cfg.version)       -- 3
        print("max_conn =", cfg.max_conn)     -- 100 (renamed field)
        print("summary =", cfg:summary())     -- MyApp v3 (max 100)

        -- Writable field
        cfg.app_name = "NewApp"
        print("updated app_name =", cfg.app_name)

        -- Read-only field error
        local ok, err = pcall(function()
            cfg.version = 99
        end)
        print("set readonly ok?", ok)         -- false
        print("error:", err)

        -- Skipped field is invisible
        local ok2, err2 = pcall(function()
            return cfg.internal_token
        end)
        print("access skipped ok?", ok2)      -- false
    "#,
    )?;
    println!();
    Ok(())
}

/// Example 3: Calculator — Result returns and pcall error handling
fn example_calculator() -> LuaResult<()> {
    println!("=== Example 3: Calculator ===");

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    let state = vm.main_state();
    state.register_type_of::<Calculator>("Calculator")?;

    state.execute(
        r#"
        local calc = Calculator.new()

        calc:add(16)
        print("memory =", calc.memory)            -- 16.0

        local result = calc:divide_by(4)
        print("after /4 =", result)                -- 4.0
        print("memory =", calc.memory)             -- 4.0

        print("sqrt =", calc:sqrt())               -- 2.0

        -- Error: divide by zero
        local ok, err = pcall(function()
            calc:divide_by(0)
        end)
        print("divide by 0 ok?", ok)               -- false
        print("error:", err)

        -- Reset and test negative sqrt
        calc:reset()
        calc:add(-9)
        local ok2, err2 = pcall(function()
            return calc:sqrt()
        end)
        print("sqrt(-9) ok?", ok2)                 -- false
        print("error:", err2)
    "#,
    )?;
    println!();
    Ok(())
}

/// Example 4: Multiple UserData types working together
fn example_multi_type() -> LuaResult<()> {
    println!("=== Example 4: Multi-type Interaction ===");

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    let state = vm.main_state();
    state.register_type_of::<Vec2>("Vec2")?;
    state.register_type_of::<Color>("Color")?;

    state.execute(
        r#"
        -- Create objects of different types
        local pos = Vec2.new(100, 200)
        local color = Color.red()

        -- Each type's methods work independently
        print("position length =", pos:length())
        print("color hex =", color:hex())           -- #FF0000
        print("color =", tostring(color))            -- rgb(255, 0, 0)

        -- Combine in Lua functions
        local function describe(name, position, c)
            return string.format(
                "%s at (%g, %g) colored %s",
                name, position.x, position.y, c:hex()
            )
        end

        print(describe("Player", pos, color))

        -- Predefined constructors
        local colors = {
            Color.red(),
            Color.green(),
            Color.blue(),
            Color.new(128, 128, 128),
        }
        for i, c in ipairs(colors) do
            print(i, tostring(c))
        end
    "#,
    )?;
    println!();
    Ok(())
}

/// Example 5: Pushing an existing Rust instance to Lua (no constructor needed)
fn example_push_existing() -> LuaResult<()> {
    println!("=== Example 5: Push Existing Instance ===");

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    // Create a Rust object and push it as a Lua global
    let origin = Vec2 { x: 0.0, y: 0.0 };
    let state = vm.main_state();
    let ud = LuaUserdata::new(origin);
    let ud_val = state.create_userdata(ud)?;
    state.set_global("origin", ud_val)?;

    state.execute(
        r#"
        -- Use the pre-created instance directly
        print("origin =", tostring(origin))          -- Vec2(0, 0)
        print("origin.x =", origin.x)                -- 0.0
        origin.x = 42
        print("updated origin.x =", origin.x)        -- 42.0
        print("origin:length() =", origin:length())   -- 42.0
    "#,
    )?;
    println!();
    Ok(())
}

/// Example 6: Borrowed reference — Rust object lent to Lua without ownership transfer
fn example_borrowed_ref() -> LuaResult<()> {
    println!("=== Example 6: Borrowed Reference ===");

    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All)?;

    // A Rust-owned object that we want to share with Lua temporarily
    let mut player_pos = Vec2 { x: 100.0, y: 200.0 };

    println!("Before Lua: player_pos = ({}, {})", player_pos.x, player_pos.y);

    {
        let state = vm.main_state();

        // Lend to Lua by reference — no ownership transfer, zero overhead
        // Safety: player_pos outlives the execute_string call below
        let ud_val = unsafe { state.create_userdata_ref(&mut player_pos)? };
        state.set_global("pos", ud_val)?;

        state.execute(
            r#"
            -- Lua can read and write fields, call methods
            print("pos =", tostring(pos))           -- Vec2(100, 200)
            print("length =", pos:length())          -- ~223.6
            pos.x = 300
            pos.y = 400
            print("updated pos =", tostring(pos))   -- Vec2(300, 400)
        "#,
        )?;
    }

    // Back in Rust — mutations from Lua are visible!
    println!("After Lua:  player_pos = ({}, {})", player_pos.x, player_pos.y);
    assert_eq!(player_pos.x, 300.0);
    assert_eq!(player_pos.y, 400.0);

    println!();
    Ok(())
}
