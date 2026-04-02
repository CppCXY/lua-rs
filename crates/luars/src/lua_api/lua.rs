#[cfg(feature = "sandbox")]
use luars::SandboxConfig;
use luars::lua_vm::SafeOption;
use luars::lua_vm::{LuaTypedAsyncCallback, LuaTypedCallback};
use luars::{
    FromLua, FromLuaMulti, IntoLua, LuaEnum, LuaRegistrable, LuaResult, LuaUserdata, LuaVM, Stdlib,
    UserDataRef, UserDataTrait,
};

use crate::lua_api::util::{collect_values, from_value, into_single_value};
use crate::lua_api::{Chunk, Function, LuaString, Scope, Table, Value};

/// Safe, embedding-oriented Lua runtime.
///
/// This type sits on top of `luars::LuaVM` and exposes a narrower API that
/// avoids raw `LuaValue` plumbing in the common host-facing surface.
pub struct Lua {
    vm: Box<LuaVM>,
}

impl Lua {
    /// Create a new Lua runtime.
    pub fn new(option: SafeOption) -> Self {
        Lua {
            vm: LuaVM::new(option),
        }
    }

    /// Open standard libraries.
    #[inline]
    pub fn open_stdlib(&mut self, lib: Stdlib) -> LuaResult<()> {
        self.vm.open_stdlib(lib)
    }

    /// Open standard libraries.
    #[inline]
    pub fn load_stdlibs(&mut self, lib: Stdlib) -> LuaResult<()> {
        self.open_stdlib(lib)
    }

    /// Execute source code and discard raw return values.
    #[inline]
    pub fn execute(&mut self, source: &str) -> LuaResult<()> {
        self.vm.execute(source).map(|_| ())
    }

    pub(crate) fn load_value(&mut self, source: &str) -> LuaResult<luars::LuaValue> {
        self.vm.load(source)
    }

    pub(crate) fn load_value_with_name(
        &mut self,
        source: &str,
        chunk_name: &str,
    ) -> LuaResult<luars::LuaValue> {
        self.vm.load_with_name(source, chunk_name)
    }

    pub(crate) fn value_to_function(&mut self, value: luars::LuaValue) -> LuaResult<Function> {
        let function = self
            .vm
            .to_function_ref(value)
            .ok_or_else(|| self.vm.error("compiled chunk is not a function"))?;
        Ok(Function::new(function))
    }

    pub(crate) fn value_to_string(&mut self, value: luars::LuaValue) -> LuaResult<LuaString> {
        let string = self
            .vm
            .to_string_ref(value)
            .ok_or_else(|| self.vm.error("value is not a string"))?;
        Ok(LuaString::new(string))
    }

    pub(crate) fn value_to_userdata<T: 'static>(
        &mut self,
        value: luars::LuaValue,
    ) -> LuaResult<UserDataRef<T>> {
        self.vm
            .to_userdata_ref(value)
            .ok_or_else(|| self.vm.error("value is not the expected userdata type"))
    }

    pub(crate) fn call_function_value(
        &mut self,
        func: luars::LuaValue,
    ) -> LuaResult<Vec<luars::LuaValue>> {
        self.vm.call_raw(func, vec![])
    }

    pub(crate) async fn call_function_value_async(
        &mut self,
        func: luars::LuaValue,
        args: Vec<luars::LuaValue>,
    ) -> LuaResult<Vec<luars::LuaValue>> {
        self.vm.call_async(func, args).await
    }

    pub(crate) fn pack_multi<T: IntoLua>(
        &mut self,
        value: T,
        _api_name: &str,
    ) -> LuaResult<Vec<luars::LuaValue>> {
        collect_values(&mut self.vm, value)
    }

    #[cfg(feature = "sandbox")]
    pub(crate) fn load_sandboxed_value(
        &mut self,
        source: &str,
        config: &SandboxConfig,
    ) -> LuaResult<luars::LuaValue> {
        self.vm.load_sandboxed(source, config)
    }

    #[cfg(feature = "sandbox")]
    pub(crate) fn load_sandboxed_value_with_name(
        &mut self,
        source: &str,
        chunk_name: &str,
        config: &SandboxConfig,
    ) -> LuaResult<luars::LuaValue> {
        self.vm.load_with_name_sandboxed(source, chunk_name, config)
    }

    pub(crate) fn unpack_multi_values<R: FromLuaMulti>(
        &mut self,
        values: Vec<luars::LuaValue>,
        api_name: &str,
    ) -> LuaResult<R> {
        R::from_lua_multi(values, self.vm.main_state())
            .map_err(|msg| self.vm.error(format!("{}: {}", api_name, msg)))
    }

    pub(crate) fn unpack_value<T: FromLua>(
        &mut self,
        value: luars::LuaValue,
        api_name: &str,
    ) -> LuaResult<T> {
        from_value(&mut self.vm, value, api_name)
    }

    /// Execute source code and convert the first returned value.
    pub fn eval<R: FromLua>(&mut self, source: &str) -> LuaResult<R> {
        let values = self.vm.execute(source)?;
        let value = values
            .into_iter()
            .next()
            .unwrap_or_else(luars::LuaValue::nil);
        from_value(&mut self.vm, value, "eval")
    }

    /// Execute source code and convert all returned values.
    pub fn eval_multi<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R> {
        let values = self.vm.execute(source)?;
        R::from_lua_multi(values, self.vm.main_state()).map_err(|msg| self.vm.error(msg))
    }

    /// Execute source code asynchronously and discard raw return values.
    pub async fn exec_async(&mut self, source: &str) -> LuaResult<()> {
        self.load(source).exec_async().await
    }

    /// Execute source code asynchronously and convert the first returned value.
    pub async fn eval_async<R: FromLua>(&mut self, source: &str) -> LuaResult<R> {
        self.load(source).eval_async().await
    }

    /// Execute source code asynchronously and convert all returned values.
    pub async fn eval_multi_async<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R> {
        self.load(source).eval_multi_async().await
    }

    /// Set a global from a Rust value that converts to a single Lua value.
    pub fn set_global<T: IntoLua>(&mut self, name: &str, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.vm, value, "set_global")?;
        self.vm.set_global(name, value)
    }

    /// Return a handle to the global environment table.
    #[inline]
    pub fn globals(&mut self) -> Table {
        Table::new(self.vm.globals_table())
    }

    /// Get and convert a global variable.
    #[inline]
    pub fn get_global<T: FromLua>(&mut self, name: &str) -> LuaResult<Option<T>> {
        self.vm.get_global_as(name)
    }

    /// Call a global function and convert all results.
    #[inline]
    pub fn call_global<A: IntoLua, R: FromLuaMulti>(
        &mut self,
        name: &str,
        args: A,
    ) -> LuaResult<R> {
        self.vm.call_global(name, args)
    }

    /// Call a global function and convert the first result.
    #[inline]
    pub fn call_global1<A: IntoLua, R: FromLua>(&mut self, name: &str, args: A) -> LuaResult<R> {
        self.vm.call1_global(name, args)
    }

    /// Call a function handle asynchronously and convert all results.
    pub async fn call_async<A: IntoLua, R: FromLuaMulti>(
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

    /// Call a function handle asynchronously and convert only the first result.
    pub async fn call_async1<A: IntoLua, R: FromLua>(
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

    /// Call a global function asynchronously and convert all results.
    pub async fn call_async_global<A: IntoLua, R: FromLuaMulti>(
        &mut self,
        name: &str,
        args: A,
    ) -> LuaResult<R> {
        let function = self
            .get_function(name)?
            .ok_or_else(|| self.vm.error(format!("global '{}' not found", name)))?;
        self.call_async(&function, args).await
    }

    /// Call a global function asynchronously and convert only the first result.
    pub async fn call_async_global1<A: IntoLua, R: FromLua>(
        &mut self,
        name: &str,
        args: A,
    ) -> LuaResult<R> {
        let function = self
            .get_function(name)?
            .ok_or_else(|| self.vm.error(format!("global '{}' not found", name)))?;
        self.call_async1(&function, args).await
    }

    /// Register a typed Rust callback as a Lua global.
    #[inline]
    pub fn register_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedCallback<Args, R>,
    {
        self.vm.register_function_typed(name, f)
    }

    /// Create a typed Rust callback as a standalone Lua function handle.
    #[inline]
    pub fn create_function<F, Args, R>(&mut self, f: F) -> LuaResult<Function>
    where
        F: LuaTypedCallback<Args, R>,
    {
        self.vm.create_function_typed(f).map(Function::new)
    }

    /// Register a typed async Rust callback as a Lua global.
    #[inline]
    pub fn register_async_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedAsyncCallback<Args, R>,
    {
        self.vm.register_async_typed(name, f)
    }

    /// Register a userdata type in the global environment.
    #[inline]
    pub fn register_type_of<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<()> {
        self.vm.register_type_of::<T>(name)
    }

    /// Register a userdata type and return the exported global type table.
    pub fn register_type<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<Table> {
        self.vm.register_type_of::<T>(name)?;
        self.get_table(name)?.ok_or_else(|| {
            self.vm.error(format!(
                "registered type '{}' did not produce a table",
                name
            ))
        })
    }

    #[inline]
    pub fn register_enum_of<T: LuaEnum>(&mut self, name: &str) -> LuaResult<()> {
        self.vm.register_enum_of::<T>(name)
    }

    /// Return a chunk builder that can be executed, evaluated, or compiled.
    pub fn load<'lua>(&'lua mut self, source: &str) -> Chunk<'lua> {
        Chunk::new(self, source)
    }

    /// Return a chunk builder bound to an isolated sandbox environment.
    #[cfg(feature = "sandbox")]
    pub fn load_sandboxed<'lua>(
        &'lua mut self,
        source: &str,
        config: &SandboxConfig,
    ) -> Chunk<'lua> {
        self.load(source).with_sandbox(config)
    }

    /// Execute source code inside a sandbox and discard returned values.
    #[cfg(feature = "sandbox")]
    pub fn execute_sandboxed(&mut self, source: &str, config: &SandboxConfig) -> LuaResult<()> {
        self.vm.execute_sandboxed(source, config).map(|_| ())
    }

    /// Execute source code inside a sandbox and convert the first returned value.
    #[cfg(feature = "sandbox")]
    pub fn eval_sandboxed<R: FromLua>(
        &mut self,
        source: &str,
        config: &SandboxConfig,
    ) -> LuaResult<R> {
        let value = self
            .vm
            .execute_sandboxed(source, config)?
            .into_iter()
            .next()
            .unwrap_or_else(luars::LuaValue::nil);
        self.unpack_value(value, "eval_sandboxed")
    }

    /// Execute source code inside a sandbox and convert all returned values.
    #[cfg(feature = "sandbox")]
    pub fn eval_multi_sandboxed<R: FromLuaMulti>(
        &mut self,
        source: &str,
        config: &SandboxConfig,
    ) -> LuaResult<R> {
        let values = self.vm.execute_sandboxed(source, config)?;
        self.unpack_multi_values(values, "eval_multi_sandboxed")
    }

    /// Copy an existing global into a sandbox config as an injected global.
    #[cfg(feature = "sandbox")]
    pub fn sandbox_capture_global(
        &mut self,
        config: &mut SandboxConfig,
        name: &str,
    ) -> LuaResult<()> {
        let value = self
            .vm
            .get_global(name)?
            .ok_or_else(|| self.vm.error(format!("global '{}' not found", name)))?;
        config.insert_global(name, value);
        Ok(())
    }

    /// Insert a Rust value into a sandbox config as an injected global.
    #[cfg(feature = "sandbox")]
    pub fn sandbox_insert_global<T: IntoLua>(
        &mut self,
        config: &mut SandboxConfig,
        name: &str,
        value: T,
    ) -> LuaResult<()> {
        let value = into_single_value(&mut self.vm, value, "sandbox_insert_global")?;
        config.insert_global(name, value);
        Ok(())
    }

    /// Run a lexical scope that can create non-`'static` Lua callbacks and borrowed userdata.
    pub fn scope<'lua, R>(
        &'lua mut self,
        f: impl for<'scope> FnOnce(&mut Scope<'scope, 'lua>) -> LuaResult<R>,
    ) -> LuaResult<R> {
        let mut scope = Scope::new(self);
        f(&mut scope)
    }

    /// Compile source and return a callable function handle.
    pub fn load_function(&mut self, source: &str) -> LuaResult<Function> {
        self.load(source).into_function()
    }

    /// Create a safe Lua string handle.
    pub fn create_string(&mut self, value: &str) -> LuaResult<LuaString> {
        let value = self.vm.create_string(value)?;
        self.value_to_string(value)
    }

    pub(crate) fn create_raw_function<F>(&mut self, f: F) -> LuaResult<Function>
    where
        F: Fn(&mut luars::LuaState) -> LuaResult<usize> + 'static,
    {
        let value = self.vm.create_closure(f)?;
        self.value_to_function(value)
    }

    pub(crate) fn create_userdata_value<T: UserDataTrait + 'static>(
        &mut self,
        data: T,
    ) -> LuaResult<Value> {
        let value = self.vm.create_userdata(LuaUserdata::new(data))?;
        Ok(Value::new(self.vm.to_ref(value)))
    }

    /// Create a new empty table handle.
    pub fn create_table(&mut self) -> LuaResult<Table> {
        self.create_table_with_capacity(0, 0)
    }

    /// Create a new table handle with explicit capacities.
    pub fn create_table_with_capacity(&mut self, narr: usize, nrec: usize) -> LuaResult<Table> {
        self.vm.create_table_ref(narr, nrec).map(Table::new)
    }

    /// Create a GC-managed userdata and return a typed handle to it.
    pub fn create_userdata<T: UserDataTrait + 'static>(
        &mut self,
        data: T,
    ) -> LuaResult<UserDataRef<T>> {
        let value = self.vm.create_userdata(LuaUserdata::new(data))?;
        self.value_to_userdata(value)
    }

    /// Create a GC-managed userdata that borrows a Rust value.
    ///
    /// # Safety
    /// The referenced Rust value must outlive all Lua accesses to the returned userdata.
    pub unsafe fn create_userdata_ref<T: UserDataTrait + 'static>(
        &mut self,
        reference: &mut T,
    ) -> LuaResult<UserDataRef<T>> {
        let value = unsafe { self.vm.main_state().create_userdata_ref(reference) }?;
        self.value_to_userdata(value)
    }

    /// Create a table and populate it from key-value pairs.
    pub fn create_table_from<K, V, I>(&mut self, iter: I) -> LuaResult<Table>
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

    /// Create a table from a sequence of values using `1..` as keys.
    pub fn create_sequence_from<T, I>(&mut self, iter: I) -> LuaResult<Table>
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

    /// Get a global function handle.
    pub fn get_function(&mut self, name: &str) -> LuaResult<Option<Function>> {
        self.vm
            .get_global_function(name)
            .map(|opt| opt.map(Function::new))
    }

    /// Get a global table handle.
    pub fn get_table(&mut self, name: &str) -> LuaResult<Option<Table>> {
        self.vm
            .get_global_table(name)
            .map(|opt| opt.map(Table::new))
    }

    /// Bind a safe table handle into the global environment.
    pub fn set_global_table(&mut self, name: &str, table: &Table) -> LuaResult<()> {
        self.vm.set_global(name, table.value())
    }

    /// Bind a safe function handle into the global environment.
    pub fn set_global_function(&mut self, name: &str, function: &Function) -> LuaResult<()> {
        self.vm.set_global(name, function.inner.to_value())
    }

    /// Set a string-keyed table field from a Rust value.
    pub fn table_set<T: IntoLua>(&mut self, table: &Table, key: &str, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.vm, value, "table_set")?;
        table.inner.set(key, value)
    }

    /// Set an integer-keyed table field from a Rust value.
    pub fn table_seti<T: IntoLua>(&mut self, table: &Table, key: i64, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.vm, value, "table_seti")?;
        table.inner.seti(key, value)
    }

    /// Get and convert a string-keyed table field.
    pub fn table_get<T: FromLua>(&mut self, table: &Table, key: &str) -> LuaResult<T> {
        let value = table.inner.get(key)?;
        from_value(&mut self.vm, value, "table_get")
    }

    /// Get and convert an integer-keyed table field.
    pub fn table_geti<T: FromLua>(&mut self, table: &Table, key: i64) -> LuaResult<T> {
        let value = table.inner.geti(key)?;
        from_value(&mut self.vm, value, "table_geti")
    }

    /// Append a Rust value to the array part of a table.
    pub fn table_push<T: IntoLua>(&mut self, table: &Table, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.vm, value, "table_push")?;
        table.inner.push(value)
    }

    /// Convert a table snapshot into typed key-value pairs.
    pub fn table_pairs<K: FromLua, V: FromLua>(&mut self, table: &Table) -> LuaResult<Vec<(K, V)>> {
        table.pairs()
    }

    /// Read the array portion of a table in order from `1..=#t`.
    pub fn table_array<T: FromLua>(&mut self, table: &Table) -> LuaResult<Vec<T>> {
        table.sequence_values()
    }

    /// Pack a Rust value into a safe registry-backed Lua value handle.
    #[inline]
    pub fn pack<T: IntoLua>(&mut self, value: T) -> LuaResult<Value> {
        let value = into_single_value(&mut self.vm, value, "pack")?;
        Ok(Value::new(self.vm.to_ref(value)))
    }

    /// Unpack a safe Lua value handle into a Rust value.
    #[inline]
    pub fn unpack<T: FromLua>(&mut self, value: Value) -> LuaResult<T> {
        self.unpack_value(value.to_value(), "unpack")
    }

    /// Convert one Rust/Lua-convertible value into another.
    #[inline]
    pub fn convert<T: IntoLua, U: FromLua>(&mut self, value: T) -> LuaResult<U> {
        let value = into_single_value(&mut self.vm, value, "convert")?;
        self.unpack_value(value, "convert")
    }

    /// Get a mutable reference to the underlying LuaVM for advanced use cases.
    pub unsafe fn vm_mut(&mut self) -> &mut LuaVM {
        &mut self.vm
    }
}

impl Default for Lua {
    fn default() -> Self {
        Self::new(SafeOption::default())
    }
}
