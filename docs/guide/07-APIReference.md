# API Reference

This reference reflects the current public design.

## Recommended Entry Point: `Lua`

`Lua` is the high-level host-facing runtime.

```rust
use luars::{Lua, LuaApi, SafeOption};

let mut lua = Lua::new(SafeOption::default());
```

### Common `LuaApi` Methods

```rust
lua.open_stdlib(Stdlib::All) -> LuaResult<()>
lua.collect_garbage() -> LuaResult<()>

lua.load(source) -> Chunk<'_, Lua>
lua.execute(source) -> LuaResult<()>
lua.eval::<T>(source) -> LuaResult<T>
lua.eval_multi::<T>(source) -> LuaResult<T>

lua.set_global(name, value) -> LuaResult<()>
lua.get_global::<T>(name) -> LuaResult<Option<T>>
lua.globals() -> Table

lua.call_global::<Args, Ret>(name, args) -> LuaResult<Ret>
lua.call_global1::<Args, Ret>(name, args) -> LuaResult<Ret>

lua.register_function(name, callback) -> LuaResult<()>
lua.create_function(callback) -> LuaResult<Function>
lua.register_async_function(name, callback) -> LuaResult<()>

lua.register_type_of::<T>(name) -> LuaResult<()>
lua.register_type::<T>(name) -> LuaResult<Table>
lua.register_enum_of::<T>(name) -> LuaResult<()>
```

### Value Helpers

```rust
lua.create_string(value) -> LuaResult<LuaString>
lua.create_table() -> LuaResult<Table>
lua.create_table_with_capacity(narr, nrec) -> LuaResult<Table>
lua.create_userdata(data) -> LuaResult<UserDataRef<T>>

lua.pack(value) -> LuaResult<Value>
lua.unpack::<T>(value) -> LuaResult<T>
lua.convert::<T, U>(value) -> LuaResult<U>

lua.get_metatable(value) -> LuaResult<Option<Table>>
lua.set_metatable(value, metatable) -> LuaResult<()>

lua.registry() -> Table
lua.registry_get::<T>(key) -> LuaResult<Option<T>>
lua.registry_set(key, value) -> LuaResult<()>
lua.registry_geti::<T>(key) -> LuaResult<Option<T>>
lua.registry_seti(key, value) -> LuaResult<()>
```

## Async Host API: `LuaAsyncApi`

Implemented by `Lua`.

```rust
lua.exec_async(source).await -> LuaResult<()>
lua.eval_async::<T>(source).await -> LuaResult<T>
lua.eval_multi_async::<T>(source).await -> LuaResult<T>

lua.call_async(function, args).await -> LuaResult<T>
lua.call_async1(function, args).await -> LuaResult<T>
lua.call_async_global(name, args).await -> LuaResult<T>
lua.call_async_global1(name, args).await -> LuaResult<T>
```

## Sandbox Host API: `LuaSandboxApi`

Implemented by `Lua` when the `sandbox` feature is enabled.

```rust
lua.load_sandboxed(source, config) -> Chunk<'_, Lua>
lua.execute_sandboxed(source, config) -> LuaResult<()>
lua.eval_sandboxed::<T>(source, config) -> LuaResult<T>
lua.eval_multi_sandboxed::<T>(source, config) -> LuaResult<T>
```

## Execution Context: `LuaState`

`LuaState` is the low-level per-thread execution context. You usually obtain it from callbacks or from `GlobalState::main_state()`.

### Core Execution

```rust
state.load(source) -> LuaResult<LuaValue>
state.load_with_name(source, chunk_name) -> LuaResult<LuaValue>
state.dofile(path) -> LuaResult<Vec<LuaValue>>
state.execute(source) -> LuaResult<Vec<LuaValue>>
state.execute_chunk(chunk) -> LuaResult<Vec<LuaValue>>

state.call(func, args) -> LuaResult<Vec<LuaValue>>
state.call_global(name, args) -> LuaResult<Vec<LuaValue>>
state.pcall(func, args) -> LuaResult<(bool, Vec<LuaValue>)>
state.xpcall(func, args, handler) -> LuaResult<(bool, Vec<LuaValue>)>
state.get_global_as::<T>(name) -> LuaResult<Option<T>>
```

### Async Execution

```rust
state.register_async(name, callback) -> LuaResult<()>
state.register_async_typed(name, callback) -> LuaResult<()>
state.execute_async(source).await -> LuaResult<Vec<LuaValue>>
state.call_async(func, args).await -> LuaResult<Vec<LuaValue>>
state.call_async_global(name, args).await -> LuaResult<Vec<LuaValue>>
state.create_async_thread(chunk, args) -> LuaResult<AsyncThread>
state.create_async_call_handle(func) -> LuaResult<AsyncCallHandle>
state.create_async_call_handle_global(name) -> LuaResult<AsyncCallHandle>
```

### Sandbox Execution

```rust
state.load_sandboxed(source, config) -> LuaResult<LuaValue>
state.load_with_name_sandboxed(source, chunk_name, config) -> LuaResult<LuaValue>
state.execute_sandboxed(source, config) -> LuaResult<Vec<LuaValue>>
```

### Registration and Values

```rust
state.register_function(name, callback) -> LuaResult<()>
state.register_function_typed(name, callback) -> LuaResult<()>
state.register_type_of::<T>(name) -> LuaResult<()>

state.create_string(value) -> CreateResult
state.create_table(narr, nrec) -> CreateResult
state.create_userdata(data) -> CreateResult
state.create_closure(callback) -> CreateResult
```

## Low-Level Owner: `GlobalState`

`GlobalState` owns the runtime. It is not the recommended top-level host API anymore, but it still exposes low-level operations that `Lua` and `LuaState` build on.

### Runtime Ownership and Low-Level Services

```rust
GlobalState::new(option) -> Pin<Box<GlobalState>>
global.main_state() -> &mut LuaState

global.open_stdlib(lib) -> LuaResult<()>
global.open_stdlibs(libs) -> LuaResult<()>

global.compile(source) -> LuaResult<LuaProto>
global.compile_with_name(source, chunk_name) -> LuaResult<LuaProto>
global.load_proto_from_file(path) -> LuaResult<ProtoPtr>
```

### Globals, Registry, and Refs

```rust
global.set_global(name, value) -> LuaResult<()>
global.get_global(name) -> LuaResult<Option<LuaValue>>

global.registry_set(key, value) -> LuaResult<()>
global.registry_get(key) -> LuaResult<Option<LuaValue>>
global.registry_seti(key, value)
global.registry_geti(key) -> Option<LuaValue>

global.create_ref(value) -> LuaRefValue
global.get_ref_value(ref_value) -> LuaValue
global.release_ref(ref_value)
global.release_ref_id(ref_id)
```

### Low-Level Value Construction

```rust
global.create_string(value) -> CreateResult
global.create_table(narr, nrec) -> CreateResult
global.create_userdata(data) -> CreateResult
global.create_function(chunk, upvalues) -> CreateResult
global.create_closure(callback) -> CreateResult
global.create_thread(func) -> CreateResult
```

### Managed Ref Helpers

```rust
global.create_table_ref(narr, nrec) -> LuaResult<LuaTableRef>
global.build_table_ref(builder) -> LuaResult<LuaTableRef>

global.to_ref(value) -> LuaAnyRef
global.to_table_ref(value) -> Option<LuaTableRef>
global.to_function_ref(value) -> Option<LuaFunctionRef>
global.to_string_ref(value) -> Option<LuaStringRef>
global.to_userdata_ref::<T>(value) -> Option<UserDataRef<T>>
```

## TableBuilder

`TableBuilder` is a fluent helper for constructing Lua tables before materializing them.

```rust
use luars::{LuaValue, TableBuilder};

let builder = TableBuilder::new()
    .set("host", LuaValue::integer(1))
    .push(LuaValue::integer(42));
```

Build with either `GlobalState` or a compatible low-level context.

## Key Types

### `LuaValue`

Represents any raw Lua value.

```rust
LuaValue::nil()
LuaValue::boolean(true)
LuaValue::integer(42)
LuaValue::float(3.14)
```

Common inspection methods:

```rust
value.is_nil()
value.is_boolean()
value.is_integer()
value.is_number()
value.is_string()
value.is_table()
value.is_function()
value.is_userdata()

value.as_boolean()
value.as_integer()
value.as_number()
value.as_str()
```

### `LuaError` and `LuaFullError`

`LuaError` is the lightweight error kind. `LuaFullError` combines that kind with the human-readable message stored in the runtime.

Use:

```rust
let full = global.get_full_error(err);
```

or, on the high-level API:

```rust
let full = lua.get_error_message(err);
```

### `SafeOption`

Controls runtime limits such as:

- maximum call depth
- maximum stack size
- maximum GC memory
- optional instruction limit

### `Stdlib`

The standard library selector enum:

```rust
Stdlib::Basic
Stdlib::String
Stdlib::Table
Stdlib::Math
Stdlib::IO
Stdlib::OS
Stdlib::Coroutine
Stdlib::Utf8
Stdlib::Package
Stdlib::Debug
Stdlib::All
```

Convenience constructors: `string(s)`, `integer(n)`, `float(n)`, `boolean(b)`, `nil()`, `table(pairs)`.

### Type Aliases

```rust
type CFunction = fn(&mut LuaState) -> LuaResult<usize>;
type RustCallback = Box<dyn Fn(&mut LuaState) -> LuaResult<usize>>;
type CreateResult = LuaResult<LuaValue>;
type LuaResult<T> = Result<T, LuaError>;
```
