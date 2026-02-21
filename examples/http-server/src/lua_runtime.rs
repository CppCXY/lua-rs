//! Lua runtime setup: creates and configures a LuaVM instance for request handling.
//!
//! Each VM gets:
//! - Full standard library (string, table, math, etc.)
//! - Async I/O functions (sleep, read_file, write_file, etc.)
//! - A `handle_request(method, path, headers_str, body)` entry point driven from Rust

use luars::lua_vm::{LuaVM, SafeOption};
use luars::{LuaResult, Stdlib};

use crate::async_io;

/// Create a new LuaVM configured for HTTP request handling.
///
/// The returned VM has:
/// 1. Standard library loaded
/// 2. Async I/O functions registered
/// 3. The user's Lua handler script loaded
pub fn create_vm(lua_script: &str) -> LuaResult<Box<LuaVM>> {
    let mut vm = LuaVM::new(SafeOption::default());

    // Load the full standard library
    vm.open_stdlib(Stdlib::All)?;

    // Register async I/O functions
    async_io::register_all(&mut vm)?;

    // Load the user's handler script (defines handle_request, routes, etc.)
    vm.execute(lua_script)?;

    Ok(vm)
}

/// Call the Lua `handle_request` function asynchronously.
///
/// The Lua function signature:
/// ```lua
/// function handle_request(method, path, query, headers_json, body)
///     -- returns: status_code, content_type, body
/// end
/// ```
///
/// Returns `(status_code, content_type, response_body)`.
pub async fn call_handler(
    vm: &mut LuaVM,
    method: &str,
    path: &str,
    query: Option<&str>,
    headers_json: &str,
    body: &str,
) -> LuaResult<(u16, String, String)> {
    let query_arg = match query {
        Some(q) => format!("\"{}\"", escape_lua_string(q)),
        None => "nil".to_string(),
    };
    let source = format!(
        r#"return handle_request("{}", "{}", {}, [=[{}]=], [=[{}]=])"#,
        escape_lua_string(method),
        escape_lua_string(path),
        query_arg,
        headers_json,
        body,
    );

    let results = vm.execute_async(&source).await?;

    // Extract: status_code (integer), content_type (string), body (string)
    let status = results.first().and_then(|v| v.as_integer()).unwrap_or(200) as u16;
    let content_type = results
        .get(1)
        .and_then(|v| v.as_str())
        .unwrap_or("text/plain")
        .to_string();
    let resp_body = results
        .get(2)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok((status, content_type, resp_body))
}

/// Escape a string for safe embedding in Lua double-quoted strings.
fn escape_lua_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
