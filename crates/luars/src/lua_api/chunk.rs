use crate::{FromLua, FromLuaMulti, Function, IntoLua, Lua, LuaResult};
#[cfg(feature = "sandbox")]
use luars::SandboxConfig;

/// Builder returned by `Lua::load`, similar to `mlua::Chunk`.
pub struct Chunk<'lua> {
    lua: &'lua mut Lua,
    source: String,
    name: Option<String>,
    #[cfg(feature = "sandbox")]
    sandbox: Option<SandboxConfig>,
}

impl<'lua> Chunk<'lua> {
    pub(crate) fn new(lua: &'lua mut Lua, source: &str) -> Self {
        Chunk {
            lua,
            source: source.to_owned(),
            name: None,
            #[cfg(feature = "sandbox")]
            sandbox: None,
        }
    }

    /// Set a chunk name used in diagnostics.
    pub fn set_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Execute this chunk inside a dedicated sandbox environment.
    #[cfg(feature = "sandbox")]
    pub fn with_sandbox(mut self, config: &SandboxConfig) -> Self {
        self.sandbox = Some(config.clone());
        self
    }

    fn compile_value(&mut self) -> LuaResult<luars::LuaValue> {
        #[cfg(feature = "sandbox")]
        if let Some(config) = self.sandbox.as_ref() {
            return match self.name.as_deref() {
                Some(name) => self
                    .lua
                    .load_sandboxed_value_with_name(&self.source, name, config),
                None => self.lua.load_sandboxed_value(&self.source, config),
            };
        }

        match self.name.as_deref() {
            Some(name) => self.lua.load_value_with_name(&self.source, name),
            None => self.lua.load_value(&self.source),
        }
    }

    /// Compile the chunk and return it as a callable function handle.
    pub fn into_function(mut self) -> LuaResult<Function> {
        let value = self.compile_value()?;
        self.lua.value_to_function(value)
    }

    /// Execute the chunk and discard returned values.
    pub fn exec(mut self) -> LuaResult<()> {
        let func = self.compile_value()?;
        self.lua.call_function_value(func).map(|_| ())
    }

    /// Execute the chunk asynchronously and discard returned values.
    pub async fn exec_async(mut self) -> LuaResult<()> {
        let func = self.compile_value()?;
        self.lua
            .call_function_value_async(func, vec![])
            .await
            .map(|_| ())
    }

    /// Execute the chunk and convert the first return value.
    pub fn eval<R: FromLua>(mut self) -> LuaResult<R> {
        let func = self.compile_value()?;
        let value = self
            .lua
            .call_function_value(func)?
            .into_iter()
            .next()
            .unwrap_or_else(luars::LuaValue::nil);
        self.lua.unpack_value(value, "chunk.eval")
    }

    /// Execute the chunk asynchronously and convert the first return value.
    pub async fn eval_async<R: FromLua>(mut self) -> LuaResult<R> {
        let func = self.compile_value()?;
        let value = self
            .lua
            .call_function_value_async(func, vec![])
            .await?
            .into_iter()
            .next()
            .unwrap_or_else(luars::LuaValue::nil);
        self.lua.unpack_value(value, "chunk.eval_async")
    }

    /// Execute the chunk and convert all returned values.
    pub fn eval_multi<R: FromLuaMulti>(mut self) -> LuaResult<R> {
        let func = self.compile_value()?;
        let values = self.lua.call_function_value(func)?;
        self.lua.unpack_multi_values(values, "chunk.eval_multi")
    }

    /// Execute the chunk asynchronously and convert all returned values.
    pub async fn eval_multi_async<R: FromLuaMulti>(mut self) -> LuaResult<R> {
        let func = self.compile_value()?;
        let values = self.lua.call_function_value_async(func, vec![]).await?;
        self.lua
            .unpack_multi_values(values, "chunk.eval_multi_async")
    }

    /// Compile the chunk and call it with Rust arguments.
    pub fn call<A: IntoLua, R: FromLuaMulti>(self, args: A) -> LuaResult<R> {
        self.into_function()?.call(args)
    }

    /// Compile the chunk and call it asynchronously with Rust arguments.
    pub async fn call_async<A: IntoLua, R: FromLuaMulti>(mut self, args: A) -> LuaResult<R> {
        let args = self.lua.pack_multi(args, "chunk.call_async")?;
        let func = self.compile_value()?;
        let values = self.lua.call_function_value_async(func, args).await?;
        self.lua.unpack_multi_values(values, "chunk.call_async")
    }

    /// Compile the chunk and call it, converting only the first return value.
    pub fn call1<A: IntoLua, R: FromLua>(self, args: A) -> LuaResult<R> {
        self.into_function()?.call1(args)
    }

    /// Compile the chunk and call it asynchronously, converting only the first return value.
    pub async fn call1_async<A: IntoLua, R: FromLua>(mut self, args: A) -> LuaResult<R> {
        let args = self.lua.pack_multi(args, "chunk.call1_async")?;
        let func = self.compile_value()?;
        let value = self
            .lua
            .call_function_value_async(func, args)
            .await?
            .into_iter()
            .next()
            .unwrap_or_else(luars::LuaValue::nil);
        self.lua.unpack_value(value, "chunk.call1_async")
    }
}
