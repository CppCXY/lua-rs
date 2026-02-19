# Getting Started

Get a complete Rust → Lua type mapping up and running in 5 minutes.

> **Runnable example:** [`examples/luars-example/src/main.rs`](../../examples/luars-example/src/main.rs) — `example_vec2()`

## Minimal Example

### 1. Define a Rust struct

```rust
use luars::{LuaUserData, lua_methods};
use std::fmt;

#[derive(LuaUserData, PartialEq)]
#[lua_impl(Display, PartialEq)]
struct Point {
    pub x: f64,
    pub y: f64,
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Point({}, {})", self.x, self.y)
    }
}

#[lua_methods]
impl Point {
    /// Constructor — called as Point.new(x, y) in Lua
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    /// Instance method — called as p:distance() in Lua
    pub fn distance(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Mutating method — called as p:translate(dx, dy) in Lua
    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
    }
}
```

### 2. Register with the Lua VM

```rust
use luars::lua_vm::{LuaVM, SafeOption};
use luars::Stdlib;

fn main() {
    // Create a VM and load the standard library
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::Basic).unwrap();

    // Register the Point type — creates a "Point" table in Lua globals
    let state = vm.main_state();
    state.register_type("Point", Point::__lua_static_methods()).unwrap();

    // Execute Lua code
    let results = vm.execute_string(r#"
        local p = Point.new(3, 4)
        print(p.x, p.y)         -- 3.0  4.0
        print(p:distance())     -- 5.0
        p:translate(10, 20)
        print(tostring(p))      -- Point(13, 24)
        return p.x
    "#).unwrap();

    println!("Rust got back: {:?}", results[0].as_number());
}
```

### 3. Run

```bash
cargo run --release
```

Output:

```
3.0	4.0
5.0
Point(13, 24)
Rust got back: Some(13.0)
```

## What happened?

| Macro / API | Purpose |
|------------|---------|
| `#[derive(LuaUserData)]` | Auto-generates `UserDataTrait`, exposing `pub` fields to Lua for reading/writing |
| `#[lua_impl(Display, PartialEq)]` | Maps Rust `Display` → `__tostring`, `PartialEq` → `__eq` |
| `#[lua_methods]` | Wraps `pub fn` items as Lua-callable C functions |
| `pub fn new(...) -> Self` | No `self` parameter → associated function → registered as a static method |
| `pub fn distance(&self)` | `&self` → instance method → called via `p:distance()` |
| `pub fn translate(&mut self, ...)` | `&mut self` → mutable instance method |
| `register_type("Point", ...)` | Creates a Lua global table `Point` populated with static methods (e.g. `new`) |

## Next

- [#[derive(LuaUserData)]](DeriveUserData.md) — field attributes (skip, readonly, name)
- [#[lua_methods]](LuaMethods.md) — methods, constructors, return value handling
- [register_type](RegisterType.md) — type registration flow
- [Type Conversions](TypeConversions.md) — supported parameter and return types
