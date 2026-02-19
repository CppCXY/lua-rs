# #[lua_methods]

The `#[lua_methods]` attribute macro is placed on an `impl` block to automatically wrap `pub fn` items as Lua-callable functions.

It handles two kinds of functions:

| Function Type | Signature | Lua Call Syntax | Generated Into |
|--------------|-----------|-----------------|----------------|
| **Instance method** | `&self` / `&mut self` | `obj:method(args)` | `__lua_lookup_method()` |
| **Associated function** | No `self` (e.g. constructor) | `Type.func(args)` | `__lua_static_methods()` |

## Instance Methods

Methods with `&self` or `&mut self` become instance methods:

```rust
#[lua_methods]
impl Point {
    /// &self — read-only method
    pub fn distance(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// &mut self — mutating method
    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
    }
}
```

Called in Lua with `:` syntax:

```lua
local d = p:distance()      -- calls &self method
p:translate(10, 20)          -- calls &mut self method
```

You can also use `.` syntax to get a method reference (CFunction):

```lua
local f = p.distance         -- type(f) == "function"
print(f(p))                  -- equivalent to p:distance()
```

## Associated Functions (Constructors)

`pub fn` items without a `self` parameter become associated functions, typically used as constructors:

```rust
#[lua_methods]
impl Point {
    /// Constructor — no self parameter, returns Self
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    /// Another constructor
    pub fn origin() -> Self {
        Point { x: 0.0, y: 0.0 }
    }
}
```

After registration via `register_type`, call them in Lua with `.` syntax:

```lua
local p = Point.new(3, 4)    -- calls the associated function
local o = Point.origin()     -- another constructor
```

### Self Return Value

When an associated function returns `Self`, the macro automatically:
1. Calls the Rust function to get a struct instance
2. Wraps it in `LuaUserdata::new(result)`
3. Allocates it via `create_userdata` (GC-managed)
4. Pushes it onto the Lua stack as a userdata return value

The Lua side receives a full userdata object with field access, method calls, and metamethods.

## Visibility Rules

| Condition | Wrapper Generated? |
|-----------|-------------------|
| `pub fn method(&self, ...)` | ✅ Instance method |
| `pub fn method(&mut self, ...)` | ✅ Mutable instance method |
| `pub fn func(args...) -> ...` | ✅ Associated function |
| `fn private_method(&self, ...)` | ❌ Skipped (not `pub`) |
| `pub async fn ...` | ❌ Skipped (async not supported) |

**Only `pub` functions are processed.** Private methods are never exposed to Lua.

## Supported Parameter Types

```rust
#[lua_methods]
impl MyType {
    pub fn example(
        &self,
        a: i64,                  // integer
        b: f64,                  // float
        c: bool,                 // boolean
        d: String,               // string
        e: Option<String>,       // optional (nil → None)
    ) -> f64 {
        // ...
    }
}
```

See [Type Conversions](TypeConversions.md) for the full conversion reference.

## Supported Return Types

```rust
#[lua_methods]
impl MyType {
    // No return value
    pub fn action(&mut self) { /* ... */ }

    // Basic type
    pub fn get_value(&self) -> f64 { 42.0 }

    // String
    pub fn get_name(&self) -> String { "hello".into() }

    // Option<T> — None becomes nil
    pub fn find(&self, key: String) -> Option<i64> { None }

    // Result<T, E> — Err triggers a Lua error
    pub fn divide(&self, d: f64) -> Result<f64, String> {
        if d == 0.0 { Err("div by zero".into()) }
        else { Ok(self.value / d) }
    }

    // Self — wrapped as userdata (associated functions only)
    pub fn new(x: f64) -> Self { MyType { value: x } }
}
```

## Multiple impl Blocks

`#[lua_methods]` should only be used on **one** impl block. If multiple impl blocks use this attribute, the later one's `__lua_lookup_method` and `__lua_static_methods` will shadow the former (since they are same-name inherent methods).

```rust
// ✅ Recommended: put all methods in a single #[lua_methods] block
#[lua_methods]
impl Point {
    pub fn new(x: f64, y: f64) -> Self { /* ... */ }
    pub fn distance(&self) -> f64 { /* ... */ }
    pub fn translate(&mut self, dx: f64, dy: f64) { /* ... */ }
}
```

## Generated Code

For the `Point` example above, `#[lua_methods]` roughly generates:

```rust
// Original impl block is preserved
impl Point {
    pub fn new(x: f64, y: f64) -> Self { /* ... */ }
    pub fn distance(&self) -> f64 { /* ... */ }
    pub fn translate(&mut self, dx: f64, dy: f64) { /* ... */ }
}

// Additional generated impl block
impl Point {
    /// Instance method lookup
    pub fn __lua_lookup_method(key: &str) -> Option<CFunction> {
        // Internally defines __lua_method_distance, __lua_method_translate, etc.
        match key {
            "distance" => Some(__lua_method_distance),
            "translate" => Some(__lua_method_translate),
            _ => None,
        }
    }

    /// Associated function (static method) list
    pub fn __lua_static_methods() -> &'static [(&'static str, CFunction)] {
        // Internally defines __lua_static_new wrapper
        &[("new", __lua_static_new)]
    }
}
```

> **Runnable example:** See `Vec2` in [`examples/luars-example/src/main.rs`](../../examples/luars-example/src/main.rs) — `example_vec2()`

## Next

- [register_type](RegisterType.md) — how to register associated functions in the Lua global table
- [Type Conversions](TypeConversions.md) — detailed conversion rules for parameters and return values
