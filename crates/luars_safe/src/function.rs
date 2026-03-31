use luars::{FromLua, FromLuaMulti, IntoLua, LuaFunctionRef, LuaResult, LuaValue};

/// Safe wrapper around a callable Lua function handle.
pub struct Function {
    pub(crate) inner: LuaFunctionRef,
}

impl Function {
    pub(crate) fn new(inner: LuaFunctionRef) -> Self {
        Function { inner }
    }

    /// Call the function and convert all returned values.
    #[inline]
    pub fn call<A: IntoLua, R: FromLuaMulti>(&self, args: A) -> LuaResult<R> {
        self.inner.call(args)
    }

    /// Call the function and convert the first returned value.
    #[inline]
    pub fn call1<A: IntoLua, R: FromLua>(&self, args: A) -> LuaResult<R> {
        self.inner.call1(args)
    }
}

impl IntoLua for Function {
    #[inline]
    fn into_lua(self, state: &mut luars::LuaState) -> Result<usize, String> {
        state
            .push_value(self.inner.to_value())
            .map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

impl IntoLua for &Function {
    #[inline]
    fn into_lua(self, state: &mut luars::LuaState) -> Result<usize, String> {
        state
            .push_value(self.inner.to_value())
            .map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

impl FromLua for Function {
    fn from_lua(value: LuaValue, state: &mut luars::LuaState) -> Result<Self, String> {
        let actual = value.type_name();
        let function = state
            .to_function_ref(value)
            .ok_or_else(|| format!("expected function, got {}", actual))?;
        Ok(Function::new(function))
    }
}