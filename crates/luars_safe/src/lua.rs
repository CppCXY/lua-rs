use crate::util::{from_value, into_single_value};
use crate::{Function, Table, TableBuilder};
use luars::lua_vm::{LuaTypedAsyncCallback, LuaTypedCallback};
use luars::{
    FromLua, FromLuaMulti, IntoLua, LuaEnum, LuaRegistrable, LuaResult, LuaVM, Stdlib
};
use luars::lua_vm::SafeOption;

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

    /// Execute source code and discard raw return values.
    #[inline]
    pub fn execute(&mut self, source: &str) -> LuaResult<()> {
        self.vm.execute(source).map(|_| ())
    }

    /// Execute source code and convert the first returned value.
    pub fn eval<R: FromLua>(&mut self, source: &str) -> LuaResult<R> {
        let values = self.vm.execute(source)?;
        let value = values.into_iter().next().unwrap_or_else(luars::LuaValue::nil);
        from_value(&mut self.vm, value, "eval")
    }

    /// Execute source code and convert all returned values.
    pub fn eval_multi<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R> {
        let values = self.vm.execute(source)?;
        R::from_lua_multi(values, self.vm.main_state()).map_err(|msg| self.vm.error(msg))
    }

    /// Set a global from a Rust value that converts to a single Lua value.
    pub fn set_global<T: IntoLua>(&mut self, name: &str, value: T) -> LuaResult<()> {
        let value = into_single_value(&mut self.vm, value, "set_global")?;
        self.vm.set_global(name, value)
    }

    /// Get and convert a global variable.
    #[inline]
    pub fn get_global<T: FromLua>(&mut self, name: &str) -> LuaResult<Option<T>> {
        self.vm.get_global_as(name)
    }

    /// Call a global function and convert all results.
    #[inline]
    pub fn call_global<A: IntoLua, R: FromLuaMulti>(&mut self, name: &str, args: A) -> LuaResult<R> {
        self.vm.call_global(name, args)
    }

    /// Call a global function and convert the first result.
    #[inline]
    pub fn call_global1<A: IntoLua, R: FromLua>(&mut self, name: &str, args: A) -> LuaResult<R> {
        self.vm.call1_global(name, args)
    }

    /// Register a typed Rust callback as a Lua global.
    #[inline]
    pub fn register_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedCallback<Args, R>,
    {
        self.vm.register_function_typed(name, f)
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

    #[inline]
    pub fn register_enum_of<T: LuaEnum>(&mut self, name: &str) -> LuaResult<()> {
        self.vm.register_enum_of::<T>(name)
    }

    /// Compile source and return a callable function handle.
    pub fn load(&mut self, source: &str) -> LuaResult<Function> {
        let value = self.vm.load(source)?;
        let function = self
            .vm
            .to_function_ref(value)
            .ok_or_else(|| self.vm.error("compiled chunk is not a function"))?;
        Ok(Function::new(function))
    }

    /// Create a new table handle.
    pub fn create_table(&mut self, narr: usize, nrec: usize) -> LuaResult<Table> {
        self.vm.create_table_ref(narr, nrec).map(Table::new)
    }

    /// Build a safe table using `TableBuilder`.
    pub fn build_table(&mut self, builder: TableBuilder) -> LuaResult<Table> {
        builder.build(self)
    }

    /// Get a global function handle.
    pub fn get_function(&mut self, name: &str) -> LuaResult<Option<Function>> {
        self.vm
            .get_global_function(name)
            .map(|opt| opt.map(Function::new))
    }

    /// Get a global table handle.
    pub fn get_table(&mut self, name: &str) -> LuaResult<Option<Table>> {
        self.vm.get_global_table(name).map(|opt| opt.map(Table::new))
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
    pub fn table_pairs<K: FromLua, V: FromLua>(
        &mut self,
        table: &Table,
    ) -> LuaResult<Vec<(K, V)>> {
        let pairs = table.pairs_raw()?;
        let mut converted = Vec::with_capacity(pairs.len());
        for (key, value) in pairs {
            let key = from_value(&mut self.vm, key, "table_pairs(key)")?;
            let value = from_value(&mut self.vm, value, "table_pairs(value)")?;
            converted.push((key, value));
        }
        Ok(converted)
    }

    /// Read the array portion of a table in order from `1..=#t`.
    pub fn table_array<T: FromLua>(&mut self, table: &Table) -> LuaResult<Vec<T>> {
        let len = table.len()?;
        let mut values = Vec::with_capacity(len);
        for index in 1..=len {
            values.push(self.table_geti(table, index as i64)?);
        }
        Ok(values)
    }

    pub(crate) fn vm_mut(&mut self) -> &mut LuaVM {
        &mut self.vm
    }
}

impl Default for Lua {
    fn default() -> Self {
        Self::new(SafeOption::default())
    }
}

