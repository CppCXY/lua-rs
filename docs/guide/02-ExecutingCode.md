# Executing Code

This guide covers the different ways to compile and execute Lua code.

## execute

The simplest approach — compile and run a Lua source string in one call:

```rust
let results = vm.execute(r#"
    return 1 + 2, "hello"
"#)?;
```

Also available on `LuaState`:

```rust
let state = vm.main_state();
let results = state.execute(r#"
    return "from state"
"#)?;
```

Returns `LuaResult<Vec<LuaValue>>` — the values returned by the Lua chunk.

## load / load_with_name

Compile source code into a callable function value **without executing it**:

```rust
let func = vm.load("return 1 + 1")?;
// func is a LuaValue (function) — call it later
let results = vm.call(func, vec![])?;
```

Give the chunk a name for better error messages:

```rust
let func = vm.load_with_name("return 42", "my_script.lua")?;
```

Also available on `LuaState`:

```rust
let func = state.load("return 1 + 1")?;
```

## dofile

Read, compile, and execute a Lua file from disk:

```rust
let results = vm.dofile("scripts/init.lua")?;
```

Also available on `LuaState`:

```rust
let results = state.dofile("scripts/config.lua")?;
```

## call / call_global

Call a Lua function from Rust:

```rust
// Prepare a function value
vm.execute("function add(a, b) return a + b end")?;

// Look up a global by name and call it
let results = vm.call_global("add", vec![
    LuaValue::integer(3),
    LuaValue::integer(4),
])?;
assert_eq!(results[0].as_integer(), Some(7));

// Or call an arbitrary function value
let func = vm.get_global("add")?.unwrap();
let results = vm.call(func, vec![
    LuaValue::integer(10),
    LuaValue::integer(20),
])?;
```

Also available on `LuaState` (as `call_function` / `call_global`):

```rust
let results = state.call_global("add", vec![LuaValue::integer(1), LuaValue::integer(2)])?;
```

## Compilation (Two Steps)

For repeated execution of the same code, compile once and execute multiple times:

```rust
use std::rc::Rc;

// Step 1: Compile to a Chunk
let chunk = vm.compile(r#"
    local x = ...    -- varargs become chunk arguments
    return x * x
"#)?;

let chunk = Rc::new(chunk);

// Step 2: Execute the compiled chunk
let results = vm.execute_chunk(chunk.clone())?;
```

### compile_with_name

Give the chunk a name for better error messages:

```rust
let chunk = vm.compile_with_name(
    "return 42",
    "my_script.lua"  // appears in error tracebacks
)?;
```

## Return Values

Lua chunks can return multiple values. The result is always `Vec<LuaValue>`:

```rust
let results = vm.execute("return 1, 'two', true, nil")?;

assert_eq!(results.len(), 4);
assert_eq!(results[0].as_integer(), Some(1));
assert_eq!(results[1].as_str(), Some("two"));
assert_eq!(results[2].as_boolean(), Some(true));
assert!(results[3].is_nil());
```

If the chunk doesn't return anything, the result is an empty vector:

```rust
let results = vm.execute("print('hello')")?;
assert!(results.is_empty());
```

## LuaValue Inspection

`LuaValue` represents any Lua value. Use these methods to inspect and extract:

### Type Checking

```rust
value.is_nil()
value.is_boolean()
value.is_integer()
value.is_number()       // float
value.is_string()
value.is_table()
value.is_function()     // any function type
value.is_userdata()
```

### Value Extraction

```rust
value.as_boolean()  -> Option<bool>
value.as_integer()  -> Option<i64>
value.as_number()   -> Option<f64>    // also converts integers to f64
value.as_str()      -> Option<&str>
```

### Constructing LuaValue

```rust
LuaValue::nil()
LuaValue::boolean(true)
LuaValue::integer(42)
LuaValue::float(3.14)
LuaValue::cfunction(my_fn)  // from a fn(&mut LuaState) -> LuaResult<usize>
```

> **Note:** Strings, tables, and userdata are GC-managed and must be created through `vm.create_string("hello")`, `vm.create_table(0, 0)`, etc. See [Working with Values](03-WorkingWithValues.md).

## Error Handling

`execute` returns `LuaResult<Vec<LuaValue>>`. Errors include compilation errors and runtime errors:

```rust
match vm.execute("invalid lua {{{{") {
    Ok(results) => println!("Success: {} values", results.len()),
    Err(e) => {
        let msg = vm.get_error_message(e);
        eprintln!("Error: {}", msg);
    }
}
```

For richer errors, use `into_full_error`:

```rust
match vm.execute("error('boom')") {
    Ok(_) => {}
    Err(e) => {
        let full = vm.into_full_error(e);
        eprintln!("{}", full);  // includes source location
    }
}
```

See [Error Handling](06-ErrorHandling.md) for more details.

## Next

- [Working with Values](03-WorkingWithValues.md) — globals, tables, strings
- [Rust Functions in Lua](04-RustFunctions.md) — register Rust functions callable from Lua
