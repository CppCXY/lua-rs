# HTTP Server Example

A complete async HTTP server using luars. Demonstrates Pattern 2 (Thread-per-VM Pool) from the [Multi-VM Patterns](./05-multi-vm.md) guide.

---

## Project Structure

```
examples/http-server/
├── main.rs           Entry point, HTTP listener
├── http.rs           Lightweight HTTP request/response parser
├── async_io.rs       Async I/O functions registered into Lua
├── lua_runtime.rs    LuaVM initialization and setup
├── vm_pool.rs        Thread-based VM pool with round-robin dispatch
└── handler.lua       Request handler written in Lua
```

---

## Architecture

```text
                     ┌─────────────┐
  HTTP Request ──────► Main Thread │
                     │  (Listener) │
                     └──────┬──────┘
                            │ round-robin dispatch
              ┌─────────────┼─────────────┐
              ▼             ▼             ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐
        │ Worker 0 │ │ Worker 1 │ │ Worker 2 │
        │  LuaVM   │ │  LuaVM   │ │  LuaVM   │
        │  tokio   │ │  tokio   │ │  tokio   │
        │  (CT)    │ │  (CT)    │ │  (CT)    │
        └──────────┘ └──────────┘ └──────────┘
```

Each worker thread owns its own LuaVM and current_thread tokio runtime. HTTP request handling is written in Lua (`handler.lua`), while async I/O operations (HTTP fetch, sleep, file read) are provided by Rust.

---

## Module Details

### `main.rs` — Entry Point

- Parses command-line arguments (port, worker count)
- Creates the VM pool (`VmPool::new()`)
- Listens for TCP connections
- Dispatches each request to a worker via round-robin

### `http.rs` — HTTP Parser

A minimal HTTP/1.1 parser. Handles:
- Request parsing: method, path, headers, body
- Response formatting with status code, content type, and body
- No external HTTP library dependency

### `async_io.rs` — Async I/O for Lua

Registers the following async functions into each LuaVM:

| Function | Arguments | Return Value | Description |
|----------|-----------|--------------|-------------|
| `http_get(url)` | URL string | Response body string | Performs an HTTP GET request |
| `sleep(secs)` | Number (seconds) | `true` | Async sleep |
| `read_file(path)` | File path string | File contents string | Reads a file asynchronously |

All functions use `AsyncReturnValue` to return results to Lua after the async operation completes.

### `lua_runtime.rs` — VM Setup

- Creates a LuaVM with standard library
- Registers all async I/O functions via `register_async()`
- Loads `handler.lua`
- Returns the configured VM

### `vm_pool.rs` — Thread Pool

Implements the Pattern 2 architecture:
- Spawns N worker threads, each with its own LuaVM + tokio runtime
- Uses `mpsc` channels to send requests to workers
- Uses `oneshot` channels to receive responses
- Round-robin dispatch via `AtomicUsize`

### `handler.lua` — Request Handler

The Lua script that processes HTTP requests:

```lua
function handle_request(method, path, body)
    if path == "/" then
        return 200, "text/plain", "Hello from luars!"

    elseif path == "/time" then
        return 200, "text/plain", os.date("%Y-%m-%d %H:%M:%S")

    elseif path == "/echo" and method == "POST" then
        return 200, "text/plain", body

    elseif path == "/sleep" then
        sleep(1)
        return 200, "text/plain", "Slept for 1 second"

    elseif path == "/fetch" then
        local data = http_get("http://httpbin.org/get")
        return 200, "application/json", data

    else
        return 404, "text/plain", "Not Found"
    end
end
```

---

## Available Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Returns "Hello from luars!" |
| `/time` | GET | Returns current date and time |
| `/echo` | POST | Echoes back the request body |
| `/sleep` | GET | Sleeps 1 second asynchronously, then responds |
| `/fetch` | GET | Fetches data from an external URL |
| `*` | Any | Returns 404 Not Found |

---

## Running the Server

### Build

```bash
cargo build --release --example http-server
```

### Start

```bash
# Default: port 8080, workers = number of CPU cores
cargo run --release --example http-server

# Custom port and worker count
cargo run --release --example http-server -- --port 3000 --workers 4
```

### Test

```bash
# Basic endpoint
curl http://localhost:8080/

# Current time
curl http://localhost:8080/time

# Echo
curl -X POST -d "Hello!" http://localhost:8080/echo

# Async sleep
curl http://localhost:8080/sleep

# External fetch
curl http://localhost:8080/fetch
```

---

## Performance Notes

- Each worker handles requests sequentially, but async I/O (sleep, fetch) doesn't block the worker's event loop
- With N workers, up to N requests can be handled in parallel
- The round-robin dispatcher ensures even load distribution
- Worker count is typically set to the number of CPU cores

---

## Related Documentation

- [Multi-VM Patterns](./05-multi-vm.md) — Detailed explanation of the thread-per-VM pattern
- [API Reference](./02-api-reference.md) — `register_async()`, `execute_string_async()`
- [Architecture](./04-architecture.md) — How async bridging works internally
