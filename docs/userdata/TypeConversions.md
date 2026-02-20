# Type Conversion Reference

This document lists all supported Rust ↔ Lua type mappings for `#[lua_methods]` and `#[derive(LuaUserData)]`.

> **See also:** [FromLua / IntoLua](../guide/05-FromLuaIntoLua.md) — the underlying conversion traits used by `#[lua_methods]`. You can implement these traits for your own types.

## Parameter Types (Lua → Rust)

Method parameters are extracted from the Lua stack and converted to Rust types.

### Integer Types

| Rust Type | Lua Source | Conversion |
|----------|-----------|------------|
| `i8`, `i16`, `i32`, `i64`, `isize` | integer / number | `as_integer()`, floats truncated to integer |
| `u8`, `u16`, `u32`, `u64`, `usize` | integer / number | same as above |

```rust
pub fn set_level(&mut self, level: i64) { ... }
```

```lua
obj:set_level(42)       -- ✅ integer
obj:set_level(42.0)     -- ✅ float truncated to integer
obj:set_level("hello")  -- ❌ error: expected integer
```

### Float Types

| Rust Type | Lua Source | Conversion |
|----------|-----------|------------|
| `f32`, `f64` | number / integer | `as_number()`, integers auto-converted to float |

```rust
pub fn set_speed(&mut self, speed: f64) { ... }
```

```lua
obj:set_speed(3.14)     -- ✅
obj:set_speed(42)       -- ✅ integer → 42.0
obj:set_speed("fast")   -- ❌ error: expected number
```

### Boolean Type

| Rust Type | Lua Source | Conversion |
|----------|-----------|------------|
| `bool` | boolean | `as_boolean()`, missing value defaults to `false` |

```rust
pub fn set_visible(&mut self, visible: bool) { ... }
```

```lua
obj:set_visible(true)   -- ✅
obj:set_visible(false)  -- ✅
```

### String Types

| Rust Type | Lua Source | Conversion |
|----------|-----------|------------|
| `String` | string | `as_str().to_owned()` |
| `&str` | string | same (actually `String`) |

```rust
pub fn set_name(&mut self, name: String) { ... }
```

```lua
obj:set_name("Alice")   -- ✅
obj:set_name(42)        -- ❌ error: expected string
```

### Option Wrapper

`Option<T>` parameters represent optional arguments. Passing `nil` or omitting the argument yields `None`:

| Rust Type | Lua nil / missing | Lua has value |
|----------|------------------|--------------|
| `Option<i64>` | `None` | `Some(integer)` |
| `Option<f64>` | `None` | `Some(number)` |
| `Option<bool>` | `None` | `Some(boolean)` |
| `Option<String>` | `None` | `Some(string)` |

```rust
pub fn greet(&self, name: Option<String>) -> String {
    match name {
        Some(n) => format!("Hello {}", n),
        None => "Hello anonymous".into(),
    }
}
```

```lua
obj:greet("Alice")   -- "Hello Alice"
obj:greet()          -- "Hello anonymous" (missing arg → None)
obj:greet(nil)       -- "Hello anonymous" (nil → None)
```

## Return Types (Rust → Lua)

Method return values are converted from Rust and pushed onto the Lua stack.

### No Return Value

```rust
pub fn reset(&mut self) { self.x = 0.0; }
```

Calling from Lua returns nothing (0 return values).

### Basic Types

| Rust Type | Lua Type | Push Method |
|----------|---------|-------------|
| `i8`..`i64`, `u8`..`u64`, `isize`, `usize` | integer | `LuaValue::integer(v as i64)` |
| `f32`, `f64` | number | `LuaValue::float(v as f64)` |
| `bool` | boolean | `LuaValue::boolean(v)` |
| `String`, `&str` | string | `create_string(&v)` |

```rust
pub fn distance(&self) -> f64 { ... }
pub fn name(&self) -> String { ... }
pub fn is_alive(&self) -> bool { ... }
```

```lua
local d = obj:distance()   -- number
local n = obj:name()       -- string
local a = obj:is_alive()   -- boolean
```

### Option\<T\> Return Values

`None` maps to Lua `nil`, `Some(v)` is pushed according to the inner type:

```rust
pub fn find_item(&self, name: String) -> Option<i64> {
    self.items.get(&name).copied()
}
```

```lua
local id = obj:find_item("sword")   -- integer or nil
if id then
    print("found:", id)
end
```

### Result\<T, E\> Return Values

`Ok(v)` returns normally, `Err(e)` triggers a Lua error (catchable with `pcall`):

```rust
pub fn divide(&self, divisor: f64) -> Result<f64, String> {
    if divisor == 0.0 {
        Err("division by zero".into())
    } else {
        Ok(self.value / divisor)
    }
}
```

```lua
-- Normal call
local result = obj:divide(2)      -- returns number

-- Error handling
local ok, err = pcall(function()
    return obj:divide(0)
end)
print(ok)    -- false
print(err)   -- "...division by zero"
```

**Requirement:** The `E` type must implement `Display` (i.e. `format!("{}", err)` must work).

> **Runnable example:** See `Calculator` in [`examples/luars-example/src/main.rs`](../../examples/luars-example/src/main.rs) — `example_calculator()`

### Self Return Value

Only for associated functions (no `self` parameter). The returned struct is automatically wrapped as userdata:

```rust
pub fn new(x: f64, y: f64) -> Self {
    Point { x, y }
}
```

Conversion flow:
1. Call `Point::new(x, y)` to get a `Point` instance
2. `LuaUserdata::new(result)` wraps it
3. `create_userdata(ud)` allocates to GC
4. `push_value(ud_val)` pushes onto the Lua stack

## Field Types (#[derive(LuaUserData)])

Types supported by `get_field` and `set_field`:

### get_field (Rust → UdValue → LuaValue)

| Rust Field Type | UdValue | Final LuaValue |
|----------------|---------|----------------|
| `i8`..`i64`, `u8`..`u64`, `isize`, `usize` | `UdValue::Integer(v as i64)` | integer |
| `f32`, `f64` | `UdValue::Number(v as f64)` | number |
| `bool` | `UdValue::Boolean(v)` | boolean |
| `String` | `UdValue::Str(v.clone())` | string |

### set_field (LuaValue → UdValue → Rust)

| UdValue Input | Target Rust Type | Conversion |
|--------------|-----------------|------------|
| `UdValue::Integer(i)` | integer types | `i as T` (unsigned types check `i >= 0`) |
| `UdValue::Number(n)` | float types | `n as T` |
| `UdValue::Boolean(b)` | `bool` | direct assignment |
| `UdValue::Str(s)` | `String` | `s.to_owned()` |
| type mismatch | any | returns an error string |

## Summary Table

| Context | Supported Types |
|---------|----------------|
| Method parameters | `i8`..`u64`, `f32`, `f64`, `bool`, `String`, `Option<T>` |
| Method return values | above + `()`, `Result<T, E>`, `Self` (associated functions only) |
| Struct fields | `i8`..`u64`, `f32`, `f64`, `bool`, `String` |

## Next

- [Complete Examples](Examples.md) — end-to-end usage examples
