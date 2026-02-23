# #[derive(LuaUserData)]

`#[derive(LuaUserData)]` auto-generates a `UserDataTrait` implementation for your Rust struct, exposing `pub` fields to Lua for reading and writing.

## Basic Usage

```rust
#[derive(LuaUserData)]
struct Player {
    pub name: String,
    pub health: f64,
    pub level: i64,
    /// Private field — not exposed to Lua
    internal_id: u64,
}
```

In Lua:

```lua
print(player.name)        -- "Alice"
print(player.health)      -- 100.0
player.health = 80.0      -- writable
print(player.internal_id) -- error! private field not accessible
```

**Rule:** Only `pub` fields are exposed to Lua. Private fields are completely invisible.

## Field Attributes

### `#[lua(skip)]` — Skip a field

Hides a field from Lua even if it is `pub`:

```rust
#[derive(LuaUserData)]
struct Config {
    pub name: String,
    #[lua(skip)]
    pub secret_key: String,   // pub but invisible to Lua
}
```

```lua
print(cfg.name)       -- "my_app"
print(cfg.secret_key) -- error! field does not exist
```

### `#[lua(readonly)]` — Read-only field

Allows Lua to read the field but not write to it:

```rust
#[derive(LuaUserData)]
struct Config {
    pub name: String,
    #[lua(readonly)]
    pub version: i64,   // read-only
}
```

```lua
print(cfg.version)   -- 42
cfg.version = 99     -- error! field 'version' is read-only
cfg.name = "new"     -- OK, name is writable
```

### `#[lua(name = "...")]` — Rename a field

Use a different name in Lua:

```rust
#[derive(LuaUserData)]
struct Config {
    #[lua(name = "count")]
    pub item_count: u32,
}
```

```lua
print(cfg.count)      -- 42 (uses the Lua name)
print(cfg.item_count) -- error! Rust name is not available
```

### Combining attributes

Attributes can be combined:

```rust
#[derive(LuaUserData)]
struct GameEntity {
    pub name: String,                    // read-write
    #[lua(readonly)]
    pub entity_type: String,             // read-only
    #[lua(name = "hp")]
    pub hit_points: f64,                 // renamed to "hp"
    #[lua(skip)]
    pub cache: Vec<u8>,                  // hidden from Lua
    _internal: u32,                      // private, automatically hidden
}
```

> **Runnable example:** See `AppConfig` in [`examples/luars-example/src/main.rs`](../../examples/luars-example/src/main.rs) — `example_config()`

## Metamethod Mapping (`#[lua_impl]`)

Use `#[lua_impl(...)]` to automatically map Rust standard traits to Lua metamethods:

```rust
#[derive(LuaUserData, Clone, PartialEq, PartialOrd)]
#[lua_impl(Display, PartialEq, PartialOrd, Add, Sub, Neg)]
struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl std::fmt::Display for Vec2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Vec2({}, {})", self.x, self.y)
    }
}

impl std::ops::Add for Vec2 {
    type Output = Vec2;
    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2 { x: self.x + rhs.x, y: self.y + rhs.y }
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2 { x: self.x - rhs.x, y: self.y - rhs.y }
    }
}

impl std::ops::Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2 { x: -self.x, y: -self.y }
    }
}
```

### Supported Mappings

| Rust Trait | Lua Metamethod | Lua Usage |
|-----------|----------------|-----------|
| `Display` | `__tostring` | `tostring(obj)` / `print(obj)` |
| `PartialEq` | `__eq` | `obj1 == obj2` |
| `PartialOrd` | `__lt` / `__le` | `obj1 < obj2` / `obj1 <= obj2` |
| `Add` | `__add` | `obj1 + obj2` |
| `Sub` | `__sub` | `obj1 - obj2` |
| `Mul` | `__mul` | `obj1 * obj2` |
| `Div` | `__div` | `obj1 / obj2` |
| `Rem` | `__mod` | `obj1 % obj2` |
| `Neg` | `__unm` | `-obj` |

**Requirements for arithmetic operators:**
- The struct must derive `Clone` (needed for the operation — originals are not consumed)
- You must implement the corresponding `std::ops` trait (e.g. `impl Add for MyType`)
- List the trait name in `#[lua_impl(...)]`

**Note:** Arithmetic operators work on same-type operands (e.g. `Vec2 + Vec2`). The result is a new GC-managed userdata with full field access and method calls.

### Example: Object with Metamethods

```rust
#[derive(LuaUserData, PartialEq, PartialOrd)]
#[lua_impl(Display, PartialEq, PartialOrd)]
struct Score {
    pub value: i64,
    pub player: String,
}

impl std::fmt::Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.player, self.value)
    }
}

#[lua_methods]
impl Score {
    pub fn new(value: i64, player: String) -> Self {
        Score { value, player }
    }
}
```

```lua
-- Create instances via the constructor
local a = Score.new(100, "Alice")
local b = Score.new(200, "Bob")

print(a)        -- "Alice: 100"
print(a == b)   -- false
print(a < b)    -- true (compared via derived PartialOrd)
```

## Supported Field Types

| Rust Type | Lua Type | Notes |
|----------|---------|-------|
| `i8`, `i16`, `i32`, `i64`, `isize` | integer | read/write supported |
| `u8`, `u16`, `u32`, `u64`, `usize` | integer | non-negative check on write |
| `f32`, `f64` | number | read/write supported |
| `bool` | boolean | read/write supported |
| `String` | string | cloned on read, converted from Lua string on write |

Other types can exist in the struct but should be marked with `#[lua(skip)]`, or the type must implement `Into<UdValue>`.

## Generated Code

`#[derive(LuaUserData)]` generates the following for your struct:

1. **`UserDataTrait` implementation**
   - `type_name()` → returns the struct name (e.g. `"Point"`)
   - `get_field(key)` → matches field names and returns values; falls back to method lookup for unknown keys
   - `set_field(key, value)` → matches field names and sets values
   - `field_names()` → returns a list of all exposed field names
   - `as_any()` / `as_any_mut()` → for type downcasting
   - Metamethods (if `#[lua_impl(...)]` is used)

2. **Field lookup fallback to methods**
   - When `get_field` can't find a matching field name, it calls `Self::__lua_lookup_method(key)` to look up methods
   - This function is generated by `#[lua_methods]` (see [#[lua_methods]](LuaMethods.md))

## Next

- [#[lua_methods]](LuaMethods.md) — defining methods and constructors
- [Type Conversions](TypeConversions.md) — detailed conversion rules for parameters and return values
