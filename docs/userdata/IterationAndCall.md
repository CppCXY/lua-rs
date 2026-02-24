# Iteration & Callable Userdata

This document covers two advanced userdata features:

1. **`#[lua(iter)]`** — make a `Vec<T>` field iterable via `pairs()` in Lua
2. **`lua_call`** — make a userdata callable like a function

---

## Iteration: `#[lua(iter)]`

Mark a `Vec<T>` field with `#[lua(iter)]` to make the struct iterable via Lua's `pairs()`.

### Basic Usage

```rust
#[derive(LuaUserData)]
struct Inventory {
    pub owner: String,
    #[lua(iter)]
    items: Vec<i64>,   // private is OK — #[lua(iter)] doesn't expose the field itself
}
```

In Lua:

```lua
for i, item in pairs(inventory) do
    print(i, item)   -- 1-based index, element value
end

print(#inventory)    -- length operator also works (auto-generated)
```

### What gets generated

`#[lua(iter)]` on a `Vec<T>` field auto-generates two trait methods:

- **`lua_next(control) → Option<(UdValue, UdValue)>`** — stateless iterator using integer index as control variable
- **`lua_len() → Option<UdValue>`** — returns `self.field.len()` for the `#` operator

The iterator follows Lua's generic-for protocol:
- Control starts at `Nil` (mapped to index 0)
- Each call returns `(index + 1, element)` — 1-based indices like Lua tables
- Returns `None` when the index exceeds the Vec length

### Supported element types

| Vec Element Type | Lua Value |
|---------|-----------|
| `i8`..`i64`, `u8`..`u64`, `isize`, `usize` | integer |
| `f32`, `f64` | number |
| `bool` | boolean |
| `String` | string |
| Custom types implementing `Into<UdValue>` | depends on conversion |

### Rules

- Only **one** field per struct can have `#[lua(iter)]`
- The field must be `Vec<T>`
- The field does **not** need to be `pub` — `#[lua(iter)]` works on private fields
- `#[lua(iter)]` can coexist with regular field exposure (`pub` fields are still accessible)
- If the Vec is empty, `pairs()` returns immediately with no iterations

### Example: String list

```rust
#[derive(LuaUserData)]
struct Tags {
    #[lua(iter)]
    tags: Vec<String>,
}
```

```lua
for _, tag in pairs(my_tags) do
    print(tag)    -- prints each tag string
end
```

### Manual implementation

If you need custom iteration logic (e.g., iterating a `HashMap`, a tree, or filtered elements), implement `lua_next` directly on your `UserDataTrait`:

```rust
impl UserDataTrait for MyCollection {
    fn lua_next(&self, control: &UdValue) -> Option<(UdValue, UdValue)> {
        let idx = match control {
            UdValue::Nil => 0,
            UdValue::Integer(i) => *i as usize,
            _ => return None,
        };
        // Your custom iteration logic
        self.get_item(idx).map(|(key, val)| (
            UdValue::Integer((idx + 1) as i64),
            UdValue::Str(val.to_string()),
        ))
    }

    // ...
}
```

The contract:
- `control` starts as `UdValue::Nil` (first call)
- Return `Some((next_control, value))` to yield an element
- Return `None` to stop iteration
- The `next_control` value is passed back as `control` on the next call

---

## Callable Userdata: `lua_call`

Make a userdata callable from Lua by implementing `lua_call()` → `Option<CFunction>`.

### Basic Usage

```rust
struct Multiplier {
    factor: i64,
}

impl UserDataTrait for Multiplier {
    fn type_name(&self) -> &'static str { "Multiplier" }

    fn lua_call(&self) -> Option<CFunction> {
        fn call_impl(l: &mut LuaState) -> LuaResult<usize> {
            // arg 1 = self (the userdata), arg 2+ = caller's arguments
            let ud = l.get_arg(1).unwrap();
            let ud_ref = ud.as_userdata_mut().unwrap();
            let mul = ud_ref.get_trait().as_any()
                .downcast_ref::<Multiplier>().unwrap();

            let val = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(0);

            l.push_value(LuaValue::integer(val * mul.factor))?;
            Ok(1)
        }
        Some(call_impl)
    }

    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
```

In Lua:

```lua
local double = Multiplier { factor = 2 }  -- assume created from Rust
print(double(21))   -- 42
print(double(5))    -- 10
```

### How it works

When Lua tries to call a userdata value:

1. The VM checks `ud.get_trait().lua_call()` first
2. If it returns `Some(cfunc)`, the call is dispatched:
   - The userdata itself becomes arg 1 (like `self` in a method call)
   - The caller's arguments follow as arg 2, 3, ...
3. If `lua_call()` returns `None`, the VM falls back to the metatable `__call` metamethod

### CFunction signature

The `CFunction` you return has the standard Lua C function signature:

```rust
fn(l: &mut LuaState) -> LuaResult<usize>
```

- **Arguments:** `l.get_arg(1)` is the userdata itself, `l.get_arg(2)` onward are the caller's args
- **Return:** Push values onto the stack and return the count

### Multiple return values

```rust
fn call_impl(l: &mut LuaState) -> LuaResult<usize> {
    let val = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(0);
    l.push_value(LuaValue::integer(val / 10))?;  // quotient
    l.push_value(LuaValue::integer(val % 10))?;  // remainder
    Ok(2)  // two return values
}
```

```lua
local q, r = divmod(47)   -- q=4, r=7
```

### Combining with other features

`lua_call` works alongside all other userdata features:

```lua
-- Field access
print(obj.name)

-- Method calls
obj:some_method()

-- Iteration
for k, v in pairs(obj) do ... end

-- Callable
local result = obj(42)
```

---

## Summary

| Feature | Attribute / Method | Effect |
|---------|-------------------|--------|
| Iteration | `#[lua(iter)]` on `Vec<T>` | Auto-generates `lua_next` + `lua_len` |
| Manual iteration | `fn lua_next(&self, control)` | Custom iteration logic |
| Length | Auto with `#[lua(iter)]` | `#obj` returns Vec length |
| Callable | `fn lua_call(&self) -> Option<CFunction>` | `obj(args...)` in Lua |

## Next

- [#[derive(LuaUserData)]](DeriveUserData.md) — field access and metamethods
- [#[lua_methods]](LuaMethods.md) — methods and constructors
- [Type Conversions](TypeConversions.md) — parameter and return type mapping
