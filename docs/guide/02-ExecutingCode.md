# Executing Code

This guide covers the different ways to compile and execute Lua code.

## execute_string

The simplest approach — compile and run a Lua source string in one call:

```rust
// On LuaVM
let results = vm.execute_string(r#"
    return 1 + 2, "hello"
"#)?;

// On LuaState (same API available on the main state)
let state = vm.main_state();
let results = state.execute_string(r#"
    return "from state"
"#)?;
```

Returns `LuaResult<Vec<LuaValue>>` — the values returned by the Lua chunk.

## Compilation + Execution (Two Steps)

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
let results = vm.execute(chunk.clone())?;
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
let results = vm.execute_string("return 1, 'two', true, nil")?;

assert_eq!(results.len(), 4);
assert_eq!(results[0].as_integer(), Some(1));
assert_eq!(results[1].as_str(), Some("two"));
assert_eq!(results[2].as_boolean(), Some(true));
assert!(results[3].is_nil());
```

If the chunk doesn't return anything, the result is an empty vector:

```rust
let results = vm.execute_string("print('hello')")?;
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

`execute_string` returns `LuaResult<Vec<LuaValue>>`. Errors include compilation errors and runtime errors:

```rust
match vm.execute_string("invalid lua {{{{") {
    Ok(results) => println!("Success: {} values", results.len()),
    Err(e) => {
        let msg = vm.get_error_message(e);
        eprintln!("Error: {}", msg);
    }
}
```

See [Error Handling](06-ErrorHandling.md) for more details.

## Next

- [Working with Values](03-WorkingWithValues.md) — globals, tables, strings
- [Rust Functions in Lua](04-RustFunctions.md) — register Rust functions callable from Lua
