//! High-level Lua runtime setup for the HTTP example.

use luars::{
    Lua, LuaApi, LuaAsyncApi, LuaFunction, LuaResult, LuaSandboxApi, SafeOption, SandboxConfig,
    Stdlib,
};

use crate::async_io;

pub struct LuaRuntime {
    lua: Lua,
    handler: LuaFunction,
}

pub fn create_runtime(lua_script: &str) -> LuaResult<Box<LuaRuntime>> {
    let mut lua = Lua::new(SafeOption::default());
    lua.open_stdlib(Stdlib::All)?;
    async_io::register_all(&mut lua)?;

    let mut sandbox = SandboxConfig::default();
    for name in ["sleep", "read_file", "write_file", "time", "env", "log"] {
        lua.sandbox_capture_global(&mut sandbox, name)?;
    }

    let handler: LuaFunction = lua
        .load_sandboxed(lua_script, &sandbox)
        .set_name("handler.lua")
        .eval()?;

    Ok(Box::new(LuaRuntime { lua, handler }))
}

impl LuaRuntime {
    pub async fn call_handler(
        &mut self,
        method: &str,
        path: &str,
        query: Option<&str>,
        headers_json: &str,
        body: &str,
    ) -> LuaResult<(u16, String, String)> {
        let handler = self.handler.clone();
        self.lua
            .call_async(
                &handler,
                (
                    method.to_string(),
                    path.to_string(),
                    query.unwrap_or_default().to_string(),
                    headers_json.to_string(),
                    body.to_string(),
                ),
            )
            .await
    }
}
