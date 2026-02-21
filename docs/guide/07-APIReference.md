# API Reference

Quick reference of all commonly used public methods on `LuaVM`, `LuaState`, and supporting types.

## LuaVM

### Lifecycle

```rust
LuaVM::new(option: SafeOption) -> Box<Self>
vm.main_state() -> &mut LuaState
vm.open_stdlib(lib: Stdlib) -> LuaResult<()>
vm.open_stdlibs(libs: &[Stdlib]) -> LuaResult<()>
```

### Executing Code

```rust
vm.execute(source: &str) -> LuaResult<Vec<LuaValue>>
vm.execute_chunk(chunk: Rc<Chunk>) -> LuaResult<Vec<LuaValue>>
vm.load(source: &str) -> LuaResult<LuaValue>
vm.load_with_name(source: &str, chunk_name: &str) -> LuaResult<LuaValue>
vm.dofile(path: &str) -> LuaResult<Vec<LuaValue>>
vm.compile(source: &str) -> LuaResult<Chunk>
vm.compile_with_name(source: &str, chunk_name: &str) -> LuaResult<Chunk>
```

### Calling Functions

```rust
vm.call(func: LuaValue, args: Vec<LuaValue>) -> LuaResult<Vec<LuaValue>>
vm.call_global(name: &str, args: Vec<LuaValue>) -> LuaResult<Vec<LuaValue>>
```

### Globals

```rust
vm.set_global(name: &str, value: LuaValue) -> LuaResult<()>
vm.get_global(name: &str) -> LuaResult<Option<LuaValue>>
vm.get_global_as<T: FromLua>(name: &str) -> LuaResult<Option<T>>
```

### Registration

```rust
vm.register_function(name: &str, f: F) -> LuaResult<()>   // F: Fn(&mut LuaState) -> LuaResult<usize> + 'static
vm.register_type_of<T: LuaRegistrable>(name: &str) -> LuaResult<()>
vm.register_enum<T: LuaEnum>(name: &str) -> LuaResult<()>
```

### Creating Values

```rust
vm.create_string(s: &str) -> CreateResult
vm.create_table(array_size: usize, hash_size: usize) -> CreateResult
vm.create_userdata(data: LuaUserdata) -> CreateResult
vm.create_closure(func: F) -> CreateResult      // F: Fn(&mut LuaState) -> LuaResult<usize> + 'static
vm.create_closure_with_upvalues(func: F, upvalues: Vec<LuaValue>) -> CreateResult
vm.create_c_closure(func: CFunction, upvalues: Vec<LuaValue>) -> CreateResult
```

### Table Operations (raw, no metamethods)

```rust
vm.raw_get(table: &LuaValue, key: &LuaValue) -> Option<LuaValue>
vm.raw_set(table: &LuaValue, key: LuaValue, value: LuaValue) -> bool
vm.raw_geti(table: &LuaValue, key: i64) -> Option<LuaValue>
vm.raw_seti(table: &LuaValue, key: i64, value: LuaValue) -> bool
vm.table_pairs(table: &LuaValue) -> LuaResult<Vec<(LuaValue, LuaValue)>>
vm.table_length(table: &LuaValue) -> LuaResult<usize>
```

### Protected Calls

```rust
vm.protected_call(func: LuaValue, args: Vec<LuaValue>) -> LuaResult<(bool, Vec<LuaValue>)>
vm.protected_call_with_handler(func: LuaValue, args: Vec<LuaValue>, handler: LuaValue) -> LuaResult<(bool, Vec<LuaValue>)>
```

### Coroutines

```rust
vm.create_thread(func: LuaValue) -> CreateResult
vm.resume_thread(thread: LuaValue, args: Vec<LuaValue>) -> LuaResult<(bool, Vec<LuaValue>)>
```

### Registry

```rust
vm.registry_seti(key: i64, value: LuaValue)
vm.registry_geti(key: i64) -> Option<LuaValue>
vm.registry_set(key: &str, value: LuaValue) -> LuaResult<()>
vm.registry_get(key: &str) -> LuaResult<Option<LuaValue>>
```

### References

```rust
vm.create_ref(value: LuaValue) -> LuaRefValue
vm.get_ref_value(lua_ref: &LuaRefValue) -> LuaValue
vm.release_ref(lua_ref: LuaRefValue)
```

### Errors

```rust
vm.error(message: impl Into<String>) -> LuaError
vm.get_error_message(e: LuaError) -> String
vm.into_full_error(e: LuaError) -> LuaFullError
vm.generate_traceback(error_msg: &str) -> String
```

### Serde (feature = "serde")

```rust
vm.serialize_to_json(value: &LuaValue) -> Result<serde_json::Value, String>
vm.serialize_to_json_string(value: &LuaValue, pretty: bool) -> Result<String, String>
vm.deserialize_from_json(json: &serde_json::Value) -> Result<LuaValue, String>
vm.deserialize_from_json_string(json_str: &str) -> Result<LuaValue, String>
```

### Async

```rust
vm.register_async(name: &str, f: F) -> LuaResult<()>
vm.execute_async(source: &str) -> impl Future<Output = LuaResult<Vec<LuaValue>>>
vm.call_async(func: LuaValue, args: Vec<LuaValue>) -> impl Future<Output = LuaResult<Vec<LuaValue>>>
vm.call_async_global(name: &str, args: Vec<LuaValue>) -> impl Future<Output = LuaResult<Vec<LuaValue>>>
vm.create_async_thread(func: LuaValue) -> LuaResult<AsyncThread>
vm.create_async_call_handle(name: &str) -> LuaResult<AsyncCallHandle>
```

---

## LuaState

> `LuaState` is the per-coroutine execution context. Inside a `CFunction`, it is the `&mut LuaState` parameter. From the host, access it via `vm.main_state()`.

### Executing Code

```rust
state.execute(source: &str) -> LuaResult<Vec<LuaValue>>
state.load(source: &str) -> LuaResult<LuaValue>
state.load_with_name(source: &str, chunk_name: &str) -> LuaResult<LuaValue>
state.dofile(path: &str) -> LuaResult<Vec<LuaValue>>
```

### Calling Functions

```rust
state.call(func: LuaValue, args: Vec<LuaValue>) -> LuaResult<Vec<LuaValue>>
state.call_function(func: LuaValue, args: Vec<LuaValue>) -> LuaResult<Vec<LuaValue>>
state.call_global(name: &str, args: Vec<LuaValue>) -> LuaResult<Vec<LuaValue>>
state.pcall(func: LuaValue, args: Vec<LuaValue>) -> LuaResult<(bool, Vec<LuaValue>)>
state.xpcall(func: LuaValue, args: Vec<LuaValue>, handler: LuaValue) -> LuaResult<(bool, Vec<LuaValue>)>
```

### Globals

```rust
state.set_global(name: &str, value: LuaValue) -> LuaResult<()>
state.get_global(name: &str) -> LuaResult<Option<LuaValue>>
```

### Registration

```rust
state.register_function(name: &str, f: F) -> LuaResult<()>
state.register_type(name: &str, static_methods: &[(&str, CFunction)]) -> LuaResult<()>
state.register_type_of<T: LuaRegistrable>(name: &str) -> LuaResult<()>
```

### Creating Values

```rust
state.create_table(narr: usize, nrec: usize) -> CreateResult
state.create_string(s: &str) -> CreateResult
state.create_userdata(data: LuaUserdata) -> CreateResult
state.create_closure(func: F) -> CreateResult
state.create_closure_with_upvalues(func: F, upvalues: Vec<LuaValue>) -> CreateResult
```

### Arguments (inside CFunction / RClosure)

```rust
state.arg_count() -> usize
state.get_arg(index: usize) -> Option<LuaValue>   // 1-based
state.get_args() -> Vec<LuaValue>
```

### Stack Operations

```rust
state.push_value(value: LuaValue) -> LuaResult<()>
state.get_top() -> usize
```

### Table Operations

```rust
// Raw (no metamethods)
state.raw_get(table: &LuaValue, key: &LuaValue) -> Option<LuaValue>
state.raw_set(table: &LuaValue, key: LuaValue, value: LuaValue) -> bool
state.raw_geti(table: &LuaValue, index: i64) -> Option<LuaValue>
state.raw_seti(table: &LuaValue, index: i64, value: LuaValue) -> bool

// With metamethods
state.table_get(table: &LuaValue, key: &LuaValue) -> LuaResult<Option<LuaValue>>
state.table_set(table: &LuaValue, key: LuaValue, value: LuaValue) -> LuaResult<()>
```

### Errors

```rust
state.error(msg: String) -> LuaError
state.last_error_msg() -> &str
state.get_error_msg(e: LuaError) -> String
```

### Misc

```rust
state.to_string(value: &LuaValue) -> LuaResult<String>
state.collect_garbage() -> LuaResult<()>
state.is_main_thread() -> bool
```

---

## TableBuilder

Fluent builder for constructing Lua tables from Rust.

```rust
use luars::TableBuilder;

let table = TableBuilder::new()
    .set("key", vm.create_string("value")?)     // hash part: t["key"] = "value"
    .set_int(1, LuaValue::integer(100))          // array part: t[1] = 100
    .set_value(LuaValue::boolean(true), LuaValue::integer(1))  // arbitrary key
    .push(LuaValue::integer(42))                 // auto-increment array index
    .build(&mut vm)?;
```

### Methods

```rust
TableBuilder::new() -> Self
builder.set(key: &str, value: LuaValue) -> Self       // string key
builder.set_int(key: i64, value: LuaValue) -> Self     // integer key
builder.set_value(key: LuaValue, value: LuaValue) -> Self  // any LuaValue key
builder.push(value: LuaValue) -> Self                  // auto-incrementing array push
builder.build(vm: &mut LuaVM) -> CreateResult          // materializes the table
```

---

## Key Types

### LuaValue

```rust
// Construction
LuaValue::nil()
LuaValue::boolean(v: bool)
LuaValue::integer(v: i64)
LuaValue::float(v: f64)
LuaValue::cfunction(f: CFunction)

// Type checking
value.is_nil() -> bool
value.is_boolean() -> bool
value.is_integer() -> bool
value.is_number() -> bool
value.is_string() -> bool
value.is_table() -> bool
value.is_function() -> bool
value.is_userdata() -> bool

// Extraction
value.as_boolean() -> Option<bool>
value.as_integer() -> Option<i64>
value.as_number() -> Option<f64>
value.as_str() -> Option<&str>
```

### LuaError

A 1-byte enum. The actual error message is stored inside the VM.

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

Implements `Display` and `std::error::Error`.

### LuaFullError

Rich error combining the `LuaError` variant with the error message. Created via `vm.into_full_error(e)`.

```rust
pub struct LuaFullError {
    pub kind: LuaError,     // the error variant
    pub message: String,    // human-readable message with source location
}
```

Implements `Display` and `std::error::Error`. Works with `anyhow`, `thiserror`, and `?`.

```rust
let result = vm.execute("bad code")
    .map_err(|e| vm.into_full_error(e))?;
```

### SafeOption

```rust
SafeOption {
    max_call_depth: usize,        // default: 200
    max_stack_size: usize,        // default: 1_000_000
    max_gc_memory: usize,         // default: 512 MB
    max_instruction_count: usize, // default: 0 (unlimited)
}
```

### Stdlib

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stdlib {
    Basic, String, Table, Math, IO, OS,
    Coroutine, Utf8, Package, Debug, All,
}
```

### AsyncReturnValue

Return type for async Rust functions registered with `register_async`.

```rust
pub enum AsyncReturnValue {
    Nil,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Str(String),
    UserData(Box<dyn Any + Send + Sync>),
    Table(Vec<(AsyncReturnValue, AsyncReturnValue)>),
}
```

Convenience constructors: `string(s)`, `integer(n)`, `float(n)`, `boolean(b)`, `nil()`, `table(pairs)`.

### Type Aliases

```rust
type CFunction = fn(&mut LuaState) -> LuaResult<usize>;
type RustCallback = Box<dyn Fn(&mut LuaState) -> LuaResult<usize>>;
type CreateResult = LuaResult<LuaValue>;
type LuaResult<T> = Result<T, LuaError>;
```
