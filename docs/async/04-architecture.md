# Internal Architecture

Implementation details of luars async features. Understanding these helps with debugging and designing advanced usage patterns.

---

## Core Idea

luars uses a **coroutine-driven Future bridging** pattern:

1. An async Rust function is wrapped as a synchronous Lua C function
2. When called, the C function creates a Future and stores it in the coroutine state, then yields
3. The external driver (`AsyncThread`) detects the yield, extracts the Future, and polls it
4. Once the Future completes, the driver resumes the coroutine with the result
5. From Lua's perspective, the function call is synchronous — yield/resume is completely transparent

```text
Lua's view:                     Rust's view:

local x = async_fn(42)         ┌─ coroutine.resume()
                                │   ├─ async_fn is called
-- Lua doesn't know what       │   ├─ Creates Future
-- happened — it just waits    │   └─ yield(SENTINEL)
-- for the return value --      │
                                ├─ AsyncThread detects SENTINEL
                                ├─ Extracts Future
                                ├─ Polls Future
                                │   ├─ Pending → return Poll::Pending
                                │   └─ Ready(result) ↓
                                │
                                ├─ coroutine.resume(result)
                                │   └─ async_fn resumes from yield
                                │       Return value passed back to Lua
                                └─ x = result
print(x)
```

---

## Key Components

### 1. `wrap_async_function()` — Function Wrapper

Wraps `Fn(Vec<LuaValue>) -> Future` into `Fn(&mut LuaState) -> LuaResult<usize>`:

```text
wrap_async_function(f) produces a closure that:

1. Collects arguments from the Lua stack → args: Vec<LuaValue>
2. Calls f(args) → creates a Future
3. Stores the Future in state.pending_future
4. yield(ASYNC_SENTINEL) → signals the AsyncThread
```

```rust
// Simplified source
pub fn wrap_async_function<F, Fut>(f: F) -> impl Fn(&mut LuaState) -> LuaResult<usize>
where
    F: Fn(Vec<LuaValue>) -> Fut + 'static,
    Fut: Future<Output = LuaResult<Vec<AsyncReturnValue>>> + 'static,
{
    move |state: &mut LuaState| {
        let args = state.get_args();
        let future = f(args);
        state.set_pending_future(Box::pin(future));
        state.do_yield(vec![async_sentinel_value()])?;
        Ok(0) // never reached
    }
}
```

### 2. ASYNC_SENTINEL — Sentinel Value

The key mechanism for distinguishing "async yield" from regular `coroutine.yield()`:

```rust
static ASYNC_SENTINEL_STORAGE: u8 = 0;

pub fn async_sentinel_value() -> LuaValue {
    LuaValue::lightuserdata(&ASYNC_SENTINEL_STORAGE as *const u8 as *mut c_void)
}

pub fn is_async_sentinel(values: &[LuaValue]) -> bool {
    values.len() == 1
        && values[0].ttislightuserdata()
        && values[0].pvalue() == &ASYNC_SENTINEL_STORAGE as *const u8 as *mut c_void
}
```

A `lightuserdata` address is used as a unique identifier. Lua code cannot (and doesn't need to) detect this sentinel.

### 3. `AsyncThread` — Future Driver

The core state machine implementing the `std::future::Future` trait:

```rust
pub struct AsyncThread {
    thread_val: LuaValue,              // Coroutine
    vm: *mut LuaVM,                     // VM raw pointer
    ref_id: RefId,                      // Registry reference (prevents GC)
    pending: Option<AsyncFuture>,       // Currently pending Future
    initial_args: Option<Vec<LuaValue>>, // Arguments for first resume
}
```

#### Poll Flow

```text
AsyncThread::poll(cx)
│
├─ has pending future?
│   ├─ YES → poll it
│   │   ├─ Pending → return Poll::Pending
│   │   └─ Ready(result)
│   │       ├─ materialize_values() — convert AsyncReturnValue → LuaValue
│   │       └─ resume(result_values)
│   │           ├─ Finished → return Poll::Ready(result)
│   │           ├─ AsyncYield → set new pending, continue loop
│   │           └─ NormalYield → wake(), return Pending
│   │
│   └─ NO → resume(initial_args)
│       ├─ Finished → return Poll::Ready(result)
│       ├─ AsyncYield → set pending, enter poll_pending loop
│       └─ NormalYield → wake(), return Pending
```

#### ResumeResult Classification

```rust
enum ResumeResult {
    Finished(LuaResult<Vec<LuaValue>>),   // Coroutine completed (return or error)
    AsyncYield(AsyncFuture),               // Async yield + pending Future
    NormalYield(Vec<LuaValue>),            // Regular coroutine.yield()
}
```

After each resume, the result is checked:
- **Finished**: Coroutine execution complete, return results
- **AsyncYield**: SENTINEL detected, extract pending future
- **NormalYield**: Regular yield, wake immediately and re-poll

### 4. `AsyncReturnValue` — Return Value Intermediate Layer

Solves the GC safety problem:

```text
                    During async Future execution
                    (no &mut LuaVM available)
                            │
                            ▼
               AsyncReturnValue::String("hello")
               AsyncReturnValue::UserData(point)
                            │
                    After Future completes
                   AsyncThread takes over
                            │
                            ▼
            materialize_values(&mut vm, values)
                            │
                            ▼
                vm.create_string("hello")
                vm.create_userdata(point)
                            │
                            ▼
                   LuaValue (GC-managed object)
```

### 5. GC Safety

`AsyncThread` maintains a registry reference to the coroutine on creation:

```rust
impl AsyncThread {
    pub(crate) fn new(thread_val: LuaValue, vm: *mut LuaVM, args: Vec<LuaValue>) -> Self {
        // Register in registry to prevent GC collection
        let ref_id = {
            let vm_ref = unsafe { &mut *vm };
            let lua_ref = vm_ref.create_ref(thread_val);
            lua_ref.ref_id().unwrap_or(0)
        };
        // ...
    }
}

impl Drop for AsyncThread {
    fn drop(&mut self) {
        // Release registry reference, allowing GC to collect
        if self.ref_id > 0 {
            let vm = unsafe { &mut *self.vm };
            vm.release_ref_id(self.ref_id);
        }
    }
}
```

---

## `!Send` Constraint

Both `AsyncThread` and `AsyncFuture` are `!Send` because:

1. `LuaVM` contains raw pointers (`*mut`) and `Rc`
2. `AsyncThread` holds `*mut LuaVM`
3. GC objects (strings, tables, etc.) are not thread-safe

This means:
- Must use `tokio::runtime::Builder::new_current_thread()` or `LocalSet`
- Cannot use `tokio::spawn()` (requires `Send`) — use `tokio::task::spawn_local()` instead
- Multi-threaded scenarios require the thread-per-VM pattern

---

## Related Documentation

- [API Reference](./02-api-reference.md) — Type and method usage documentation
- [Multi-VM Patterns](./05-multi-vm.md) — How to use in multi-threaded scenarios
- [Async_Design.md](../Async_Design.md) — Original design RFC document
