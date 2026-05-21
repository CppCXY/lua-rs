use std::ffi::c_void;

use crate::lua_api::{Chunk, Function, LuaApi, LuaString, Table, Value};
use crate::lua_vm::{LuaTypedAsyncCallback, LuaTypedCallback};
use crate::{
    FromLua, FromLuaMulti, IntoLua, LuaEnum, LuaError, LuaFullError, LuaRegistrable, LuaResult,
    LuaState, LuaUserdata, LuaValue, LuaValueKind, Stdlib, UserDataRef, UserDataTrait,
};

fn collect_values<T: IntoLua>(state: &mut LuaState, value: T) -> LuaResult<Vec<LuaValue>> {
    let base_top = state.get_top();

    let pushed = match value.into_lua(state) {
        Ok(pushed) => pushed,
        Err(err) => {
            state.set_top_raw(base_top);
            return Err(state.error(err));
        }
    };

    let mut values = Vec::with_capacity(pushed);
    for index in base_top..base_top + pushed {
        let Some(value) = state.stack_get(index) else {
            state.set_top_raw(base_top);
            return Err(
                state.error("internal error: failed to collect Lua values from stack".to_owned())
            );
        };
        values.push(value);
    }
    state.set_top_raw(base_top);

    Ok(values)
}

fn into_single_value<T: IntoLua>(
    state: &mut LuaState,
    value: T,
    api_name: &str,
) -> LuaResult<LuaValue> {
    let mut values = collect_values(state, value)?;
    if values.len() != 1 {
        return Err(state.error(format!(
            "{} expects exactly one Lua value, got {}",
            api_name,
            values.len()
        )));
    }
    Ok(values.pop().unwrap())
}

fn from_value<T: FromLua>(state: &mut LuaState, value: LuaValue, api_name: &str) -> LuaResult<T> {
    T::from_lua(value, state).map_err(|msg| state.error(format!("{}: {}", api_name, msg)))
}

impl LuaApi for LuaState {
    fn open_stdlib(&mut self, lib: Stdlib) -> LuaResult<()> {
        self.global_state_mut().open_stdlib(lib)
    }

    fn load_stdlibs(&mut self, lib: Stdlib) -> LuaResult<()> {
        self.open_stdlib(lib)
    }

    fn collect_garbage(&mut self) -> LuaResult<()> {
        LuaState::collect_garbage(self)
    }

    fn execute(&mut self, source: &str) -> LuaResult<()> {
        LuaState::execute(self, source).map(|_| ())
    }

    fn dofile<R: FromLuaMulti>(&mut self, path: &str) -> LuaResult<R> {
        let values = LuaState::dofile(self, path)?;
        R::from_lua_multi(values, self).map_err(|msg| self.error(format!("dofile: {}", msg)))
    }

    fn eval<R: FromLua>(&mut self, source: &str) -> LuaResult<R> {
        let value = LuaState::execute(self, source)?
            .into_iter()
            .next()
            .unwrap_or_else(LuaValue::nil);
        from_value(self, value, "eval")
    }

    fn eval_multi<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R> {
        let values = LuaState::execute(self, source)?;
        R::from_lua_multi(values, self).map_err(|msg| self.error(msg))
    }

    fn set_global<T: IntoLua>(&mut self, name: &str, value: T) -> LuaResult<()> {
        let value = into_single_value(self, value, "set_global")?;
        LuaState::set_global(self, name, value)
    }

    fn globals(&mut self) -> Table {
        let global = self.global_state().global;
        Table::new(
            self.to_table_ref(global)
                .expect("global environment must be a table"),
        )
    }

    fn get_global<T: FromLua>(&mut self, name: &str) -> LuaResult<Option<T>> {
        self.get_global_as(name)
    }

    fn call_global<A: IntoLua, R: FromLuaMulti>(&mut self, name: &str, args: A) -> LuaResult<R> {
        let args = collect_values(self, args)?;
        let values = LuaState::call_global(self, name, args)?;
        R::from_lua_multi(values, self).map_err(|msg| self.error(format!("call_global: {}", msg)))
    }

    fn call_global1<A: IntoLua, R: FromLua>(&mut self, name: &str, args: A) -> LuaResult<R> {
        let args = collect_values(self, args)?;
        let value = LuaState::call_global(self, name, args)?
            .into_iter()
            .next()
            .unwrap_or_else(LuaValue::nil);
        from_value(self, value, "call_global1")
    }

    fn register_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedCallback<Args, R>,
    {
        LuaState::register_function_typed(self, name, f)
    }

    fn create_function<F, Args, R>(&mut self, f: F) -> LuaResult<Function>
    where
        F: LuaTypedCallback<Args, R>,
    {
        let closure = self.create_closure(move |state| f.invoke_typed(state))?;
        let function = self
            .to_function_ref(closure)
            .expect("created closure must be a function");
        Ok(Function::new(function))
    }

    fn register_async_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedAsyncCallback<Args, R>,
    {
        LuaState::register_async_typed(self, name, f)
    }

    fn register_type_of<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<()> {
        self.global_state_mut().register_type_of::<T>(name)
    }

    fn register_type<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<Table> {
        self.register_type_of::<T>(name)?;
        self.get_table(name)?.ok_or_else(|| {
            self.error(format!(
                "registered type '{}' did not produce a table",
                name
            ))
        })
    }

    fn register_enum_of<T: LuaEnum>(&mut self, name: &str) -> LuaResult<()> {
        self.global_state_mut().register_enum_of::<T>(name)
    }

    fn load<'lua>(&'lua mut self, source: &str) -> Chunk<'lua, Self>
    where
        Self: Sized,
    {
        Chunk::new(self, source)
    }

    fn load_function(&mut self, source: &str) -> LuaResult<Function> {
        let value = LuaState::load(self, source)?;
        let function = self
            .to_function_ref(value)
            .ok_or_else(|| self.error("compiled chunk is not a function".to_string()))?;
        Ok(Function::new(function))
    }

    fn create_string(&mut self, value: &str) -> LuaResult<LuaString> {
        let value = LuaState::create_string(self, value)?;
        let string = self
            .global_state_mut()
            .to_string_ref(value)
            .ok_or_else(|| self.error("value is not a string".to_string()))?;
        Ok(LuaString::new(string))
    }

    fn create_table(&mut self) -> LuaResult<Table> {
        self.create_table_with_capacity(0, 0)
    }

    fn create_table_with_capacity(&mut self, narr: usize, nrec: usize) -> LuaResult<Table> {
        let table = LuaState::create_table(self, narr, nrec)?;
        Ok(Table::new(
            self.to_table_ref(table)
                .expect("created table must be a table"),
        ))
    }

    fn create_userdata<T: UserDataTrait + 'static>(
        &mut self,
        data: T,
    ) -> LuaResult<UserDataRef<T>> {
        let value = LuaState::create_userdata(self, LuaUserdata::new(data))?;
        self.to_userdata_ref(value)
            .ok_or_else(|| self.error("value is not the expected userdata type".to_string()))
    }

    unsafe fn create_userdata_ref<T: UserDataTrait + 'static>(
        &mut self,
        reference: &mut T,
    ) -> LuaResult<UserDataRef<T>> {
        let value = unsafe { LuaState::create_userdata_ref(self, reference)? };
        self.to_userdata_ref(value)
            .ok_or_else(|| self.error("value is not the expected userdata type".to_string()))
    }

    fn create_table_from<K, V, I>(&mut self, iter: I) -> LuaResult<Table>
    where
        K: IntoLua,
        V: IntoLua,
        I: IntoIterator<Item = (K, V)>,
    {
        let table = <LuaState as LuaApi>::create_table(self)?;
        for (key, value) in iter {
            table.set(key, value)?;
        }
        Ok(table)
    }

    fn create_sequence_from<T, I>(&mut self, iter: I) -> LuaResult<Table>
    where
        T: IntoLua,
        I: IntoIterator<Item = T>,
    {
        let table = <LuaState as LuaApi>::create_table(self)?;
        for value in iter {
            table.push(value)?;
        }
        Ok(table)
    }

    fn get_function(&mut self, name: &str) -> LuaResult<Option<Function>> {
        let value = LuaState::get_global(self, name)?;
        Ok(value
            .and_then(|value| self.to_function_ref(value))
            .map(Function::new))
    }

    fn get_table(&mut self, name: &str) -> LuaResult<Option<Table>> {
        let value = LuaState::get_global(self, name)?;
        Ok(value
            .and_then(|value| self.to_table_ref(value))
            .map(Table::new))
    }

    fn set_global_table(&mut self, name: &str, table: &Table) -> LuaResult<()> {
        <LuaState as LuaApi>::set_global(self, name, table.clone())
    }

    fn set_global_function(&mut self, name: &str, function: &Function) -> LuaResult<()> {
        <LuaState as LuaApi>::set_global(self, name, function.clone())
    }

    fn table_set<T: IntoLua>(&mut self, table: &Table, key: &str, value: T) -> LuaResult<()> {
        let value = into_single_value(self, value, "table_set")?;
        table.inner.set(key, value)
    }

    fn table_seti<T: IntoLua>(&mut self, table: &Table, key: i64, value: T) -> LuaResult<()> {
        let value = into_single_value(self, value, "table_seti")?;
        table.inner.seti(key, value)
    }

    fn table_get<T: FromLua>(&mut self, table: &Table, key: &str) -> LuaResult<T> {
        let value = table.inner.get(key)?;
        from_value(self, value, "table_get")
    }

    fn table_geti<T: FromLua>(&mut self, table: &Table, key: i64) -> LuaResult<T> {
        let value = table.inner.geti(key)?;
        from_value(self, value, "table_geti")
    }

    fn table_push<T: IntoLua>(&mut self, table: &Table, value: T) -> LuaResult<()> {
        let value = into_single_value(self, value, "table_push")?;
        table.inner.push(value)
    }

    fn table_pairs<K: FromLua, V: FromLua>(&mut self, table: &Table) -> LuaResult<Vec<(K, V)>> {
        table.pairs()
    }

    fn table_array<T: FromLua>(&mut self, table: &Table) -> LuaResult<Vec<T>> {
        table.sequence_values()
    }

    fn get_metatable<T: IntoLua>(&mut self, value: T) -> LuaResult<Option<Table>> {
        Ok(self.pack(value)?.get_metatable())
    }

    fn set_metatable<T: IntoLua>(&mut self, value: T, metatable: Option<&Table>) -> LuaResult<()> {
        let value = self.pack(value)?;
        if let Some(table) = value.as_table() {
            return table.set_metatable(metatable);
        }
        value.set_metatable(metatable)
    }

    fn pack<T: IntoLua>(&mut self, value: T) -> LuaResult<Value> {
        let value = into_single_value(self, value, "pack")?;
        Ok(Value::new(self.to_any_ref(value)))
    }

    fn unpack<T: FromLua>(&mut self, value: Value) -> LuaResult<T> {
        from_value(self, value.to_value(), "unpack")
    }

    fn convert<T: IntoLua, U: FromLua>(&mut self, value: T) -> LuaResult<U> {
        let value = into_single_value(self, value, "convert")?;
        from_value(self, value, "convert")
    }

    fn set_extra_space(&mut self, pointer: *mut c_void) {
        self.global_state_mut().set_extra_space(pointer);
    }

    fn extra_space(&self) -> *mut c_void {
        self.global_state().extra_space()
    }

    fn create_lightuserdata(&mut self, pointer: *mut c_void) -> Value {
        Value::new(self.to_any_ref(LuaValue::lightuserdata(pointer)))
    }

    fn to_pointer<T: IntoLua>(&mut self, value: T) -> LuaResult<Option<*const c_void>> {
        Ok(self.pack(value)?.to_pointer())
    }

    fn registry(&mut self) -> Table {
        let registry = self.global_state().registry;
        Table::new(
            self.to_table_ref(registry)
                .expect("registry must be a table"),
        )
    }

    fn registry_get<T: FromLua>(&mut self, key: &str) -> LuaResult<Option<T>> {
        let Some(value) = self.global_state_mut().registry_get(key)? else {
            return Ok(None);
        };
        from_value(self, value, "registry_get").map(Some)
    }

    fn registry_set<T: IntoLua>(&mut self, key: &str, value: T) -> LuaResult<()> {
        let value = into_single_value(self, value, "registry_set")?;
        self.global_state_mut().registry_set(key, value)
    }

    fn registry_geti<T: FromLua>(&mut self, key: i64) -> LuaResult<Option<T>> {
        let Some(value) = self.global_state().registry_geti(key) else {
            return Ok(None);
        };
        from_value(self, value, "registry_geti").map(Some)
    }

    fn registry_seti<T: IntoLua>(&mut self, key: i64, value: T) -> LuaResult<()> {
        let value = into_single_value(self, value, "registry_seti")?;
        self.global_state_mut().registry_seti(key, value);
        Ok(())
    }

    fn get_type_metatable(&mut self, kind: LuaValueKind) -> Option<Table> {
        let metatable = self.global_state().get_basic_metatable(kind)?;
        self.to_table_ref(metatable).map(Table::new)
    }

    fn set_type_metatable(
        &mut self,
        kind: LuaValueKind,
        metatable: Option<&Table>,
    ) -> LuaResult<()> {
        self.global_state_mut()
            .set_basic_metatable(kind, metatable.map(Table::value));
        Ok(())
    }

    fn get_error_message(&mut self, error: LuaError) -> LuaFullError {
        self.get_full_error(error)
    }

    fn gc_stop(&mut self) {
        self.global_state_mut().gc.gc_stopped = true;
    }

    fn gc_restart(&mut self) {
        self.global_state_mut().gc.gc_stopped = false;
        self.global_state_mut().gc.set_debt(0);
    }
}
