# Complete Examples

This document provides several end-to-end usage examples.

All examples below have runnable Rust code in [`examples/luars-example/src/main.rs`](../../examples/luars-example/src/main.rs).

---

## Example 1: 2D Vector

A complete 2D vector type with constructors, methods, and metamethods.

> **Source:** [`examples/luars-example/src/main.rs`](../../examples/luars-example/src/main.rs) — `Vec2` struct and `example_vec2()`

### Rust Definition

```rust
use luars::{LuaUserData, lua_methods};
use std::fmt;

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

    /// Dot product (passed as components)
    pub fn dot(&self, other_x: f64, other_y: f64) -> f64 {
        self.x * other_x + self.y * other_y
    }

    /// Scale both components
    pub fn scale(&mut self, factor: f64) {
        self.x *= factor;
        self.y *= factor;
    }
}
```

### Registration

```rust
let state = vm.main_state();
state.register_type("Vec2", Vec2::__lua_static_methods())?;
```

### Lua Usage

```lua
-- Create vectors
local v = Vec2.new(3, 4)
local z = Vec2.zero()

-- Read fields
print(v.x, v.y)        -- 3.0  4.0

-- Call methods
print(v:length())       -- 5.0

-- Mutating methods
v:scale(2)
print(v.x, v.y)        -- 6.0  8.0

-- Normalize
local old_len = v:normalize()
print(old_len)          -- 10.0
print(v:length())       -- 1.0 (approx)

-- Equality
local a = Vec2.new(1, 2)
local b = Vec2.new(1, 2)
print(a == b)           -- true

-- tostring
print(tostring(v))      -- Vec2(0.6, 0.8)
```

---

## Example 2: Config Object (Read-only Fields)

Demonstrates `#[lua(readonly)]`, `#[lua(skip)]`, and `#[lua(name = "...")]`.

> **Source:** [`examples/luars-example/src/main.rs`](../../examples/luars-example/src/main.rs) — `AppConfig` struct and `example_config()`

### Rust Definition

```rust
#[derive(LuaUserData)]
struct AppConfig {
    pub app_name: String,

    #[lua(readonly)]
    pub version: i64,

    #[lua(name = "max_conn")]
    pub max_connections: u32,

    #[lua(skip)]
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
        format!("{} v{} (max {})", self.app_name, self.version, self.max_connections)
    }
}
```

### Registration and Usage

```rust
state.register_type("AppConfig", AppConfig::__lua_static_methods())?;
```

```lua
local cfg = AppConfig.new("MyApp", 3, 100)

-- Read fields
print(cfg.app_name)      -- "MyApp"
print(cfg.version)       -- 3
print(cfg.max_conn)      -- 100 (uses the Lua alias)
print(cfg:summary())     -- "MyApp v3 (max 100)"

-- Writable field
cfg.app_name = "NewApp"  -- ✅ OK

-- Read-only field → error
cfg.version = 99         -- ❌ error: field 'version' is read-only

-- Skipped field → error
print(cfg.internal_token) -- ❌ field does not exist

-- Rust name → error
print(cfg.max_connections) -- ❌ must use the Lua alias max_conn
```

---

## Example 3: Calculator with Error Handling

Demonstrates `Result<T, E>` return values and `pcall` error catching.

> **Source:** [`examples/luars-example/src/main.rs`](../../examples/luars-example/src/main.rs) — `Calculator` struct and `example_calculator()`

### Rust Definition

```rust
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
            Err(format!("cannot take sqrt of negative number: {}", self.memory))
        } else {
            Ok(self.memory.sqrt())
        }
    }

    pub fn reset(&mut self) {
        self.memory = 0.0;
    }
}
```

### Lua Usage

```lua
local calc = Calculator.new()

calc:add(16)
print(calc.memory)              -- 16.0

local result = calc:divide_by(4)
print(result)                   -- 4.0
print(calc.memory)              -- 4.0

print(calc:sqrt())              -- 2.0

-- Error handling
local ok, err = pcall(function()
    calc:divide_by(0)
end)
print(ok)    -- false
print(err)   -- "...cannot divide by zero"

-- Safe wrapper
local function safe_divide(c, d)
    local ok, result = pcall(function() return c:divide_by(d) end)
    if ok then return result
    else return nil, result end
end

local val, err = safe_divide(calc, 2)
print(val)   -- 2.0
```

---

## Example 4: Multi-type Interaction

Demonstrates multiple UserData types interacting in the same Lua environment.

> **Source:** [`examples/luars-example/src/main.rs`](../../examples/luars-example/src/main.rs) — `Vec2` + `Color` structs and `example_multi_type()`

### Rust Definition

```rust
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

    pub fn red() -> Self { Color { r: 255, g: 0, b: 0 } }
    pub fn green() -> Self { Color { r: 0, g: 255, b: 0 } }
    pub fn blue() -> Self { Color { r: 0, g: 0, b: 255 } }

    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}
```

### Registration and Usage

```rust
state.register_type("Vec2", Vec2::__lua_static_methods())?;
state.register_type("Color", Color::__lua_static_methods())?;
```

```lua
-- Create objects of different types
local pos = Vec2.new(100, 200)
local color = Color.red()

-- Each type's methods work independently
print(pos:length())        -- 223.6...
print(color:hex())         -- "#FF0000"
print(tostring(color))     -- "rgb(255, 0, 0)"

-- Combine in Lua functions
local function describe(name, position, c)
    return string.format(
        "%s at (%g, %g) colored %s",
        name, position.x, position.y, c:hex()
    )
end

print(describe("Player", pos, color))
-- "Player at (100, 200) colored #FF0000"

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
```

---

## Example 5: Pushing an Existing Instance to Lua

Sometimes you don't need a constructor — you just want to pass a pre-created Rust object to Lua:

> **Source:** [`examples/luars-example/src/main.rs`](../../examples/luars-example/src/main.rs) — `example_push_existing()`

```rust
// Create a Rust object
let origin = Vec2 { x: 0.0, y: 0.0 };

// Wrap as userdata and set as a global variable
let state = vm.main_state();
let ud = LuaUserdata::new(origin);
let ud_val = state.create_userdata(ud)?;
state.set_global("origin", ud_val)?;
```

```lua
-- Use directly, no constructor needed
print(origin.x)          -- 0.0
origin.x = 42
print(origin.x)          -- 42.0
print(origin:length())   -- 42.0
```

This approach does not require `register_type` and is useful for:
- Singleton objects (e.g. game state, database connections)
- Context objects passed in from the Rust side
- Scenarios where creating new instances from Lua is not needed
