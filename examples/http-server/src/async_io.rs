//! Async host functions registered into the high-level Lua runtime.

use luars::{Lua, LuaResult};
use std::time::Duration;

/// Register all host functions needed by the HTTP example.
pub fn register_all(lua: &mut Lua) -> LuaResult<()> {
    register_sleep(lua)?;
    register_read_file(lua)?;
    register_write_file(lua)?;
    register_time(lua)?;
    register_env(lua)?;
    register_log(lua)?;
    Ok(())
}

fn register_sleep(lua: &mut Lua) -> LuaResult<()> {
    lua.register_async_function("sleep", |secs: Option<f64>| async move {
        let secs = secs.unwrap_or(1.0);
        tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        Ok(true)
    })
}

fn register_read_file(lua: &mut Lua) -> LuaResult<()> {
    lua.register_async_function("read_file", |path: String| async move {
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => Ok((Some(content), Option::<String>::None)),
            Err(e) => Ok((
                Option::<String>::None,
                Some(format!("read_file error: {}", e)),
            )),
        }
    })
}

fn register_write_file(lua: &mut Lua) -> LuaResult<()> {
    lua.register_async_function("write_file", |path: String, content: String| async move {
        match tokio::fs::write(&path, content.as_bytes()).await {
            Ok(()) => Ok((Some(true), Option::<String>::None)),
            Err(e) => Ok((
                Option::<bool>::None,
                Some(format!("write_file error: {}", e)),
            )),
        }
    })
}

fn register_time(lua: &mut Lua) -> LuaResult<()> {
    lua.register_async_function("time", || async move {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Ok(now.as_secs_f64())
    })
}

fn register_env(lua: &mut Lua) -> LuaResult<()> {
    lua.register_async_function("env", |name: String| async move {
        match std::env::var(&name) {
            Ok(val) => Ok(Some(val)),
            Err(_) => Ok(Option::<String>::None),
        }
    })
}

fn register_log(lua: &mut Lua) -> LuaResult<()> {
    lua.register_async_function("log", |message: String| async move {
        eprintln!("[lua] {message}");
        Ok(())
    })
}
