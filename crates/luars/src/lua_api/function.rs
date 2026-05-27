use std::ffi::c_void;

use crate::{FromLua, FromLuaMulti, IntoLua, LuaFunctionRef, LuaResult, LuaValue};

/// Safe wrapper around a callable Lua function handle.
#[derive(Clone, Debug)]
pub struct LuaFunction {
    pub(crate) inner: LuaFunctionRef,
}

impl LuaFunction {
    pub(crate) fn new(inner: LuaFunctionRef) -> Self {
        LuaFunction { inner }
    }

    /// Return the number of upvalues captured by this function.
    #[inline]
    pub fn upvalue_count(&self) -> usize {
        self.inner.upvalue_count()
    }

    /// Read and convert one upvalue.
    #[inline]
    pub fn get_upvalue<T: FromLua>(&self, n: usize) -> LuaResult<Option<(String, T)>> {
        self.inner.get_upvalue(n)
    }

    /// Replace one upvalue.
    #[inline]
    pub fn set_upvalue<T: IntoLua>(&self, n: usize, value: T) -> LuaResult<Option<String>> {
        self.inner.set_upvalue(n, value)
    }

    /// Return an opaque identity for the requested upvalue.
    #[inline]
    pub fn upvalue_id(&self, n: usize) -> Option<*mut c_void> {
        self.inner.upvalue_id(n)
    }

    /// Make two Lua function upvalues share the same storage.
    #[inline]
    pub fn join_upvalue(&self, n1: usize, other: &LuaFunction, n2: usize) -> LuaResult<bool> {
        self.inner.join_upvalue(n1, &other.inner, n2)
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

    /// # Safety
    /// The returned `LuaValue` must not be used after the `LuaFunction` is
    pub unsafe fn to_value(&self) -> LuaValue {
        self.inner.to_value()
    }
}

impl IntoLua for LuaFunction {
    #[inline]
    fn into_lua(self, state: &mut luars::LuaState) -> Result<usize, String> {
        state
            .push_value(self.inner.to_value())
            .map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

impl IntoLua for &LuaFunction {
    #[inline]
    fn into_lua(self, state: &mut luars::LuaState) -> Result<usize, String> {
        state
            .push_value(self.inner.to_value())
            .map_err(|e| format!("{:?}", e))?;
        Ok(1)
    }
}

impl FromLua for LuaFunction {
    fn from_lua(value: LuaValue, state: &mut luars::LuaState) -> Result<Self, String> {
        let actual = value.type_name();
        let function = state
            .to_function_ref(value)
            .ok_or_else(|| format!("expected function, got {}", actual))?;
        Ok(LuaFunction::new(function))
    }
}
