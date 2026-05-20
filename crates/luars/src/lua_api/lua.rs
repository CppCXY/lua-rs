use std::ffi::c_void;
use std::pin::Pin;

#[cfg(feature = "sandbox")]
use luars::SandboxConfig;
use luars::lua_vm::SafeOption;
use luars::lua_vm::{LuaTypedAsyncCallback, LuaTypedCallback};
use luars::{
    FromLua, FromLuaMulti, GlobalState, IntoLua, LuaEnum, LuaLibrary, LuaRegistrable, LuaResult,
    LuaUserdata, LuaValueKind, Stdlib, UserDataRef, UserDataTrait,
};

#[cfg(feature = "sandbox")]
use crate::LuaSandboxApi;
use crate::lua_api::util::{collect_values, from_value, into_single_value};
use crate::lua_api::{Chunk, Function, LuaString, Scope, Table, Value};
use crate::{LuaApi, LuaAsyncApi, LuaError, LuaFullError};

/// Safe, embedding-oriented Lua runtime.
///
/// This type sits on top of the low-level runtime and exposes a narrower API that
/// avoids raw `LuaValue` plumbing in the common host-facing surface.
pub struct Lua {
    global_state_owner: Pin<Box<GlobalState>>,
}

impl Lua {
    /// Create a new Lua runtime.
    pub fn new(option: SafeOption) -> Self {
        Lua {
            global_state_owner: GlobalState::new(option),
        }
    }

    /// Install a library provided by luars or an external crate.
    #[inline]
    pub fn install_library<L: LuaLibrary>(&mut self, library: L) -> LuaResult<()> {
        library.install(self)
    }

    pub(crate) fn load_value(&mut self, source: &str) -> LuaResult<luars::LuaValue> {
        self.global_state_owner.main_state().load(source)
    }

    pub(crate) fn load_value_with_name(
        &mut self,
        source: &str,
        chunk_name: &str,
    ) -> LuaResult<luars::LuaValue> {
        self.global_state_owner
            .main_state()
            .load_with_name(source, chunk_name)
    }

    pub(crate) fn value_to_function(&mut self, value: luars::LuaValue) -> LuaResult<Function> {
        let function = self
            .global_state_owner
            .to_function_ref(value)
            .ok_or_else(|| {
                self.global_state_owner
                    .main_state()
                    .error("compiled chunk is not a function".to_string())
            })?;
        Ok(Function::new(function))
    }

    pub(crate) fn value_to_string(&mut self, value: luars::LuaValue) -> LuaResult<LuaString> {
        let string = self
            .global_state_owner
            .to_string_ref(value)
            .ok_or_else(|| {
                self.global_state_owner
                    .main_state()
                    .error("value is not a string".to_string())
            })?;
        Ok(LuaString::new(string))
    }

    pub(crate) fn value_to_userdata<T: 'static>(
        &mut self,
        value: luars::LuaValue,
    ) -> LuaResult<UserDataRef<T>> {
        self.global_state_owner
            .to_userdata_ref(value)
            .ok_or_else(|| {
                self.global_state_owner
                    .main_state()
                    .error("value is not the expected userdata type".to_string())
            })
    }

    pub(crate) fn call_function_value(
        &mut self,
        func: luars::LuaValue,
    ) -> LuaResult<Vec<luars::LuaValue>> {
        self.global_state_owner.main_state().call(func, vec![])
    }

    pub(crate) async fn call_function_value_async(
        &mut self,
        func: luars::LuaValue,
        args: Vec<luars::LuaValue>,
    ) -> LuaResult<Vec<luars::LuaValue>> {
        self.global_state_owner
            .main_state()
            .call_async(func, args)
            .await
    }

    pub(crate) fn pack_multi<T: IntoLua>(
        &mut self,
        value: T,
        _api_name: &str,
    ) -> LuaResult<Vec<luars::LuaValue>> {
        collect_values(&mut self.global_state_owner, value)
    }

    #[cfg(feature = "sandbox")]
    pub(crate) fn load_sandboxed_value(
        &mut self,
        source: &str,
        config: &SandboxConfig,
    ) -> LuaResult<luars::LuaValue> {
        self.global_state_owner
            .main_state()
            .load_sandboxed(source, config)
    }

    #[cfg(feature = "sandbox")]
    pub(crate) fn load_sandboxed_value_with_name(
        &mut self,
        source: &str,
        chunk_name: &str,
        config: &SandboxConfig,
    ) -> LuaResult<luars::LuaValue> {
        self.global_state_owner
            .main_state()
            .load_with_name_sandboxed(source, chunk_name, config)
    }

    pub(crate) fn unpack_multi_values<R: FromLuaMulti>(
        &mut self,
        values: Vec<luars::LuaValue>,
        api_name: &str,
    ) -> LuaResult<R> {
        R::from_lua_multi(values, self.global_state_owner.main_state()).map_err(|msg| {
            self.global_state_owner
                .main_state()
                .error(format!("{}: {}", api_name, msg))
        })
    }

    pub(crate) fn unpack_value<T: FromLua>(
        &mut self,
        value: luars::LuaValue,
        api_name: &str,
    ) -> LuaResult<T> {
        from_value(&mut self.global_state_owner, value, api_name)
    }

    /// Run a lexical scope that can create non-`'static` Lua callbacks and borrowed userdata.
    pub fn scope<'lua, R>(
        &'lua mut self,
        f: impl for<'scope> FnOnce(&mut Scope<'scope, 'lua>) -> LuaResult<R>,
    ) -> LuaResult<R> {
        let mut scope = Scope::new(self);
        f(&mut scope)
    }

    pub(crate) fn create_raw_function<F>(&mut self, f: F) -> LuaResult<Function>
    where
        F: Fn(&mut luars::LuaState) -> LuaResult<usize> + 'static,
    {
        let value = self.global_state_owner.create_closure(f)?;
        self.value_to_function(value)
    }

    pub(crate) fn create_userdata_value<T: UserDataTrait + 'static>(
        &mut self,
        data: T,
    ) -> LuaResult<Value> {
        let value = self
            .global_state_owner
            .create_userdata(LuaUserdata::new(data))?;
        Ok(Value::new(self.global_state_owner.to_ref(value)))
    }

    /// Get a mutable reference to the underlying GlobalState for advanced use cases.
    pub fn global_state_mut(&mut self) -> &mut GlobalState {
        &mut self.global_state_owner
    }
}

impl Default for Lua {
    fn default() -> Self {
        Self::new(SafeOption::default())
    }
}

impl LuaApi for Lua {
    #[inline]
    fn open_stdlib(&mut self, lib: Stdlib) -> LuaResult<()> {
        self.global_state_owner.open_stdlib(lib)
    }

    #[inline]
    fn load_stdlibs(&mut self, lib: Stdlib) -> LuaResult<()> {
        self.open_stdlib(lib)
    }

    #[inline]
    fn collect_garbage(&mut self) -> LuaResult<()> {
        self.global_state_owner.main_state().collect_garbage()
    }

    #[inline]
    fn execute(&mut self, source: &str) -> LuaResult<()> {
        self.global_state_owner
            .main_state()
            .execute(source)
            .map(|_| ())
    }

    #[inline]
    fn eval<R: FromLua>(&mut self, source: &str) -> LuaResult<R> {
        let values = self.global_state_owner.main_state().execute(source)?;
        let value = values
            .into_iter()
            .next()
            .unwrap_or_else(luars::LuaValue::nil);
        from_value(&mut self.global_state_owner, value, "eval")
    }

    #[inline]
    fn eval_multi<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R> {
        let values = self.global_state_owner.main_state().execute(source)?;
        R::from_lua_multi(values, self.global_state_owner.main_state())
            .map_err(|msg| self.global_state_owner.main_state().error(msg))
    }

    #[inline]
    fn set_global<T: IntoLua>(&mut self, name: &str, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.global_state_owner, value, "set_global")?;
        self.global_state_owner.set_global(name, value)
    }

    #[inline]
    fn globals(&mut self) -> Table {
        Table::new(self.global_state_owner.globals_table())
    }

    #[inline]
    fn get_global<T: FromLua>(&mut self, name: &str) -> LuaResult<Option<T>> {
        self.global_state_owner.main_state().get_global_as(name)
    }

    #[inline]
    fn call_global<A: IntoLua, R: FromLuaMulti>(&mut self, name: &str, args: A) -> LuaResult<R> {
        let values = collect_values(&mut self.global_state_owner, args)?;
        self.global_state_owner
            .main_state()
            .call_global(name, values)
            .and_then(|values| self.unpack_multi_values(values, "call_global"))
    }

    #[inline]
    fn call_global1<A: IntoLua, R: FromLua>(&mut self, name: &str, args: A) -> LuaResult<R> {
        let args = collect_values(&mut self.global_state_owner, args)?;
        let values = self
            .global_state_owner
            .main_state()
            .call_global(name, args)?;
        let value = values
            .into_iter()
            .next()
            .unwrap_or_else(luars::LuaValue::nil);
        self.unpack_value(value, "call_global1")
    }

    #[inline]
    fn register_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedCallback<Args, R>,
    {
        self.global_state_owner.register_function_typed(name, f)
    }

    #[inline]
    fn create_function<F, Args, R>(&mut self, f: F) -> LuaResult<Function>
    where
        F: LuaTypedCallback<Args, R>,
    {
        self.global_state_owner
            .create_function_typed(f)
            .map(Function::new)
    }

    #[inline]
    fn register_async_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedAsyncCallback<Args, R>,
    {
        self.global_state_owner.register_async_typed(name, f)
    }

    #[inline]
    fn register_type_of<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<()> {
        self.global_state_owner.register_type_of::<T>(name)
    }

    #[inline]
    fn register_type<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<Table> {
        self.global_state_owner.register_type_of::<T>(name)?;
        self.get_table(name)?.ok_or_else(|| {
            self.global_state_owner.main_state().error(format!(
                "registered type '{}' did not produce a table",
                name
            ))
        })
    }

    #[inline]
    fn register_enum_of<T: LuaEnum>(&mut self, name: &str) -> LuaResult<()> {
        self.global_state_owner.register_enum_of::<T>(name)
    }

    #[inline]
    fn load<'lua>(&'lua mut self, source: &str) -> Chunk<'lua, Self> {
        Chunk::new(self, source)
    }

    #[inline]
    fn load_function(&mut self, source: &str) -> LuaResult<Function> {
        self.load(source).into_function()
    }

    #[inline]
    fn create_string(&mut self, value: &str) -> LuaResult<LuaString> {
        let value = self.global_state_owner.create_string(value)?;
        self.value_to_string(value)
    }

    #[inline]
    fn create_table(&mut self) -> LuaResult<Table> {
        self.create_table_with_capacity(0, 0)
    }

    #[inline]
    fn create_table_with_capacity(&mut self, narr: usize, nrec: usize) -> LuaResult<Table> {
        self.global_state_owner
            .create_table_ref(narr, nrec)
            .map(Table::new)
    }

    #[inline]
    fn create_userdata<T: UserDataTrait + 'static>(
        &mut self,
        data: T,
    ) -> LuaResult<UserDataRef<T>> {
        let value = self
            .global_state_owner
            .create_userdata(LuaUserdata::new(data))?;
        self.value_to_userdata(value)
    }

    #[inline]
    unsafe fn create_userdata_ref<T: UserDataTrait + 'static>(
        &mut self,
        reference: &mut T,
    ) -> LuaResult<UserDataRef<T>> {
        let ud = unsafe { LuaUserdata::from_ref(reference) };
        let value = self.global_state_owner.create_userdata(ud)?;
        self.value_to_userdata(value)
    }

    #[inline]
    fn create_table_from<K, V, I>(&mut self, iter: I) -> LuaResult<Table>
    where
        K: IntoLua,
        V: IntoLua,
        I: IntoIterator<Item = (K, V)>,
    {
        let table = self.create_table()?;
        for (key, value) in iter {
            table.set(key, value)?;
        }
        Ok(table)
    }

    #[inline]
    fn create_sequence_from<T, I>(&mut self, iter: I) -> LuaResult<Table>
    where
        T: IntoLua,
        I: IntoIterator<Item = T>,
    {
        let table = self.create_table()?;
        for value in iter {
            table.push(value)?;
        }
        Ok(table)
    }

    #[inline]
    fn get_function(&mut self, name: &str) -> LuaResult<Option<Function>> {
        self.global_state_owner
            .get_global_function(name)
            .map(|opt| opt.map(Function::new))
    }

    #[inline]
    fn get_table(&mut self, name: &str) -> LuaResult<Option<Table>> {
        self.global_state_owner
            .get_global_table(name)
            .map(|opt| opt.map(Table::new))
    }

    #[inline]
    fn set_global_table(&mut self, name: &str, table: &Table) -> LuaResult<()> {
        self.global_state_owner.set_global(name, table.value())
    }

    #[inline]
    fn set_global_function(&mut self, name: &str, function: &Function) -> LuaResult<()> {
        self.global_state_owner
            .set_global(name, function.inner.to_value())
    }

    #[inline]
    fn table_set<T: IntoLua>(&mut self, table: &Table, key: &str, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.global_state_owner, value, "table_set")?;
        table.inner.set(key, value)
    }

    #[inline]
    fn table_seti<T: IntoLua>(&mut self, table: &Table, key: i64, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.global_state_owner, value, "table_seti")?;
        table.inner.seti(key, value)
    }

    #[inline]
    fn table_get<T: FromLua>(&mut self, table: &Table, key: &str) -> LuaResult<T> {
        let value = table.inner.get(key)?;
        from_value(&mut self.global_state_owner, value, "table_get")
    }

    #[inline]
    fn table_geti<T: FromLua>(&mut self, table: &Table, key: i64) -> LuaResult<T> {
        let value = table.inner.geti(key)?;
        from_value(&mut self.global_state_owner, value, "table_geti")
    }

    #[inline]
    fn table_push<T: IntoLua>(&mut self, table: &Table, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.global_state_owner, value, "table_push")?;
        table.inner.push(value)
    }

    #[inline]
    fn table_pairs<K: FromLua, V: FromLua>(&mut self, table: &Table) -> LuaResult<Vec<(K, V)>> {
        table.pairs()
    }

    #[inline]
    fn table_array<T: FromLua>(&mut self, table: &Table) -> LuaResult<Vec<T>> {
        table.sequence_values()
    }

    #[inline]
    fn get_metatable<T: IntoLua>(&mut self, value: T) -> LuaResult<Option<Table>> {
        Ok(self.pack(value)?.get_metatable())
    }

    #[inline]
    fn set_metatable<T: IntoLua>(&mut self, value: T, metatable: Option<&Table>) -> LuaResult<()> {
        let value = self.pack(value)?;
        if let Some(table) = value.as_table() {
            return table.set_metatable(metatable);
        }
        value.set_metatable(metatable)
    }

    #[inline]
    fn pack<T: IntoLua>(&mut self, value: T) -> LuaResult<Value> {
        let value = into_single_value(&mut self.global_state_owner, value, "pack")?;
        Ok(Value::new(self.global_state_owner.to_ref(value)))
    }

    #[inline]
    fn unpack<T: FromLua>(&mut self, value: Value) -> LuaResult<T> {
        self.unpack_value(value.to_value(), "unpack")
    }

    #[inline]
    fn convert<T: IntoLua, U: FromLua>(&mut self, value: T) -> LuaResult<U> {
        let value = into_single_value(&mut self.global_state_owner, value, "convert")?;
        self.unpack_value(value, "convert")
    }

    #[inline]
    fn create_lightuserdata(&mut self, pointer: *mut c_void) -> Value {
        Value::new(
            self.global_state_owner
                .to_ref(luars::LuaValue::lightuserdata(pointer)),
        )
    }

    #[inline]
    fn to_pointer<T: IntoLua>(&mut self, value: T) -> LuaResult<Option<*const c_void>> {
        Ok(self.pack(value)?.to_pointer())
    }

    #[inline]
    fn registry(&mut self) -> Table {
        let registry = self.global_state_owner.registry;
        Table::new(
            self.global_state_owner
                .to_table_ref(registry)
                .expect("registry must be a table"),
        )
    }

    #[inline]
    fn registry_get<T: FromLua>(&mut self, key: &str) -> LuaResult<Option<T>> {
        let Some(value) = self.global_state_owner.registry_get(key)? else {
            return Ok(None);
        };
        self.unpack_value(value, "registry_get").map(Some)
    }

    #[inline]
    fn registry_set<T: IntoLua>(&mut self, key: &str, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.global_state_owner, value, "registry_set")?;
        self.global_state_owner.registry_set(key, value)
    }

    #[inline]
    fn registry_geti<T: FromLua>(&mut self, key: i64) -> LuaResult<Option<T>> {
        let Some(value) = self.global_state_owner.registry_geti(key) else {
            return Ok(None);
        };
        self.unpack_value(value, "registry_geti").map(Some)
    }

    #[inline]
    fn registry_seti<T: IntoLua>(&mut self, key: i64, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.global_state_owner, value, "registry_seti")?;
        self.global_state_owner.registry_seti(key, value);
        Ok(())
    }

    #[inline]
    fn get_type_metatable(&mut self, kind: LuaValueKind) -> Option<Table> {
        let metatable = self.global_state_owner.get_basic_metatable(kind)?;
        self.global_state_owner
            .to_table_ref(metatable)
            .map(Table::new)
    }

    #[inline]
    fn set_type_metatable(
        &mut self,
        kind: LuaValueKind,
        metatable: Option<&Table>,
    ) -> LuaResult<()> {
        self.global_state_owner
            .set_basic_metatable(kind, metatable.map(Table::value));
        Ok(())
    }

    #[inline]
    fn get_error_message(&mut self, error: LuaError) -> LuaFullError {
        self.global_state_owner.main_state().get_full_error(error)
    }

    #[inline]
    fn gc_stop(&mut self) {
        self.global_state_mut().gc.gc_stopped = true;
    }

    #[inline]
    fn gc_restart(&mut self) {
        self.global_state_mut().gc.gc_stopped = false;
        self.global_state_mut().gc.set_debt(0);
    }
}

impl LuaAsyncApi for Lua {
    async fn exec_async(&mut self, source: &str) -> LuaResult<()> {
        self.load(source).exec_async().await
    }

    async fn eval_async<R: FromLua>(&mut self, source: &str) -> LuaResult<R> {
        self.load(source).eval_async().await
    }

    async fn eval_multi_async<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R> {
        self.load(source).eval_multi_async().await
    }

    async fn call_async<A: IntoLua, R: FromLuaMulti>(
        &mut self,
        function: &Function,
        args: A,
    ) -> LuaResult<R> {
        let args = self.pack_multi(args, "call_async")?;
        let values = self
            .call_function_value_async(function.inner.to_value(), args)
            .await?;
        self.unpack_multi_values(values, "call_async")
    }

    async fn call_async1<A: IntoLua, R: FromLua>(
        &mut self,
        function: &Function,
        args: A,
    ) -> LuaResult<R> {
        let args = self.pack_multi(args, "call_async1")?;
        let value = self
            .call_function_value_async(function.inner.to_value(), args)
            .await?
            .into_iter()
            .next()
            .unwrap_or_else(luars::LuaValue::nil);
        self.unpack_value(value, "call_async1")
    }

    async fn call_async_global<A: IntoLua, R: FromLuaMulti>(
        &mut self,
        name: &str,
        args: A,
    ) -> LuaResult<R> {
        let function = self.get_function(name)?.ok_or_else(|| {
            self.global_state_owner
                .main_state()
                .error(format!("global '{}' not found", name))
        })?;
        self.call_async(&function, args).await
    }

    async fn call_async_global1<A: IntoLua, R: FromLua>(
        &mut self,
        name: &str,
        args: A,
    ) -> LuaResult<R> {
        let function = self.get_function(name)?.ok_or_else(|| {
            self.global_state_owner
                .main_state()
                .error(format!("global '{}' not found", name))
        })?;
        self.call_async1(&function, args).await
    }
}

#[cfg(feature = "sandbox")]
impl LuaSandboxApi for Lua {
    fn load_sandboxed<'lua>(
        &'lua mut self,
        source: &str,
        config: &SandboxConfig,
    ) -> Chunk<'lua, Self> {
        self.load(source).with_sandbox(config)
    }

    fn execute_sandboxed(&mut self, source: &str, config: &SandboxConfig) -> LuaResult<()> {
        self.global_state_owner
            .main_state()
            .execute_sandboxed(source, config)
            .map(|_| ())
    }

    fn eval_sandboxed<R: FromLua>(&mut self, source: &str, config: &SandboxConfig) -> LuaResult<R> {
        let value = self
            .global_state_owner
            .main_state()
            .execute_sandboxed(source, config)?
            .into_iter()
            .next()
            .unwrap_or_else(luars::LuaValue::nil);
        self.unpack_value(value, "eval_sandboxed")
    }

    fn eval_multi_sandboxed<R: FromLuaMulti>(
        &mut self,
        source: &str,
        config: &SandboxConfig,
    ) -> LuaResult<R> {
        let values = self
            .global_state_owner
            .main_state()
            .execute_sandboxed(source, config)?;
        self.unpack_multi_values(values, "eval_multi_sandboxed")
    }

    fn sandbox_capture_global(&mut self, config: &mut SandboxConfig, name: &str) -> LuaResult<()> {
        let value = self.global_state_owner.get_global(name)?.ok_or_else(|| {
            self.global_state_owner
                .main_state()
                .error(format!("global '{}' not found", name))
        })?;
        config.insert_global(name, value);
        Ok(())
    }

    fn sandbox_insert_global<T: IntoLua>(
        &mut self,
        config: &mut SandboxConfig,
        name: &str,
        value: T,
    ) -> LuaResult<()> {
        let value =
            into_single_value(&mut self.global_state_owner, value, "sandbox_insert_global")?;
        config.insert_global(name, value);
        Ok(())
    }
}
