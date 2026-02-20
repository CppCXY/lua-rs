# API Reference

Quick reference of all commonly used public methods on `LuaVM` and `LuaState`.

## LuaVM

### Lifecycle

```rust
LuaVM::new(option: SafeOption) -> Box<Self>
vm.main_state() -> &mut LuaState
vm.open_stdlib(lib: Stdlib) -> LuaResult<()>
```

### Executing Code

```rust
vm.execute_string(source: &str) -> LuaResult<Vec<LuaValue>>
vm.execute(chunk: Rc<Chunk>) -> LuaResult<Vec<LuaValue>>
vm.compile(source: &str) -> LuaResult<Chunk>
vm.compile_with_name(source: &str, chunk_name: &str) -> LuaResult<Chunk>
```

### Globals

```rust
vm.set_global(name: &str, value: LuaValue) -> LuaResult<()>
vm.get_global(name: &str) -> LuaResult<Option<LuaValue>>
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
vm.generate_traceback(error_msg: &str) -> String
```

### Serde (feature = "serde")

```rust
vm.serialize_to_json(value: &LuaValue) -> Result<serde_json::Value, String>
vm.serialize_to_json_string(value: &LuaValue, pretty: bool) -> Result<String, String>
vm.deserialize_from_json(json: &serde_json::Value) -> Result<LuaValue, String>
vm.deserialize_from_json_string(json_str: &str) -> Result<LuaValue, String>
```

---

## LuaState

### Executing Code

```rust
state.execute_string(source: &str) -> LuaResult<Vec<LuaValue>>
state.execute(chunk: Rc<Chunk>) -> LuaResult<Vec<LuaValue>>
```

### Globals

```rust
state.set_global(name: &str, value: LuaValue) -> LuaResult<()>
state.get_global(name: &str) -> LuaResult<Option<LuaValue>>
```

### Creating Values

```rust
state.create_table(narr: usize, nrec: usize) -> CreateResult
state.create_string(s: &str) -> CreateResult
state.create_userdata(data: LuaUserdata) -> CreateResult
state.create_closure(func: F) -> CreateResult
state.create_closure_with_upvalues(func: F, upvalues: Vec<LuaValue>) -> CreateResult
```

### UserData Registration

```rust
// Register with explicit method list
state.register_type(name: &str, static_methods: &[(&str, CFunction)]) -> LuaResult<()>

// Register using LuaRegistrable trait (preferred)
state.register_type_of::<T>(name: &str) -> LuaResult<()>
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

### Calling Functions

```rust
state.call(func: LuaValue, args: Vec<LuaValue>) -> LuaResult<Vec<LuaValue>>
state.pcall(func: LuaValue, args: Vec<LuaValue>) -> LuaResult<(bool, Vec<LuaValue>)>
state.xpcall(func: LuaValue, args: Vec<LuaValue>, handler: LuaValue) -> LuaResult<(bool, Vec<LuaValue>)>
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
enum Stdlib {
    Basic, String, Table, Math, IO, OS,
    Coroutine, Utf8, Package, Debug, All,
}
```

### Type Aliases

```rust
type CFunction = fn(&mut LuaState) -> LuaResult<usize>;
type RustCallback = Box<dyn Fn(&mut LuaState) -> LuaResult<usize>>;
type CreateResult = LuaResult<LuaValue>;
type LuaResult<T> = Result<T, LuaError>;
```
