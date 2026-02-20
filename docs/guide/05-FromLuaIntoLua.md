# FromLua / IntoLua

The `FromLua` and `IntoLua` traits provide automatic type conversion between Rust types and Lua values. They are used internally by `#[lua_methods]` to convert function parameters and return values.

## Trait Definitions

```rust
/// Convert a Lua value to a Rust type
pub trait FromLua: Sized {
    fn from_lua(value: LuaValue, state: &LuaState) -> Result<Self, String>;
}

/// Convert a Rust type to Lua (pushes onto the stack)
pub trait IntoLua {
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String>;
}
```

## Built-in Implementations

### FromLua (Lua → Rust)

| Rust Type | Lua Source | Notes |
|----------|-----------|-------|
| `LuaValue` | any | Passthrough, no conversion |
| `()` | any | Always succeeds, discards the value |
| `bool` | boolean | Extracted via `as_boolean()` |
| `i8`, `i16`, `i32`, `i64`, `isize` | integer / number | Truncated to integer |
| `u8`, `u16`, `u32`, `u64`, `usize` | integer / number | Truncated to integer |
| `f32`, `f64` | number / integer | Converted to float |
| `String` | string | Cloned from Lua string |
| `Option<T>` | nil / value | `nil` → `None`, otherwise `Some(T::from_lua(...))` |

### IntoLua (Rust → Lua)

| Rust Type | Lua Result | Notes |
|----------|-----------|-------|
| `LuaValue` | any | Pushed directly |
| `()` | *(nothing)* | Returns 0 values |
| `bool` | boolean | `LuaValue::boolean(v)` |
| `i8`..`i64`, `u8`..`u64`, `isize`, `usize` | integer | `LuaValue::integer(v as i64)` |
| `f32`, `f64` | number | `LuaValue::float(v as f64)` |
| `String` | string | Created via `create_string` |
| `&str` | string | Created via `create_string` |
| `Option<T>` | nil / value | `None` → nil, `Some(v)` → `v.into_lua()` |
| `Result<T, E>` | value / error | `Ok(v)` → `v.into_lua()`, `Err(e)` → Lua error |
| `Vec<T>` | *(multiple values)* | Each element pushed; returns `vec.len()` |

## Usage in #[lua_methods]

The `#[lua_methods]` macro automatically uses `FromLua` for parameters and `IntoLua` for return values. You don't need to call these traits manually:

```rust
#[lua_methods]
impl Player {
    // Parameters: String (FromLua), i64 (FromLua)
    // Return: String (IntoLua)
    pub fn greet(&self, name: String, times: i64) -> String {
        format!("Hello {}, you visited {} times!", name, times)
    }

    // Option<T> parameter — nil or missing arg → None
    pub fn set_title(&mut self, title: Option<String>) {
        self.title = title.unwrap_or_else(|| "Untitled".to_string());
    }

    // Result<T, E> return — Err triggers a Lua error
    pub fn parse_level(&self, s: String) -> Result<i64, String> {
        s.parse::<i64>().map_err(|e| format!("invalid level: {}", e))
    }
}
```

## Manual Usage

You can also use `FromLua` / `IntoLua` directly in CFunctions or RClosures:

```rust
use luars::{FromLua, IntoLua};

fn my_func(state: &mut LuaState) -> LuaResult<usize> {
    // Extract arguments using FromLua
    let name = state.get_arg(1)
        .map(|v| String::from_lua(v, state))
        .transpose()
        .map_err(|e| state.error(e))?
        .unwrap_or_default();

    let count = state.get_arg(2)
        .map(|v| i64::from_lua(v, state))
        .transpose()
        .map_err(|e| state.error(e))?
        .unwrap_or(1);

    // Return using IntoLua
    let result = format!("{} x{}", name, count);
    let n = result.into_lua(state).map_err(|e| state.error(e))?;
    Ok(n)
}
```

## Implementing for Custom Types

You can implement `FromLua` / `IntoLua` for your own types:

```rust
use luars::{FromLua, IntoLua, LuaValue};
use luars::lua_vm::LuaState;

struct Color {
    r: u8,
    g: u8,
    b: u8,
}

impl FromLua for Color {
    fn from_lua(value: LuaValue, _state: &LuaState) -> Result<Self, String> {
        // Expect an integer encoding: 0xRRGGBB
        let n = value.as_integer().ok_or("expected integer for Color")?;
        Ok(Color {
            r: ((n >> 16) & 0xFF) as u8,
            g: ((n >> 8) & 0xFF) as u8,
            b: (n & 0xFF) as u8,
        })
    }
}

impl IntoLua for Color {
    fn into_lua(self, state: &mut LuaState) -> Result<usize, String> {
        let n = ((self.r as i64) << 16) | ((self.g as i64) << 8) | (self.b as i64);
        state.push_value(LuaValue::integer(n))
            .map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}
```

## Next

- [Error Handling](06-ErrorHandling.md) — `LuaError`, `pcall`, `xpcall`
- [API Reference](07-APIReference.md) — complete public method listing
