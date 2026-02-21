# Error Handling

This guide covers how errors work in luars — from Rust-side error types to Lua-side `pcall`/`xpcall`.

## LuaError

All fallible operations return `LuaResult<T>`, which is `Result<T, LuaError>`.

`LuaError` is a 1-byte enum representing different error categories:

```rust
pub enum LuaError {
    RuntimeError,           // general runtime error
    CompileError,           // syntax / compilation error
    Yield,                  // coroutine yield (internal)
    StackOverflow,          // call stack overflow
    OutOfMemory,            // memory allocation failure
    IndexOutOfBounds,       // stack index out of range
    Exit,                   // top-level return (internal)
    CloseThread,            // coroutine self-close (internal)
    ErrorInErrorHandling,   // error inside error handler
}
```

The actual error message is stored inside the VM, not in the enum itself — this keeps `LuaError` just 1 byte and `Result<T, LuaError>` very cheap.

### Getting the Error Message

Error messages are stored internally. Use `get_error_message` to retrieve them:

```rust
match vm.execute("error('something went wrong')") {
    Ok(_) => println!("Success"),
    Err(e) => {
        let msg = vm.get_error_message(e);
        eprintln!("Lua error: {}", msg);
    }
}
```

On `LuaState`:

```rust
match state.execute("invalid code %%%") {
    Ok(_) => {},
    Err(e) => {
        let msg = state.get_error_msg(e);
        eprintln!("Error: {}", msg);
    }
}
```

### last_error_msg

Access the last error message without consuming the error:

```rust
let msg = state.last_error_msg();  // &str
```

## LuaFullError

For contexts where you need a proper `std::error::Error` with the message included (e.g., `anyhow`, `?` operator), use `vm.into_full_error()`:

```rust
let result = vm.execute("error('boom')")
    .map_err(|e| vm.into_full_error(e))?;
```

`LuaFullError` has two public fields:

```rust
pub struct LuaFullError {
    pub kind: LuaError,     // the error variant
    pub message: String,    // human-readable message with source location
}
```

It implements `Display` and `std::error::Error`, printing the full message when displayed.

## Raising Errors from Rust

### In a CFunction / RClosure

Use `state.error()` to create a Lua error:

```rust
fn checked_divide(state: &mut LuaState) -> LuaResult<usize> {
    let a = state.get_arg(1).and_then(|v| v.as_number()).unwrap_or(0.0);
    let b = state.get_arg(2).and_then(|v| v.as_number()).unwrap_or(0.0);

    if b == 0.0 {
        return Err(state.error("division by zero".to_string()));
    }

    state.push_value(LuaValue::float(a / b))?;
    Ok(1)
}
```

### In #[lua_methods] — Result Return

Methods that return `Result<T, E>` automatically convert `Err` to a Lua error:

```rust
#[lua_methods]
impl Calculator {
    pub fn divide(&self, divisor: f64) -> Result<f64, String> {
        if divisor == 0.0 {
            Err("cannot divide by zero".into())
        } else {
            Ok(self.value / divisor)
        }
    }
}
```

The `E` type must implement `Display`.

## Catching Errors in Lua

### pcall

```lua
local ok, result = pcall(function()
    return risky_function()
end)

if ok then
    print("Result:", result)
else
    print("Error:", result)
end
```

### xpcall

`xpcall` adds a custom error handler that receives the error before the stack unwinds:

```lua
local ok, result = xpcall(
    function()
        error("oops")
    end,
    function(err)
        return "Handled: " .. tostring(err)
    end
)
-- ok = false, result = "Handled: ...oops"
```

## Catching Errors from Rust

### pcall from Rust

```rust
let func = vm.get_global("risky_func")?.unwrap();
let state = vm.main_state();
let (ok, results) = state.pcall(func, vec![])?;

if ok {
    println!("Success: {} values", results.len());
} else {
    // results[0] contains the error value
    let err_msg = state.to_string(&results[0])?;
    println!("Error: {}", err_msg);
}
```

### xpcall from Rust

```rust
let func = vm.get_global("risky_func")?.unwrap();
let handler = vm.get_global("my_handler")?.unwrap();
let state = vm.main_state();
let (ok, results) = state.xpcall(func, vec![], handler)?;
```

## Error Flow Summary

```
Rust → Lua error:
  CFunction:     return Err(state.error("msg"))
  #[lua_methods]: return Err("msg".into())  (via Result<T, E>)

Lua → Rust error catch:
  state.pcall(func, args)    → (bool, Vec<LuaValue>)
  state.xpcall(func, args, handler) → (bool, Vec<LuaValue>)

Lua error propagation:
  vm.execute(...)            → LuaResult<Vec<LuaValue>>
  state.call(func, args)     → LuaResult<Vec<LuaValue>> (propagates)

Rich error:
  vm.into_full_error(e)      → LuaFullError (impl std::error::Error)
```

## Next

- [API Reference](07-APIReference.md) — complete public method listing
