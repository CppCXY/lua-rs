# Working with Values

This guide covers how to create, read, and manipulate Lua values from Rust — including globals, strings, and tables.

## Globals

### Setting Globals

Expose Rust values to Lua by setting global variables:

```rust
// Simple values
vm.set_global("MAX_HEALTH", LuaValue::integer(100))?;
vm.set_global("PI", LuaValue::float(3.14159))?;
vm.set_global("DEBUG", LuaValue::boolean(true))?;

// Strings (GC-managed, must be created first)
let name = vm.create_string("world")?;
vm.set_global("PLAYER_NAME", name)?;

// Now accessible in Lua
vm.execute_string(r#"
    print(MAX_HEALTH)    -- 100
    print(PLAYER_NAME)   -- world
"#)?;
```

`set_global` is also available on `LuaState`:

```rust
let state = vm.main_state();
state.set_global("X", LuaValue::integer(42))?;
```

### Getting Globals

Read values set by Lua code:

```rust
vm.execute_string("MY_VAR = 123")?;

let val = vm.get_global("MY_VAR")?;
if let Some(v) = val {
    println!("MY_VAR = {:?}", v.as_integer()); // Some(123)
}
```

## Strings

Lua strings are GC-managed and interned. Create them through the VM:

```rust
let s = vm.create_string("hello")?;
assert!(s.is_string());
assert_eq!(s.as_str(), Some("hello"));

// Set as a global
vm.set_global("greeting", s)?;
```

> **Note:** luars strings are always valid UTF-8. This differs from standard Lua which allows arbitrary bytes.

## Tables

Tables are Lua's primary data structure — used as arrays, dictionaries, and objects.

### Creating Tables

```rust
// create_table(array_capacity, hash_capacity)
let t = vm.create_table(0, 2)?;
```

### Setting Fields (raw_set)

`raw_set` sets a key-value pair without triggering metamethods:

```rust
let t = vm.create_table(0, 2)?;

// String keys
let key = vm.create_string("name")?;
let val = vm.create_string("Alice")?;
vm.raw_set(&t, key, val);

// Integer keys (array-style)
vm.raw_seti(&t, 1, LuaValue::integer(10));
vm.raw_seti(&t, 2, LuaValue::integer(20));

vm.set_global("player", t)?;
```

```lua
-- In Lua:
print(player.name)  -- "Alice"
print(player[1])    -- 10
print(player[2])    -- 20
```

### Getting Fields (raw_get)

```rust
let t = vm.get_global("player")?.unwrap();

// By string key
let key = vm.create_string("name")?;
let name = vm.raw_get(&t, &key);
println!("name = {:?}", name.and_then(|v| v.as_str().map(String::from)));

// By integer key
let first = vm.raw_geti(&t, 1);
println!("first = {:?}", first.map(|v| v.as_integer()));
```

### Table Operations on LuaState

`LuaState` provides additional table methods that support metamethods:

```rust
let state = vm.main_state();

// table_get — triggers __index metamethod
let val = state.table_get(&t, &key)?;

// table_set — triggers __newindex metamethod
state.table_set(&t, key, val)?;

// raw_get / raw_set — no metamethods (also available)
```

## Building Complex Structures

Compose tables to create nested data:

```rust
let mut vm = LuaVM::new(SafeOption::default());
vm.open_stdlib(Stdlib::All)?;

// Build: config = { window = { width = 800, height = 600 }, title = "My App" }
let window = vm.create_table(0, 2)?;
let k_width = vm.create_string("width")?;
let k_height = vm.create_string("height")?;
vm.raw_set(&window, k_width, LuaValue::integer(800));
vm.raw_set(&window, k_height, LuaValue::integer(600));

let config = vm.create_table(0, 2)?;
let k_window = vm.create_string("window")?;
let k_title = vm.create_string("title")?;
let v_title = vm.create_string("My App")?;
vm.raw_set(&config, k_window, window);
vm.raw_set(&config, k_title, v_title);

vm.set_global("config", config)?;

vm.execute_string(r#"
    print(config.title)           -- My App
    print(config.window.width)    -- 800
"#)?;
```

## UserData

UserData wraps an arbitrary Rust struct as a Lua value with field access, methods, and metamethods. See the [UserData guide](../userdata/GettingStarted.md) for full details.

Quick example:

```rust
use luars::LuaUserdata;

// Assuming Point implements UserDataTrait (via #[derive(LuaUserData)])
let point = LuaUserdata::new(Point { x: 3.0, y: 4.0 });

let state = vm.main_state();
let ud_val = state.create_userdata(point)?;
state.set_global("origin", ud_val)?;
```

```lua
print(origin.x, origin.y)  -- 3.0  4.0
```

## Next

- [Rust Functions in Lua](04-RustFunctions.md) — expose Rust logic to Lua
- [FromLua / IntoLua](05-FromLuaIntoLua.md) — automatic type conversions
