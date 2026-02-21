# Multi-VM Patterns

`LuaVM` is `!Send` — it cannot be moved across threads. This document explains how to use multiple LuaVM instances in scenarios that require concurrency or parallelism.

---

## Why is LuaVM `!Send`?

```rust
pub struct LuaVM {
    gc: GarbageCollector,      // contains raw pointers
    main_state: *mut LuaState, // raw pointer
    registry: LuaValue,        // GC-managed object
    global: LuaValue,          // GC-managed object
    // ...
}
```

`LuaVM` contains raw pointers and `Rc` references, and the Rust compiler doesn't allow these to be moved across threads. This isn't a bug — it's a design decision. Lua itself is not thread-safe.

---

## Pattern 1: Single Thread + `current_thread` Runtime

The simplest pattern. Suitable for I/O-bound scenarios:

```rust
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut vm = LuaVM::new(SafeOption::default());
    vm.open_stdlib(Stdlib::All).unwrap();

    vm.register_async("fetch", |args| async move {
        // ... async I/O
        Ok(vec![AsyncReturnValue::string("result")])
    }).unwrap();

    // All requests processed sequentially, but I/O is async
    loop {
        let request = get_next_request().await;
        let result = vm.execute_async(&request).await;
        send_response(result).await;
    }
}
```

**Pros:** Simple, no thread synchronization overhead
**Cons:** Single core, CPU-intensive tasks block all subsequent requests

---

## Pattern 2: Thread-per-VM Pool

Each thread owns an independent LuaVM and single-threaded tokio runtime:

```text
┌────────────────────────────────────────────────────────┐
│                Main Thread (tokio multi-thread)         │
│  Accept requests → dispatch to worker (round-robin)    │
└──────────┬──────────────┬──────────────┬───────────────┘
           │              │              │
     ┌─────▼─────┐ ┌─────▼─────┐ ┌─────▼─────┐
     │ Thread 0   │ │ Thread 1   │ │ Thread 2   │
     │ LuaVM 0    │ │ LuaVM 1    │ │ LuaVM 2    │
     │ tokio (CT)  │ │ tokio (CT)  │ │ tokio (CT)  │
     └────────────┘ └────────────┘ └────────────┘
```

*CT = current_thread runtime*

### Full Implementation

```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{mpsc, oneshot};

/// Request sent to a worker
struct Request {
    lua_code: String,
    respond: oneshot::Sender<Result<String, String>>,
}

/// VM thread pool
struct VmPool {
    senders: Vec<mpsc::Sender<Request>>,
    next: AtomicUsize,
}

impl VmPool {
    fn new(num_workers: usize, init_script: &str) -> Self {
        let mut senders = Vec::new();

        for id in 0..num_workers {
            let (tx, mut rx) = mpsc::channel::<Request>(64);
            let script = init_script.to_string();

            // Each worker is a separate thread
            std::thread::Builder::new()
                .name(format!("lua-worker-{}", id))
                .spawn(move || {
                    // Create a thread-local tokio runtime
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();

                    rt.block_on(async move {
                        // Create a thread-local LuaVM
                        let mut vm = LuaVM::new(SafeOption::default());
                        vm.open_stdlib(Stdlib::All).unwrap();

                        // Register async functions
                        vm.register_async("sleep", |args| async move {
                            let secs = args[0].as_number().unwrap_or(1.0);
                            tokio::time::sleep(
                                std::time::Duration::from_secs_f64(secs)
                            ).await;
                            Ok(vec![AsyncReturnValue::boolean(true)])
                        }).unwrap();

                        // Load initialization script
                        vm.execute_string(&script).unwrap();

                        // Process requests
                        while let Some(req) = rx.recv().await {
                            let result = vm.execute_async(&req.lua_code).await;
                            let response = match result {
                                Ok(vals) => {
                                    let s = vals.iter()
                                        .map(|v| format!("{:?}", v))
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    Ok(s)
                                }
                                Err(e) => Err(format!("{:?}", e)),
                            };
                            let _ = req.respond.send(response);
                        }
                    });
                })
                .unwrap();

            senders.push(tx);
        }

        VmPool {
            senders,
            next: AtomicUsize::new(0),
        }
    }

    /// Round-robin request dispatch
    async fn execute(&self, lua_code: String) -> Result<String, String> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        let (tx, rx) = oneshot::channel();
        self.senders[idx]
            .send(Request { lua_code, respond: tx })
            .await
            .map_err(|_| "worker dead".to_string())?;
        rx.await.map_err(|_| "worker dropped".to_string())?
    }
}
```

**Pros:**
- Utilizes multiple CPU cores
- Complete isolation between VMs (one crash doesn't affect others)
- Each VM can have independent state (counters, caches, etc.)
- I/O within each worker is still async

**Cons:**
- Memory overhead (each VM has its own GC, string pool, etc.)
- Requires channel-based communication

---

## Pattern 3: Single Thread + `LocalSet`

Use tokio's `LocalSet` to run multiple tasks on a single thread:

```rust
use tokio::task::LocalSet;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let local = LocalSet::new();

    local.run_until(async {
        let mut vm = LuaVM::new(SafeOption::default());
        vm.open_stdlib(Stdlib::All).unwrap();

        // Register async functions...
        vm.register_async("sleep", |args| async move {
            let secs = args[0].as_number().unwrap_or(1.0);
            tokio::time::sleep(std::time::Duration::from_secs_f64(secs)).await;
            Ok(vec![AsyncReturnValue::boolean(true)])
        }).unwrap();

        // Use spawn_local for tasks (doesn't require Send)
        let handle = tokio::task::spawn_local(async move {
            vm.execute_async("sleep(1); return 42").await
        });

        let result = handle.await.unwrap();
        println!("Result: {:?}", result);
    }).await;
}
```

**Note:** `spawn_local` must be called within a `LocalSet` context.

---

## VM State Isolation

Each VM is completely independent:

```lua
-- Running on VM-0
counter = (counter or 0) + 1
print(counter)  --> 1, 2, 3, ...

-- Running on VM-1
counter = (counter or 0) + 1
print(counter)  --> 1, 2, 3, ... (independent counter)
```

This means:
- Global variables are per-VM
- One VM's GC doesn't affect other VMs
- One VM's errors (even unrecoverable ones) don't affect other VMs

If you need to share state between VMs, use Rust-side shared data structures (e.g., `Arc<Mutex<T>>`) exposed to Lua via registered functions.

---

## Selection Guide

| Scenario | Recommended Pattern |
|----------|-------------------|
| Simple script runner | Pattern 1 (single thread) |
| I/O-intensive server | Pattern 2 (thread-per-VM) |
| Multi-core with limited memory | Pattern 2 (fewer workers) |
| GUI app with embedded Lua | Pattern 3 (LocalSet) |
| Single request processing | Pattern 1 (simplest) |
| High-concurrency HTTP server | Pattern 2 (workers = CPU cores) |

---

## Related Documentation

- [HTTP Server Example](./06-http-server.md) — Complete Pattern 2 implementation
- [Internal Architecture](./04-architecture.md) — Understanding the root cause of `!Send`
