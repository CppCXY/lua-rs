//! Async I/O functions registered into the Lua VM.
//!
//! These functions demonstrate real async I/O operations available from Lua:
//! - `sleep(seconds)` — async sleep via tokio
//! - `read_file(path)` — async file reading via tokio::fs
//! - `write_file(path, content)` — async file writing via tokio::fs
//! - `fetch(url)` — simple HTTP GET via raw TCP (no external HTTP client dep)
//! - `json_encode(table_string)` / `json_decode(json_string)` — helpers
//! - `time()` — current unix timestamp (sync, but useful for benchmarks)

use luars::lua_vm::async_thread::AsyncReturnValue;
use luars::{LuaResult, LuaVM};
use std::time::Duration;

/// Register all async I/O functions into the VM.
pub fn register_all(vm: &mut LuaVM) -> LuaResult<()> {
    register_sleep(vm)?;
    register_read_file(vm)?;
    register_write_file(vm)?;
    register_time(vm)?;
    register_env(vm)?;
    register_log(vm)?;
    Ok(())
}

/// `sleep(seconds)` — async sleep, returns true when done.
fn register_sleep(vm: &mut LuaVM) -> LuaResult<()> {
    vm.register_async("sleep", |args| async move {
        let secs = args.first().and_then(|v| v.as_number()).unwrap_or(1.0);
        tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        Ok(vec![AsyncReturnValue::boolean(true)])
    })
}

/// `read_file(path)` — async file read, returns (content, nil) or (nil, error).
fn register_read_file(vm: &mut LuaVM) -> LuaResult<()> {
    vm.register_async("read_file", |args| async move {
        let path = args
            .first()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => Ok(vec![
                AsyncReturnValue::string(content),
                AsyncReturnValue::nil(),
            ]),
            Err(e) => Ok(vec![
                AsyncReturnValue::nil(),
                AsyncReturnValue::string(format!("read_file error: {}", e)),
            ]),
        }
    })
}

/// `write_file(path, content)` — async file write, returns (true, nil) or (nil, error).
fn register_write_file(vm: &mut LuaVM) -> LuaResult<()> {
    vm.register_async("write_file", |args| async move {
        let path = args
            .first()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let content = args
            .get(1)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();

        match tokio::fs::write(&path, content.as_bytes()).await {
            Ok(()) => Ok(vec![
                AsyncReturnValue::boolean(true),
                AsyncReturnValue::nil(),
            ]),
            Err(e) => Ok(vec![
                AsyncReturnValue::nil(),
                AsyncReturnValue::string(format!("write_file error: {}", e)),
            ]),
        }
    })
}

/// `time()` — returns current unix timestamp as a float (sync wrapper).
fn register_time(vm: &mut LuaVM) -> LuaResult<()> {
    vm.register_async("time", |_args| async move {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Ok(vec![AsyncReturnValue::float(now.as_secs_f64())])
    })
}

/// `env(name)` — read environment variable (sync wrapper).
fn register_env(vm: &mut LuaVM) -> LuaResult<()> {
    vm.register_async("env", |args| async move {
        let name = args
            .first()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        match std::env::var(&name) {
            Ok(val) => Ok(vec![AsyncReturnValue::string(val)]),
            Err(_) => Ok(vec![AsyncReturnValue::nil()]),
        }
    })
}

/// `log(...)` — print to server stderr, returns nil.
fn register_log(vm: &mut LuaVM) -> LuaResult<()> {
    vm.register_async("log", |args| async move {
        let parts: Vec<String> = args
            .iter()
            .map(|v| {
                if let Some(s) = v.as_str() {
                    s.to_string()
                } else if let Some(n) = v.as_integer() {
                    n.to_string()
                } else if let Some(n) = v.as_number() {
                    n.to_string()
                } else if let Some(b) = v.as_bool() {
                    b.to_string()
                } else {
                    "nil".to_string()
                }
            })
            .collect();
        eprintln!("[lua] {}", parts.join("\t"));
        Ok(vec![AsyncReturnValue::nil()])
    })
}
