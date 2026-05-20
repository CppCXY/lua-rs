mod chunk;
mod function;
mod lua;
mod lua_string;
mod scope;
mod table;
mod test;
mod util;
mod value;

pub use chunk::Chunk;
pub use function::Function;
pub use lua::Lua;
pub use lua_string::LuaString;
pub use scope::{Scope, ScopedFunction, ScopedUserData};
pub use table::Table;
pub use value::Value;

use crate::{
    FromLua, FromLuaMulti, IntoLua, LuaEnum, LuaError, LuaFullError, LuaRegistrable, LuaResult,
    Stdlib, UserDataRef, UserDataTrait,
    lua_vm::{LuaTypedAsyncCallback, LuaTypedCallback},
};
#[cfg(feature = "sandbox")]
use crate::SandboxConfig;

/// High-level, embedding-oriented API shared by safe host-side Lua handles.
///
/// This trait intentionally covers the typed, ergonomic surface. Low-level raw
/// runtime escape hatches such as `vm_mut` stay on the concrete type.
pub trait LuaApi {
    fn open_stdlib(&mut self, lib: Stdlib) -> LuaResult<()>;
    fn load_stdlibs(&mut self, lib: Stdlib) -> LuaResult<()>;
    fn collect_garbage(&mut self) -> LuaResult<()>;
    fn execute(&mut self, source: &str) -> LuaResult<()>;
    fn eval<R: FromLua>(&mut self, source: &str) -> LuaResult<R>;
    fn eval_multi<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R>;
    fn set_global<T: IntoLua>(&mut self, name: &str, value: T) -> LuaResult<()>;
    fn globals(&mut self) -> Table;
    fn get_global<T: FromLua>(&mut self, name: &str) -> LuaResult<Option<T>>;
    fn call_global<A: IntoLua, R: FromLuaMulti>(&mut self, name: &str, args: A) -> LuaResult<R>;
    fn call_global1<A: IntoLua, R: FromLua>(&mut self, name: &str, args: A) -> LuaResult<R>;
    fn register_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedCallback<Args, R>;
    fn create_function<F, Args, R>(&mut self, f: F) -> LuaResult<Function>
    where
        F: LuaTypedCallback<Args, R>;
    fn register_async_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedAsyncCallback<Args, R>;
    fn register_type_of<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<()>;
    fn register_type<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<Table>;
    fn register_enum_of<T: LuaEnum>(&mut self, name: &str) -> LuaResult<()>;
    fn load<'lua>(&'lua mut self, source: &str) -> Chunk<'lua>;
    fn load_function(&mut self, source: &str) -> LuaResult<Function>;
    fn create_string(&mut self, value: &str) -> LuaResult<LuaString>;
    fn create_table(&mut self) -> LuaResult<Table>;
    fn create_table_with_capacity(&mut self, narr: usize, nrec: usize) -> LuaResult<Table>;
    fn create_userdata<T: UserDataTrait + 'static>(&mut self, data: T)
    -> LuaResult<UserDataRef<T>>;
    unsafe fn create_userdata_ref<T: UserDataTrait + 'static>(
        &mut self,
        reference: &mut T,
    ) -> LuaResult<UserDataRef<T>>;
    fn create_table_from<K, V, I>(&mut self, iter: I) -> LuaResult<Table>
    where
        K: IntoLua,
        V: IntoLua,
        I: IntoIterator<Item = (K, V)>;
    fn create_sequence_from<T, I>(&mut self, iter: I) -> LuaResult<Table>
    where
        T: IntoLua,
        I: IntoIterator<Item = T>;
    fn get_function(&mut self, name: &str) -> LuaResult<Option<Function>>;
    fn get_table(&mut self, name: &str) -> LuaResult<Option<Table>>;
    fn set_global_table(&mut self, name: &str, table: &Table) -> LuaResult<()>;
    fn set_global_function(&mut self, name: &str, function: &Function) -> LuaResult<()>;
    fn table_set<T: IntoLua>(&mut self, table: &Table, key: &str, value: T) -> LuaResult<()>;
    fn table_seti<T: IntoLua>(&mut self, table: &Table, key: i64, value: T) -> LuaResult<()>;
    fn table_get<T: FromLua>(&mut self, table: &Table, key: &str) -> LuaResult<T>;
    fn table_geti<T: FromLua>(&mut self, table: &Table, key: i64) -> LuaResult<T>;
    fn table_push<T: IntoLua>(&mut self, table: &Table, value: T) -> LuaResult<()>;
    fn table_pairs<K: FromLua, V: FromLua>(&mut self, table: &Table) -> LuaResult<Vec<(K, V)>>;
    fn table_array<T: FromLua>(&mut self, table: &Table) -> LuaResult<Vec<T>>;
    fn pack<T: IntoLua>(&mut self, value: T) -> LuaResult<Value>;
    fn unpack<T: FromLua>(&mut self, value: Value) -> LuaResult<T>;
    fn convert<T: IntoLua, U: FromLua>(&mut self, value: T) -> LuaResult<U>;
    fn get_error_message(&mut self, error: LuaError) -> LuaFullError;
    fn gc_stop(&mut self);
    fn gc_restart(&mut self);
}

/// Async high-level API shared by safe host-side Lua handles.
#[allow(async_fn_in_trait)]
pub trait LuaAsyncApi {
    async fn exec_async(&mut self, source: &str) -> LuaResult<()>;
    async fn eval_async<R: FromLua>(&mut self, source: &str) -> LuaResult<R>;
    async fn eval_multi_async<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R>;
    async fn call_async<A: IntoLua, R: FromLuaMulti>(
        &mut self,
        function: &Function,
        args: A,
    ) -> LuaResult<R>;
    async fn call_async1<A: IntoLua, R: FromLua>(
        &mut self,
        function: &Function,
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
    fn load_sandboxed<'lua>(&'lua mut self, source: &str, config: &SandboxConfig) -> Chunk<'lua>;
    fn execute_sandboxed(&mut self, source: &str, config: &SandboxConfig) -> LuaResult<()>;
    fn eval_sandboxed<R: FromLua>(&mut self, source: &str, config: &SandboxConfig)
    -> LuaResult<R>;
    fn eval_multi_sandboxed<R: FromLuaMulti>(
        &mut self,
        source: &str,
        config: &SandboxConfig,
    ) -> LuaResult<R>;
    fn sandbox_capture_global(
        &mut self,
        config: &mut SandboxConfig,
        name: &str,
    ) -> LuaResult<()>;
    fn sandbox_insert_global<T: IntoLua>(
        &mut self,
        config: &mut SandboxConfig,
        name: &str,
        value: T,
    ) -> LuaResult<()>;
}
