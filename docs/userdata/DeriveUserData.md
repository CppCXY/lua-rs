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
| `BitAnd` | `__band` | `obj1 & obj2` |
| `BitOr` | `__bor` | `obj1 \| obj2` |
| `BitXor` | `__bxor` | `obj1 ~ obj2` |
| `Shl` | `__shl` | `obj1 << n` |
| `Shr` | `__shr` | `obj1 >> n` |
| `Neg` | `__unm` | `-obj` |
| `Not` | `__bnot` | `~obj` |

**Binary arithmetic / bitwise operators (`Add` through `Shr`):**
- The struct must derive `Clone` (needed for the operation — originals are not consumed)
- You must implement the corresponding `std::ops` trait (e.g. `impl Add for MyType`)
- List the trait name in `#[lua_impl(...)]`
- Most operators work on same-type operands (e.g. `Vec2 + Vec2`)
- **`Shl`/`Shr` are special:** the right-hand side is an integer from Lua, not a userdata. Implement `impl Shl<i64> for MyType` (or `Shl<isize>`).

**Unary operators (`Neg`, `Not`):**
- `Neg` → `__unm` (-obj), `Not` → `__bnot` (~obj)
- Require `Clone` and the corresponding `std::ops` trait impl.

**Not auto-derived (implement manually):**
- `__pow` (exponentiation) — Rust has no `Pow` trait
- `__idiv` (floor division) — semantics differ from Rust's `Div`
- `__concat` — implement `lua_concat()` manually
- `__close` — see [lua_close lifetime](#lua_close) below

### Example: Object with Arithmetic + Bitwise

```rust
#[derive(LuaUserData, Clone, PartialEq)]
#[lua_impl(Display, PartialEq, Add, BitAnd, Not)]
struct Flags {
    pub bits: u32,
}

impl std::fmt::Display for Flags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Flags(0x{:08X})", self.bits)
    }
}

impl std::ops::Add for Flags {
    type Output = Flags;
    fn add(self, rhs: Flags) -> Flags {
        Flags { bits: self.bits | rhs.bits }
    }
}

impl std::ops::BitAnd for Flags {
    type Output = Flags;
    fn bitand(self, rhs: Flags) -> Flags {
        Flags { bits: self.bits & rhs.bits }
    }
}

impl std::ops::Not for Flags {
    type Output = Flags;
    fn not(self) -> Flags {
        Flags { bits: !self.bits }
    }
}
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

## lua_close — to-be-closed variables {#lua_close}

Lua 5.4+ supports to-be-closed variables via the `<close>` annotation:

```lua
local f <close> = io.open("data.txt", "r")
-- f:close() is called automatically when f goes out of scope
```

For userdata to work with `<close>`, use `#[lua(close = "method_name")]`:

```rust
#[derive(LuaUserData)]
#[lua(close = "shutdown")]
struct Connection {
    pub host: String,
}

impl Connection {
    fn shutdown(&mut self) {
        // cleanup — called when <close> variable leaves scope
    }
}
```

The derive macro auto-generates:
```rust
fn lua_close(&mut self) { self.shutdown(); }
```

This works end-to-end in Lua:
```lua
local conn = Connection.new("db.example.com")
do
    local c <close> = conn   -- marked as to-be-closed
end                          -- c leaves scope → lua_close() → shutdown()
print(conn:is_closed())      -- true
```

> **About `__gc` / `lua_gc`:** This trait method was removed in favor of Rust's standard `Drop` trait. For cleanup on garbage collection, implement `Drop` instead.

## Delegated Metamethods {#delegated}

For metamethods that cannot be auto-derived from a standard Rust trait, use `#[lua(key = "method")]` to delegate to an existing method:

| Attribute | Generates | Method signature to implement |
|----------|-----------|------------------------------|
| `#[lua(close = "f")]` | `fn lua_close(&mut self) { self.f(); }` | `fn f(&mut self)` |
| `#[lua(pow = "f")]` | `fn lua_pow(&self, o: &UdValue) -> Option<UdValue> { Some(self.f(o)) }` | `fn f(&self, o: &UdValue) -> UdValue` |
| `#[lua(idiv = "f")]` | `fn lua_idiv(&self, o: &UdValue) -> Option<UdValue> { Some(self.f(o)) }` | `fn f(&self, o: &UdValue) -> UdValue` |
| `#[lua(concat = "f")]` | `fn lua_concat(&self, o: &UdValue) -> Option<UdValue> { Some(self.f(o)) }` | `fn f(&self, o: &UdValue) -> UdValue` |

Multiple delegations can be combined:

```rust
#[derive(LuaUserData)]
#[lua(close = "close", pow = "pow", concat = "concat_with")]
struct BigNum { ... }
```

> **Why delegation?** `__pow` maps to no standard Rust trait (Rust uses `pow()` method, not `Pow` trait). `__idiv` (floor division) has different semantics from Rust's truncating `/`. `__concat` is Lua-specific. Rather than requiring a manual `impl UserDataTrait` block, delegation lets you wire them up with a single attribute.

## Next

- [#[lua_methods]](LuaMethods.md) — defining methods and constructors
- [Type Conversions](TypeConversions.md) — detailed conversion rules for parameters and return values
