# luars

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![crate](https://img.shields.io/crates/v/luars.svg?style=flat-square)](https://crates.io/crates/luars)

> **Note**: This is an **Lua 5.5** lib through AI-assisted programming.

luars is a pure Rust Lua 5.5 runtime and embedding toolkit. This repository contains the core runtime, derive macros, the standalone interpreter, debugger integration, a WASM target, benchmark scripts, and host-facing examples.

The project is shaped around three priorities:

- performance that is measured against native Lua instead of hand-waved
- safety work that keeps `unsafe` narrowly scoped to representation and truly hot VM paths
- compatibility validated against the upstream Lua test suite, not only custom examples

## Why luars

### Performance

The repository ships benchmark runners and published benchmark snapshots instead of a single cherry-picked number.

Current Windows snapshot on a Ryzen 7 5800X, measured against native Lua 5.5 on the same machine:

| Area | Relative throughput vs native Lua |
|------|-----------------------------------|
| Arithmetic | 111% |
| Locals / register-heavy code | 132% |
| Table library | 118% |
| Coroutines | 152% |
| Errors | 103% |

There is no repository-maintained Linux snapshot from a dedicated physical Linux machine yet, because the current benchmark process does not have a real Linux box behind it. For Linux runs, use the GitHub Actions benchmark workflow as the best continuous reference point.

Observed Linux behavior so far is mixed but fairly consistent: luars is often about 20% to 40% slower than native Lua on broad benchmark coverage, while still beating native Lua in a non-trivial subset of cases.

The current broader script-level snapshot is available here:

- [docs/benchmarks/windows.md](docs/benchmarks/windows.md)
- [docs/benchmarks/macos.md](docs/benchmarks/macos.md)
- GitHub Actions benchmark workflow: https://github.com/CppCXY/lua-rs/actions/workflows/benchmarks.yml

Benchmark commands are part of the repository:

- `./run_benchmarks.ps1` / `./run_benchmarks.sh`
- `./run_lua_benchmarks.ps1` / `./run_lua_benchmarks.sh`

One important caveat for the Windows numbers: the coroutine result is unusually strong partly because native Lua's coroutine implementation on Windows pays heavily for `longjmp`-based control transfer, which triggers unwind-related overhead. luars does not inherit that exact cost model, so the Windows coroutine gap is real in measurement but should not be over-generalized into a platform-independent "always faster" claim.

Performance tests on macOS generally show that it outperforms native Lua. On one hand, this may be due to Apple's M-series chips being better at branch prediction and optimization, as well as more mature LLVM optimizations on the Apple platform. On the other hand, it could be because the ARM architecture offers more registers, and LuaRS caches more register states than native Lua. On x86-64, which has fewer registers, this may lead to increased memory access and reduced performance. However, this is only a speculation, and more in-depth analysis and testing are needed to verify it.

### Safety And Audit Posture

luars is implemented in Rust, but it does not pretend that "written in Rust" automatically means the whole runtime is safe by default. The project keeps `unsafe` under active review and pushes non-critical paths back to safe Rust when that does not cost hot-path performance.

Recent cleanup work narrowed the unsafe surface in VM helper and dispatch-adjacent code, including helper, concat, call, and metamethod paths. The remaining `unsafe` usage is concentrated in the places where the runtime actually needs it:

- GC/object representation and tagged value layout
- stack/upvalue pointer fast paths in hot interpreter execution
- a small number of performance-sensitive VM internals where safe equivalents were measured to be worse

There are also Miri smoke tests in the repo for low-level runtime sanity checks.

### Compatibility And Test Status

The repository includes the official Lua test suite under `lua_tests/testes`, plus the scripts needed to run it directly.

Current status from a fresh release-mode run in this workspace:

- `./run_lua_tests.ps1 -Profile release -Script all.lua -SkipBuild`
- Result: `final OK !!!`

That means the current tree passes the upstream Lua test suite entrypoint used by this repository, with the expected skips for `testC`-dependent cases that are not active in this environment.

## Crate Features

Core crate feature flags for `luars`:

| Feature | Purpose |
|---------|---------|
| `serde` | Enable `serde` / `serde_json` integration for Lua value conversion and host interop |
| `sandbox` | Enable sandbox-oriented helpers and isolated execution entry points |
| `shared-proto` | Reuse cached compiled proto/constant-string state across loads/VM instances to reduce duplicate compilation and allocation work |

Minimal dependency:

```toml
[dependencies]
luars = "0.21"
```

With optional features:

```toml
[dependencies]
luars = { version = "0.21", features = ["serde", "sandbox", "shared-proto"] }
```

## Quick Start

```rust
use luars::{Lua, SafeOption, Stdlib};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut lua = Lua::new(SafeOption::default());
    lua.load_stdlibs(Stdlib::All)?;

    lua.register_function("add", |a: i64, b: i64| a + b)?;
    let sum: i64 = lua.load("return add(20, 22)").eval()?;

    assert_eq!(sum, 42);
    Ok(())
}
```

## High-Level API Highlights

- Execute chunks with `lua.load(...).exec()`, `eval()`, and `eval_multi()`
- Execute async chunks with `exec_async()`, `eval_async()`, and `eval_multi_async()`
- Call Lua globals with `call_global()` / `call_global1()` and async variants
- Register Rust functions with typed and untyped callback APIs
- Expose Rust types with `register_type()` and `LuaUserData`
- Work with globals and tables through `globals()`, `create_table()`, and `create_table_from()`
- Create scoped borrowed callbacks and userdata through `scope(...)`
- Run isolated chunks through sandbox helpers when the `sandbox` feature is enabled

## Repository Layout

| Path | Description |
|------|-------------|
| `crates/luars` | Core library: compiler, VM, GC, string/table runtime, and high-level `Lua` API |
| `crates/luars-derive` | `LuaUserData` and related derive/macros |
| `crates/luars_interpreter` | Standalone interpreter, tooling, and benchmark entrypoints |
| `crates/luars_debugger` | Debugger integration |
| `crates/luars_wasm` | WASM bindings |
| `benchmarks/` | Repository benchmark scripts used for performance tracking |
| `lua_tests/` | Upstream Lua test suite used for compatibility validation |
| `docs/` | Guides, benchmark snapshots, async notes, and user-facing docs |
| `examples/` | Embedding and host integration examples |

## Examples

| Example | Description |
|---------|-------------|
| [examples/luars-example](examples/luars-example) | Minimal host embedding example |
| [examples/rules-engine-demo](examples/rules-engine-demo) | Rules engine with Rust host functions and Lua policy |
| [examples/http-server](examples/http-server) | Async HTTP example with sandboxed request execution |
| [examples/rust-bind-bench](examples/rust-bind-bench) | Binding and userdata benchmark example |

## Documentation

| Document | Description |
|----------|-------------|
| [docs/Guide.md](docs/Guide.md) | High-level `Lua` API overview |
| [docs/UserGuide.md](docs/UserGuide.md) | Userdata and embedding guide |
| [docs/Async.md](docs/Async.md) | Async execution and related notes |
| [docs/Different.md](docs/Different.md) | Known differences from C Lua |
| [crates/luars/README.md](crates/luars/README.md) | Crate-level documentation |

## Validation Commands

```bash
cargo test
```

```bash
./run_lua_tests.ps1 -Profile release -Script all.lua
```

```bash
MIRIFLAGS="-Zmiri-disable-stacked-borrows -Zmiri-permissive-provenance" cargo +nightly miri test -p luars --lib miri_ -- --nocapture
```

The Miri profile disables stacked borrows and uses permissive provenance because luars relies on tagged-pointer and compact GC/object layouts that intentionally reconstruct raw pointers.

## License

MIT. See [LICENSE](LICENSE).
