use std::ffi::c_void;

mod chunk;
mod function;
mod lua;
mod lua_state;
mod lua_string;
mod scope;
mod table;
mod test;
mod value;

pub use chunk::Chunk;
pub use function::LuaFunction;
pub use lua::Lua;
pub use lua_string::LuaString;
pub use scope::{Scope, ScopedFunction};
pub use table::LuaTable;
pub use value::Value;

#[cfg(feature = "sandbox")]
use crate::SandboxConfig;
use crate::{
    FromLua, FromLuaMulti, IntoLua, LuaEnum, LuaError, LuaFullError, LuaRegistrable, LuaResult,
    LuaValue, LuaValueKind, RefAliveToken, Stdlib, UserDataRef, UserDataTrait,
    lua_vm::{LuaTypedAsyncCallback, LuaTypedCallback},
};

/// High-level, embedding-oriented API shared by safe host-side Lua handles.
///
/// This trait intentionally covers the typed, ergonomic surface. Low-level raw
/// runtime escape hatches such as `global_state` stay on the concrete type.
pub trait LuaApi {
    fn open_stdlib(&mut self, lib: Stdlib) -> LuaResult<()>;
    fn open_stdlibs(&mut self, libs: &[Stdlib]) -> LuaResult<()>;
    fn collect_garbage(&mut self) -> LuaResult<()>;
    fn execute(&mut self, source: &str) -> LuaResult<()>;
    fn dofile<R: FromLuaMulti>(&mut self, path: &str) -> LuaResult<R>;
    fn eval<R: FromLua>(&mut self, source: &str) -> LuaResult<R>;
    fn eval_multi<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R>;
    fn set_global<T: IntoLua>(&mut self, name: &str, value: T) -> LuaResult<()>;
    fn globals(&mut self) -> LuaTable;
    fn get_global<T: FromLua>(&mut self, name: &str) -> LuaResult<Option<T>>;
    fn call_global<A: IntoLua, R: FromLuaMulti>(&mut self, name: &str, args: A) -> LuaResult<R>;
    fn call_global1<A: IntoLua, R: FromLua>(&mut self, name: &str, args: A) -> LuaResult<R>;
    fn register_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedCallback<Args, R>;
    fn create_function<F, Args, R>(&mut self, f: F) -> LuaResult<LuaFunction>
    where
        F: LuaTypedCallback<Args, R>;
    fn register_async_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedAsyncCallback<Args, R>;
    fn register_type_of<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<()>;
    fn register_enum_of<T: LuaEnum>(&mut self, name: &str) -> LuaResult<()>;
    fn create_type_register_table<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<LuaTable>;
    fn load<'lua>(&'lua mut self, source: &str) -> Chunk<'lua, Self>
    where
        Self: Sized + chunk::ChunkHost;
    fn load_function(&mut self, source: &str) -> LuaResult<LuaFunction>;
    fn create_string(&mut self, value: &str) -> LuaResult<LuaString>;
    fn create_table(&mut self) -> LuaResult<LuaTable>;
    fn create_table_with_capacity(&mut self, narr: usize, nrec: usize) -> LuaResult<LuaTable>;
    fn create_userdata<T: UserDataTrait + 'static>(&mut self, data: T)
    -> LuaResult<UserDataRef<T>>;

    fn create_userdata_ref<T: UserDataTrait + 'static>(
        &mut self,
        reference: &mut T,
        alive_token: RefAliveToken,
    ) -> LuaResult<UserDataRef<T>>;
    fn create_table_from<K, V, I>(&mut self, iter: I) -> LuaResult<LuaTable>
    where
        K: IntoLua,
        V: IntoLua,
        I: IntoIterator<Item = (K, V)>;
    fn create_sequence_from<T, I>(&mut self, iter: I) -> LuaResult<LuaTable>
    where
        T: IntoLua,
        I: IntoIterator<Item = T>;

    fn pack<T: IntoLua>(&mut self, value: T) -> LuaResult<Value>;
    fn unpack<T: FromLua>(&mut self, value: Value) -> LuaResult<T>;
    fn convert<T: IntoLua, U: FromLua>(&mut self, value: T) -> LuaResult<U>;
    fn set_extra_space(&mut self, pointer: *mut c_void);
    fn extra_space(&self) -> *mut c_void;
    fn create_lightuserdata(&mut self, pointer: *mut c_void) -> Value;
    fn to_pointer<T: IntoLua>(&mut self, value: T) -> LuaResult<Option<*const c_void>>;
    fn registry(&mut self) -> LuaTable;
    fn registry_get<T: FromLua>(&mut self, key: &str) -> LuaResult<Option<T>>;
    fn registry_set<T: IntoLua>(&mut self, key: &str, value: T) -> LuaResult<()>;
    fn registry_geti<T: FromLua>(&mut self, key: i64) -> LuaResult<Option<T>>;

    fn get_type_metatable(&mut self, kind: LuaValueKind) -> Option<LuaTable>;
    fn set_type_metatable(
        &mut self,
        kind: LuaValueKind,
        metatable: Option<&LuaTable>,
    ) -> LuaResult<()>;
    fn get_error_message(&mut self, error: LuaError) -> LuaFullError;
    fn gc_stop(&mut self);
    fn gc_restart(&mut self);
}

pub(crate) trait StackValueApi {
    fn collect_values<T: IntoLua>(&mut self, value: T, api_name: &str) -> LuaResult<Vec<LuaValue>>;
    fn collect_single_value<T: IntoLua>(&mut self, value: T, api_name: &str)
    -> LuaResult<LuaValue>;
    #[allow(clippy::wrong_self_convention)]
    fn from_value<T: FromLua>(&mut self, value: LuaValue, api_name: &str) -> LuaResult<T>;
}

pub const LUA_REGISTRYINDEX: isize = -1001000;
pub const LUA_GLOBALSINDEX: isize = LUA_REGISTRYINDEX - 2;
pub const LUA_MULTRET: isize = -1;

#[inline]
pub const fn lua_upvalueindex(n: usize) -> isize {
    LUA_GLOBALSINDEX - n as isize
}

pub trait LuaStackApi {
    fn lua_gettop(&self) -> isize;
    fn lua_settop(&mut self, idx: isize) -> LuaResult<()>;
    fn lua_absindex(&self, idx: isize) -> Option<isize>;
    fn lua_rotate(&mut self, idx: isize, n: isize) -> LuaResult<()>;
    #[inline]
    fn lua_insert(&mut self, idx: isize) -> LuaResult<()> {
        self.lua_rotate(idx, 1)
    }
    #[inline]
    fn lua_remove(&mut self, idx: isize) -> LuaResult<()> {
        self.lua_rotate(idx, -1)?;
        self.lua_pop(1)
    }
    #[inline]
    fn lua_pop(&mut self, n: usize) -> LuaResult<()> {
        self.lua_settop(-(n as isize) - 1)
    }
    fn lua_type(&self, idx: isize) -> Option<LuaValueKind>;
    fn lua_typename(&self, idx: isize) -> Option<&'static str>;
    fn lua_isnone(&self, idx: isize) -> bool;
    fn lua_isnil(&self, idx: isize) -> bool;
    fn lua_isnoneornil(&self, idx: isize) -> bool;
    fn lua_isboolean(&self, idx: isize) -> bool;
    fn lua_isnumber(&self, idx: isize) -> bool;
    fn lua_isinteger(&self, idx: isize) -> bool;
    fn lua_isstring(&self, idx: isize) -> bool;
    fn lua_istable(&self, idx: isize) -> bool;
    fn lua_isfunction(&self, idx: isize) -> bool;
    fn lua_iscfunction(&self, idx: isize) -> bool;
    fn lua_isuserdata(&self, idx: isize) -> bool;
    fn lua_islightuserdata(&self, idx: isize) -> bool;
    fn lua_isthread(&self, idx: isize) -> bool;
    fn lua_pushvalue(&mut self, idx: isize) -> LuaResult<()>;
    fn lua_pushnil(&mut self) -> LuaResult<()>;
    fn lua_pushboolean(&mut self, value: bool) -> LuaResult<()>;
    fn lua_pushinteger(&mut self, value: i64) -> LuaResult<()>;
    fn lua_pushnumber(&mut self, value: f64) -> LuaResult<()>;
    fn lua_pushstring(&mut self, value: &str) -> LuaResult<()>;
    fn lua_pushlstring(&mut self, value: &[u8]) -> LuaResult<()>;
    fn lua_pushlightuserdata(&mut self, value: *mut c_void) -> LuaResult<()>;
    fn lua_pushcclosure(&mut self, func: crate::lua_vm::CFunction, n: usize) -> LuaResult<()>;
    #[inline]
    fn lua_pushcfunction(&mut self, func: crate::lua_vm::CFunction) -> LuaResult<()> {
        self.lua_pushcclosure(func, 0)
    }
    fn lua_pushrclosure<F>(&mut self, func: F, n: usize) -> LuaResult<()>
    where
        F: Fn(&mut crate::LuaState) -> LuaResult<usize> + 'static;
    fn lua_argcount(&self) -> isize;
    fn lua_toboolean(&self, idx: isize) -> bool;
    fn lua_tointegerx(&self, idx: isize) -> Option<i64>;
    fn lua_tonumberx(&self, idx: isize) -> Option<f64>;
    fn lua_tostring(&self, idx: isize) -> Option<&str>;
    fn lua_tolstring(&self, idx: isize) -> Option<&[u8]>;
    fn lua_tostring_handle(&mut self, idx: isize) -> Option<LuaString>;
    fn lua_l_checkany(&mut self, idx: isize) -> LuaResult<()>;
    fn lua_l_checkinteger(&mut self, idx: isize) -> LuaResult<i64>;
    fn lua_l_checknumber(&mut self, idx: isize) -> LuaResult<f64>;
    fn lua_l_checkstring(&mut self, idx: isize) -> LuaResult<String>;
    fn lua_l_checklstring(&mut self, idx: isize) -> LuaResult<Vec<u8>>;
    fn lua_l_optinteger(&mut self, idx: isize, default: i64) -> LuaResult<i64>;
    fn lua_l_optnumber(&mut self, idx: isize, default: f64) -> LuaResult<f64>;
    fn lua_l_optstring(&mut self, idx: isize, default: &str) -> LuaResult<String>;
    fn lua_l_optlstring(&mut self, idx: isize, default: &[u8]) -> LuaResult<Vec<u8>>;
    fn lua_createtable(&mut self, narr: usize, nrec: usize) -> LuaResult<()>;
    fn lua_newtable(&mut self) -> LuaResult<()>;
    fn lua_upvaluecount(&self, idx: isize) -> usize;
    fn lua_getupvalue(&mut self, func_idx: isize, n: usize) -> Option<String>;
    fn lua_setupvalue(&mut self, func_idx: isize, n: usize) -> LuaResult<Option<String>>;
    fn lua_pushuserdata<T: UserDataTrait + 'static>(&mut self, data: T) -> LuaResult<()>;
    fn lua_pushuserdata_ref<T: UserDataTrait + 'static>(
        &mut self,
        reference: &mut T,
        alive_token: RefAliveToken,
    ) -> LuaResult<()>;
    fn lua_touserdata_ref<T: 'static>(&mut self, idx: isize) -> Option<UserDataRef<T>>;
    fn lua_createthread(&mut self, func_idx: isize) -> LuaResult<()>;
    fn lua_pushthread(&mut self) -> LuaResult<bool>;
    fn lua_pushglobaltable(&mut self) -> LuaResult<()>;
    fn lua_call(&mut self, nargs: usize, nresults: isize) -> LuaResult<()>;
    fn lua_pcall(&mut self, nargs: usize, nresults: isize) -> LuaResult<bool>;
    fn lua_rawlen(&mut self, idx: isize) -> LuaResult<usize>;
    fn lua_len(&mut self, idx: isize) -> LuaResult<()>;
    fn lua_next(&mut self, idx: isize) -> LuaResult<bool>;
    fn lua_getglobal(&mut self, name: &str) -> LuaResult<()>;
    fn lua_setglobal(&mut self, name: &str) -> LuaResult<()>;
    fn lua_rawgetglobal(&mut self, name: &str) -> LuaResult<()>;
    fn lua_rawsetglobal(&mut self, name: &str) -> LuaResult<()>;
    fn lua_registry_geti(&mut self, index: i64) -> LuaResult<()>;
    fn lua_registry_seti(&mut self, index: i64) -> LuaResult<()>;
    fn lua_gettable(&mut self, idx: isize) -> LuaResult<()>;
    fn lua_settable(&mut self, idx: isize) -> LuaResult<()>;
    fn lua_getfield(&mut self, idx: isize, key: &str) -> LuaResult<()>;
    fn lua_setfield(&mut self, idx: isize, key: &str) -> LuaResult<()>;
    fn lua_geti(&mut self, idx: isize, key: i64) -> LuaResult<()>;
    fn lua_seti(&mut self, idx: isize, key: i64) -> LuaResult<()>;
    fn lua_rawget(&mut self, idx: isize) -> LuaResult<()>;
    fn lua_rawset(&mut self, idx: isize) -> LuaResult<()>;
    fn lua_rawgeti(&mut self, idx: isize, index: i64) -> LuaResult<()>;
    fn lua_rawseti(&mut self, idx: isize, index: i64) -> LuaResult<()>;
    fn lua_copy(&mut self, from_idx: isize, to_idx: isize) -> LuaResult<()>;
    fn lua_replace(&mut self, idx: isize) -> LuaResult<()>;
    fn lua_upvalueid(&mut self, func_idx: isize, n: usize) -> Option<*mut c_void>;
    fn lua_upvaluejoin(
        &mut self,
        func1_idx: isize,
        n1: usize,
        func2_idx: isize,
        n2: usize,
    ) -> LuaResult<bool>;
}

/// Async high-level API shared by safe host-side Lua handles.
#[allow(async_fn_in_trait)]
pub trait LuaAsyncApi {
    async fn exec_async(&mut self, source: &str) -> LuaResult<()>;
    async fn eval_async<R: FromLua>(&mut self, source: &str) -> LuaResult<R>;
    async fn eval_multi_async<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R>;
    async fn call_async<A: IntoLua, R: FromLuaMulti>(
        &mut self,
        function: &LuaFunction,
        args: A,
    ) -> LuaResult<R>;
    async fn call_async1<A: IntoLua, R: FromLua>(
        &mut self,
        function: &LuaFunction,
        args: A,
    ) -> LuaResult<R>;
    async fn call_async_global<A: IntoLua, R: FromLuaMulti>(
        &mut self,
        name: &str,
        args: A,
    ) -> LuaResult<R>;
    async fn call_async_global1<A: IntoLua, R: FromLua>(
        &mut self,
        name: &str,
        args: A,
    ) -> LuaResult<R>;
}

/// Sandbox-oriented high-level API shared by safe host-side Lua handles.
#[cfg(feature = "sandbox")]
pub trait LuaSandboxApi {
    fn load_sandboxed<'lua>(
        &'lua mut self,
        source: &str,
        config: &SandboxConfig,
    ) -> Chunk<'lua, Self>
    where
        Self: Sized + chunk::ChunkHost;
    fn execute_sandboxed(&mut self, source: &str, config: &SandboxConfig) -> LuaResult<()>;
    fn eval_sandboxed<R: FromLua>(&mut self, source: &str, config: &SandboxConfig) -> LuaResult<R>;
    fn eval_multi_sandboxed<R: FromLuaMulti>(
        &mut self,
        source: &str,
        config: &SandboxConfig,
    ) -> LuaResult<R>;
    fn sandbox_capture_global(&mut self, config: &mut SandboxConfig, name: &str) -> LuaResult<()>;
    fn sandbox_insert_global<T: IntoLua>(
        &mut self,
        config: &mut SandboxConfig,
        name: &str,
        value: T,
    ) -> LuaResult<()>;
}
