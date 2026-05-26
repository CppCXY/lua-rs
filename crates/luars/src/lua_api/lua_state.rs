use std::ffi::c_void;

use crate::lua_api::{
    Chunk, LUA_GLOBALSINDEX, LUA_MULTRET, LUA_REGISTRYINDEX, LuaApi, LuaFunction, LuaString,
    LuaTable, Value, lua_upvalueindex,
};
use crate::lua_vm::{LuaTypedAsyncCallback, LuaTypedCallback};
use crate::stdlib::basic::parse_number::parse_lua_number;
use crate::{
    FromLua, FromLuaMulti, IntoLua, LuaEnum, LuaError, LuaFullError, LuaRegistrable, LuaResult,
    LuaStackApi, LuaState, LuaUserdata, LuaValue, LuaValueKind, StackValueApi, Stdlib, UserDataRef,
    UserDataTrait,
};

fn stack_api_base(state: &LuaState) -> usize {
    if state.call_depth() == 0 {
        0
    } else {
        state.call_stack[state.call_depth() - 1].base
    }
}

fn stack_api_top(state: &LuaState) -> usize {
    state.get_top()
}

fn stack_api_len(state: &LuaState) -> usize {
    stack_api_top(state).saturating_sub(stack_api_base(state))
}

fn stack_api_current_frame_function(state: &LuaState) -> Option<LuaValue> {
    let frame = state.current_frame()?;
    let func_pos = frame.base.checked_sub(frame.func_offset as usize)?;
    state.stack_get(func_pos)
}

fn stack_api_current_upvalue_value(state: &LuaState, n: usize) -> Option<LuaValue> {
    if n == 0 {
        return None;
    }

    let frame = state.current_frame()?;
    let func = stack_api_current_frame_function(state)?;

    if frame.is_c() {
        if let Some(cclosure) = func.as_cclosure() {
            return cclosure.upvalues().get(n - 1).copied();
        }

        if let Some(rclosure) = func.as_rclosure() {
            return rclosure.upvalues().get(n - 1).copied();
        }

        return None;
    }

    let lua_func = func.as_lua_function()?;
    let upvalue_ptr = lua_func.upvalues().get(n - 1).copied()?;
    Some(upvalue_ptr.as_ref().data.get_value())
}

fn stack_api_pseudo_value(state: &LuaState, idx: isize) -> Option<LuaValue> {
    match idx {
        LUA_REGISTRYINDEX => Some(state.global_state().registry),
        LUA_GLOBALSINDEX => Some(state.global_state().global),
        _ if idx <= lua_upvalueindex(1) => {
            let n = (LUA_GLOBALSINDEX - idx) as usize;
            stack_api_current_upvalue_value(state, n)
        }
        _ => None,
    }
}

fn stack_api_resolve_index(state: &LuaState, idx: isize) -> Option<usize> {
    if idx == 0 {
        return None;
    }

    let base = stack_api_base(state);
    let top = stack_api_top(state);

    if idx > 0 {
        let abs = base + (idx as usize).saturating_sub(1);
        (abs < top).then_some(abs)
    } else {
        let offset = (-idx) as usize;
        if offset == 0 || offset > top.saturating_sub(base) {
            None
        } else {
            Some(top - offset)
        }
    }
}

fn stack_api_expect_value(state: &mut LuaState, idx: isize, api_name: &str) -> LuaResult<LuaValue> {
    stack_api_value(state, idx)
        .ok_or_else(|| state.error(format!("{}: invalid stack index {}", api_name, idx)))
}

fn stack_api_assign_pseudo(
    state: &mut LuaState,
    idx: isize,
    value: LuaValue,
    api_name: &str,
) -> LuaResult<bool> {
    if matches!(idx, LUA_REGISTRYINDEX | LUA_GLOBALSINDEX) {
        return Err(state.error(format!(
            "{}: cannot assign to pseudo index {}",
            api_name, idx
        )));
    }

    if idx <= lua_upvalueindex(1) {
        let n = (LUA_GLOBALSINDEX - idx) as usize;
        if n == 0 {
            return Ok(false);
        }

        let is_c_frame = state
            .current_frame()
            .map(|frame| frame.is_c())
            .ok_or_else(|| state.error(format!("{}: no current call frame", api_name)))?;
        let func = stack_api_current_frame_function(state)
            .ok_or_else(|| state.error(format!("{}: missing current function", api_name)))?;

        if is_c_frame {
            if let Some(cclosure) = func.as_cclosure_mut() {
                let Some(slot) = cclosure.upvalues_mut().get_mut(n - 1) else {
                    return Ok(false);
                };
                *slot = value;
                if value.is_collectable()
                    && let Some(owner) = func.as_cclosure_ptr()
                {
                    state.gc_barrier_back(owner.into());
                }
                return Ok(true);
            }

            if let Some(rclosure) = func.as_rclosure_mut() {
                let Some(slot) = rclosure.upvalues_mut().get_mut(n - 1) else {
                    return Ok(false);
                };
                *slot = value;
                if value.is_collectable()
                    && let Some(owner) = func.as_rclosure_ptr()
                {
                    state.gc_barrier_back(owner.into());
                }
                return Ok(true);
            }

            return Ok(false);
        }

        let Some(lua_func) = func.as_lua_function() else {
            return Ok(false);
        };
        let Some(upvalue_ptr) = lua_func.upvalues().get(n - 1).copied() else {
            return Ok(false);
        };
        upvalue_ptr.as_mut_ref().data.set_value(value);
        if value.is_collectable()
            && let Some(value_gc_ptr) = value.as_gc_ptr()
        {
            state.gc_barrier(upvalue_ptr, value_gc_ptr);
        }
        return Ok(true);
    }

    Ok(false)
}

fn stack_api_assign_value(
    state: &mut LuaState,
    idx: isize,
    value: LuaValue,
    api_name: &str,
) -> LuaResult<()> {
    if stack_api_assign_pseudo(state, idx, value, api_name)? {
        return Ok(());
    }

    let abs = stack_api_resolve_index(state, idx)
        .ok_or_else(|| state.error(format!("{}: invalid stack index {}", api_name, idx)))?;
    state.stack_set(abs, value)
}

fn stack_api_value(state: &LuaState, idx: isize) -> Option<LuaValue> {
    stack_api_pseudo_value(state, idx)
        .or_else(|| stack_api_resolve_index(state, idx).and_then(|abs| state.stack_get(abs)))
}

fn stack_api_function(state: &LuaState, idx: isize) -> Option<LuaValue> {
    let value = stack_api_value(state, idx)?;
    value.is_function().then_some(value)
}

fn stack_api_str(state: &LuaState, idx: isize) -> Option<&str> {
    let value = stack_api_value(state, idx)?;
    value.as_str().map(|s| unsafe { &*(s as *const str) })
}

fn stack_api_bytes(state: &LuaState, idx: isize) -> Option<&[u8]> {
    let value = stack_api_value(state, idx)?;
    value
        .as_bytes()
        .map(|bytes| unsafe { &*(bytes as *const [u8]) })
}

fn stack_api_numeric_value(state: &LuaState, idx: isize) -> Option<LuaValue> {
    let value = stack_api_value(state, idx)?;
    if value.as_number().is_some() || value.as_integer().is_some() {
        return Some(value);
    }

    value.as_str().and_then(|text| {
        let parsed = parse_lua_number(text);
        (parsed.as_number().is_some() || parsed.as_integer().is_some()).then_some(parsed)
    })
}

fn stack_api_collect_upvalues(
    state: &mut LuaState,
    n: usize,
    api_name: &str,
) -> LuaResult<Vec<LuaValue>> {
    let len = stack_api_len(state);
    if n > len {
        return Err(state.error(format!(
            "{}: not enough values on stack for {} upvalues",
            api_name, n
        )));
    }

    let top = stack_api_top(state);
    let start = top - n;
    let mut upvalues = Vec::with_capacity(n);
    for index in start..top {
        let Some(value) = state.stack_get(index) else {
            return Err(state.error(format!(
                "{}: internal error reading upvalues from stack",
                api_name
            )));
        };
        upvalues.push(value);
    }
    state.set_top_raw(start);
    Ok(upvalues)
}

fn stack_api_adjust_call_results(
    state: &mut LuaState,
    func_idx: usize,
    result_count: usize,
    nresults: isize,
    api_name: &str,
) -> LuaResult<()> {
    if nresults < LUA_MULTRET {
        return Err(state.error(format!("{}: invalid result count {}", api_name, nresults)));
    }

    if nresults == LUA_MULTRET {
        return Ok(());
    }

    let wanted = nresults as usize;
    let new_top = func_idx + wanted;
    state.set_top(new_top)?;
    for index in result_count..wanted {
        state.stack_set(func_idx + index, LuaValue::nil())?;
    }
    Ok(())
}

fn stack_api_call_fallback(
    state: &mut LuaState,
    nargs: usize,
    nresults: isize,
    api_name: &str,
) -> LuaResult<()> {
    let top = state.get_top();
    let needed = nargs + 1;
    if top < needed {
        return Err(state.error(format!(
            "{}: expected function plus {} arguments on stack",
            api_name, nargs
        )));
    }

    let func_idx = top - needed;
    let func = state
        .stack_get(func_idx)
        .ok_or_else(|| state.error(format!("{}: missing function on stack", api_name)))?;
    let mut args = Vec::with_capacity(nargs);
    for index in func_idx + 1..top {
        let Some(value) = state.stack_get(index) else {
            return Err(state.error(format!("{}: missing argument on stack", api_name)));
        };
        args.push(value);
    }

    let results = state.call(func, args)?;
    state.set_top(func_idx)?;
    for value in results {
        state.push_value(value)?;
    }
    let result_count = state.get_top().saturating_sub(func_idx);
    stack_api_adjust_call_results(state, func_idx, result_count, nresults, api_name)
}

fn stack_api_pcall_fallback(
    state: &mut LuaState,
    nargs: usize,
    nresults: isize,
) -> LuaResult<bool> {
    let top = state.get_top();
    let needed = nargs + 1;
    if top < needed {
        return Err(state.error(format!(
            "lua_pcall: expected function plus {} arguments on stack",
            nargs
        )));
    }

    let func_idx = top - needed;
    let func = state
        .stack_get(func_idx)
        .ok_or_else(|| state.error("lua_pcall: missing function on stack".to_string()))?;
    let mut args = Vec::with_capacity(nargs);
    for index in func_idx + 1..top {
        let Some(value) = state.stack_get(index) else {
            return Err(state.error("lua_pcall: missing argument on stack".to_string()));
        };
        args.push(value);
    }

    let (success, results) = state.pcall(func, args)?;
    state.set_top(func_idx)?;
    for value in results {
        state.push_value(value)?;
    }

    if success {
        let result_count = state.get_top().saturating_sub(func_idx);
        stack_api_adjust_call_results(state, func_idx, result_count, nresults, "lua_pcall")?;
    }

    Ok(success)
}

fn stack_api_upvaluecount(value: LuaValue) -> usize {
    if let Some(lua_func) = value.as_lua_function() {
        lua_func.upvalues().len()
    } else if let Some(cclosure) = value.as_cclosure() {
        cclosure.upvalues().len()
    } else if let Some(rclosure) = value.as_rclosure() {
        rclosure.upvalues().len()
    } else {
        0
    }
}

impl LuaApi for LuaState {
    fn open_stdlib(&mut self, lib: Stdlib) -> LuaResult<()> {
        self.global_state_mut().open_stdlib(lib)
    }

    fn open_stdlibs(&mut self, libs: &[Stdlib]) -> LuaResult<()> {
        for lib in libs {
            self.open_stdlib(*lib)?;
        }
        Ok(())
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
        self.from_value(value, "eval")
    }

    fn eval_multi<R: FromLuaMulti>(&mut self, source: &str) -> LuaResult<R> {
        let values = LuaState::execute(self, source)?;
        R::from_lua_multi(values, self).map_err(|msg| self.error(msg))
    }

    fn set_global<T: IntoLua>(&mut self, name: &str, value: T) -> LuaResult<()> {
        let value = self.collect_single_value(value, "set_global")?;
        LuaState::set_global_value(self, name, value)
    }

    fn globals(&mut self) -> LuaTable {
        let global = self.global_state().global;
        LuaTable::new(
            self.to_table_ref(global)
                .expect("global environment must be a table"),
        )
    }

    fn get_global<T: FromLua>(&mut self, name: &str) -> LuaResult<Option<T>> {
        self.get_global_as(name)
    }

    fn call_global<A: IntoLua, R: FromLuaMulti>(&mut self, name: &str, args: A) -> LuaResult<R> {
        let args = self.collect_values(args, "call_global")?;
        let values = LuaState::call_global(self, name, args)?;
        R::from_lua_multi(values, self).map_err(|msg| self.error(format!("call_global: {}", msg)))
    }

    fn call_global1<A: IntoLua, R: FromLua>(&mut self, name: &str, args: A) -> LuaResult<R> {
        let args = self.collect_values(args, "call_global1")?;
        let value = LuaState::call_global(self, name, args)?
            .into_iter()
            .next()
            .unwrap_or_else(LuaValue::nil);
        self.from_value(value, "call_global1")
    }

    fn register_function<F, Args, R>(&mut self, name: &str, f: F) -> LuaResult<()>
    where
        F: LuaTypedCallback<Args, R>,
    {
        LuaState::register_function_typed(self, name, f)
    }

    fn create_function<F, Args, R>(&mut self, f: F) -> LuaResult<LuaFunction>
    where
        F: LuaTypedCallback<Args, R>,
    {
        let closure = self.create_closure(move |state| f.invoke_typed(state))?;
        let function = self
            .to_function_ref(closure)
            .expect("created closure must be a function");
        Ok(LuaFunction::new(function))
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

    fn create_type_register_table<T: LuaRegistrable>(&mut self, name: &str) -> LuaResult<LuaTable> {
        self.register_type_of::<T>(name)?;
        <Self as LuaApi>::get_global::<LuaTable>(self, name)?.ok_or_else(|| {
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

    fn load_function(&mut self, source: &str) -> LuaResult<LuaFunction> {
        let value = LuaState::load(self, source)?;
        let function = self
            .to_function_ref(value)
            .ok_or_else(|| self.error("compiled chunk is not a function".to_string()))?;
        Ok(LuaFunction::new(function))
    }

    fn create_string(&mut self, value: &str) -> LuaResult<LuaString> {
        let value = LuaState::create_string(self, value)?;
        let string = self
            .global_state_mut()
            .to_string_ref(value)
            .ok_or_else(|| self.error("value is not a string".to_string()))?;
        Ok(LuaString::new(string))
    }

    fn create_table(&mut self) -> LuaResult<LuaTable> {
        self.create_table_with_capacity(0, 0)
    }

    fn create_table_with_capacity(&mut self, narr: usize, nrec: usize) -> LuaResult<LuaTable> {
        let table = LuaState::create_table(self, narr, nrec)?;
        Ok(LuaTable::new(
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

    fn create_table_from<K, V, I>(&mut self, iter: I) -> LuaResult<LuaTable>
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

    fn create_sequence_from<T, I>(&mut self, iter: I) -> LuaResult<LuaTable>
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

    fn pack<T: IntoLua>(&mut self, value: T) -> LuaResult<Value> {
        let value = self.collect_single_value(value, "pack")?;
        Ok(Value::new(self.to_any_ref(value)))
    }

    fn unpack<T: FromLua>(&mut self, value: Value) -> LuaResult<T> {
        self.from_value(value.to_value(), "unpack")
    }

    fn convert<T: IntoLua, U: FromLua>(&mut self, value: T) -> LuaResult<U> {
        let value = self.collect_single_value(value, "convert")?;
        self.from_value(value, "convert")
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

    fn registry(&mut self) -> LuaTable {
        let registry = self.global_state().registry;
        LuaTable::new(
            self.to_table_ref(registry)
                .expect("registry must be a table"),
        )
    }

    fn registry_get<T: FromLua>(&mut self, key: &str) -> LuaResult<Option<T>> {
        let Some(value) = self.global_state_mut().registry_get(key)? else {
            return Ok(None);
        };
        self.from_value(value, "registry_get").map(Some)
    }

    fn registry_set<T: IntoLua>(&mut self, key: &str, value: T) -> LuaResult<()> {
        let value = self.collect_single_value(value, "registry_set")?;
        self.global_state_mut().registry_set(key, value)
    }

    fn registry_geti<T: FromLua>(&mut self, key: i64) -> LuaResult<Option<T>> {
        let Some(value) = self.global_state().registry_geti(key) else {
            return Ok(None);
        };
        self.from_value(value, "registry_geti").map(Some)
    }

    fn get_type_metatable(&mut self, kind: LuaValueKind) -> Option<LuaTable> {
        let metatable = self.global_state().get_basic_metatable(kind)?;
        self.to_table_ref(metatable).map(LuaTable::new)
    }

    fn set_type_metatable(
        &mut self,
        kind: LuaValueKind,
        metatable: Option<&LuaTable>,
    ) -> LuaResult<()> {
        self.global_state_mut()
            .set_basic_metatable(kind, metatable.map(LuaTable::value));
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

impl LuaStackApi for LuaState {
    #[inline]
    fn lua_gettop(&self) -> isize {
        stack_api_len(self) as isize
    }

    #[inline]
    fn lua_type(&self, idx: isize) -> Option<LuaValueKind> {
        stack_api_value(self, idx).map(|value| value.kind())
    }

    #[inline]
    fn lua_typename(&self, idx: isize) -> Option<&'static str> {
        stack_api_value(self, idx).map(|value| value.type_name())
    }

    #[inline]
    fn lua_isnone(&self, idx: isize) -> bool {
        stack_api_value(self, idx).is_none()
    }

    #[inline]
    fn lua_isnil(&self, idx: isize) -> bool {
        stack_api_value(self, idx).is_some_and(|value| value.is_nil())
    }

    #[inline]
    fn lua_isnoneornil(&self, idx: isize) -> bool {
        stack_api_value(self, idx).is_none_or(|value| value.is_nil())
    }

    #[inline]
    fn lua_isboolean(&self, idx: isize) -> bool {
        stack_api_value(self, idx).is_some_and(|value| value.as_boolean().is_some())
    }

    #[inline]
    fn lua_isnumber(&self, idx: isize) -> bool {
        stack_api_numeric_value(self, idx).is_some()
    }

    #[inline]
    fn lua_isinteger(&self, idx: isize) -> bool {
        stack_api_value(self, idx).is_some_and(|value| value.kind() == LuaValueKind::Integer)
    }

    #[inline]
    fn lua_isstring(&self, idx: isize) -> bool {
        stack_api_value(self, idx).is_some_and(|value| {
            matches!(
                value.kind(),
                LuaValueKind::String | LuaValueKind::Integer | LuaValueKind::Float
            )
        })
    }

    #[inline]
    fn lua_istable(&self, idx: isize) -> bool {
        stack_api_value(self, idx).is_some_and(|value| value.is_table())
    }

    #[inline]
    fn lua_isfunction(&self, idx: isize) -> bool {
        stack_api_value(self, idx).is_some_and(|value| value.is_function())
    }

    #[inline]
    fn lua_iscfunction(&self, idx: isize) -> bool {
        stack_api_value(self, idx)
            .is_some_and(|value| value.as_cfunction().is_some() || value.as_cclosure().is_some())
    }

    #[inline]
    fn lua_isuserdata(&self, idx: isize) -> bool {
        stack_api_value(self, idx)
            .is_some_and(|value| value.is_userdata() || value.is_lightuserdata())
    }

    #[inline]
    fn lua_islightuserdata(&self, idx: isize) -> bool {
        stack_api_value(self, idx).is_some_and(|value| value.is_lightuserdata())
    }

    #[inline]
    fn lua_isthread(&self, idx: isize) -> bool {
        stack_api_value(self, idx).is_some_and(|value| value.is_thread())
    }

    fn lua_settop(&mut self, idx: isize) -> LuaResult<()> {
        let base = stack_api_base(self);
        let current_top = stack_api_top(self);
        let current_len = current_top.saturating_sub(base) as isize;
        let new_len = if idx >= 0 { idx } else { current_len + idx + 1 };

        if new_len < 0 {
            return Err(self.error(format!("lua_settop: invalid stack index {}", idx)));
        }

        self.set_top(base + new_len as usize)?;
        Ok(())
    }

    #[inline]
    fn lua_absindex(&self, idx: isize) -> Option<isize> {
        if stack_api_pseudo_value(self, idx).is_some() {
            return Some(idx);
        }

        let base = stack_api_base(self) as isize;
        stack_api_resolve_index(self, idx).map(|abs| abs as isize - base + 1)
    }

    fn lua_rotate(&mut self, idx: isize, n: isize) -> LuaResult<()> {
        let abs = stack_api_resolve_index(self, idx)
            .ok_or_else(|| self.error(format!("lua_rotate: invalid stack index {}", idx)))?;
        let top = stack_api_top(self);
        let len = top.saturating_sub(abs);

        if len <= 1 {
            return Ok(());
        }

        let shift = n.rem_euclid(len as isize) as usize;
        if shift == 0 {
            return Ok(());
        }

        self.stack_mut()[abs..top].rotate_right(shift);
        Ok(())
    }

    #[inline]
    fn lua_pushvalue(&mut self, idx: isize) -> LuaResult<()> {
        let value = stack_api_expect_value(self, idx, "lua_pushvalue")?;
        self.push_value(value)
    }

    #[inline]
    fn lua_pushnil(&mut self) -> LuaResult<()> {
        self.push_value(LuaValue::nil())
    }

    #[inline]
    fn lua_pushboolean(&mut self, value: bool) -> LuaResult<()> {
        self.push_value(LuaValue::boolean(value))
    }

    #[inline]
    fn lua_pushinteger(&mut self, value: i64) -> LuaResult<()> {
        self.push_value(LuaValue::integer(value))
    }

    #[inline]
    fn lua_pushnumber(&mut self, value: f64) -> LuaResult<()> {
        self.push_value(LuaValue::number(value))
    }

    #[inline]
    fn lua_pushstring(&mut self, value: &str) -> LuaResult<()> {
        let value = self.create_string(value)?;
        self.push_value(value)
    }

    #[inline]
    fn lua_pushlstring(&mut self, value: &[u8]) -> LuaResult<()> {
        let value = self.create_bytes(value)?;
        self.push_value(value)
    }

    #[inline]
    fn lua_pushlightuserdata(&mut self, value: *mut c_void) -> LuaResult<()> {
        self.push_value(LuaValue::lightuserdata(value))
    }

    fn lua_pushcclosure(&mut self, func: crate::lua_vm::CFunction, n: usize) -> LuaResult<()> {
        if n == 0 {
            return self.push_value(LuaValue::cfunction(func));
        }

        let upvalues = stack_api_collect_upvalues(self, n, "lua_pushcclosure")?;
        let value = self.global_state_mut().create_c_closure(func, upvalues)?;
        self.push_value(value)
    }

    fn lua_pushrclosure<F>(&mut self, func: F, n: usize) -> LuaResult<()>
    where
        F: Fn(&mut crate::LuaState) -> LuaResult<usize> + 'static,
    {
        let upvalues = stack_api_collect_upvalues(self, n, "lua_pushrclosure")?;
        let value = self
            .global_state_mut()
            .create_closure_with_upvalues(func, upvalues)?;
        self.push_value(value)
    }

    #[inline]
    fn lua_argcount(&self) -> isize {
        self.arg_count() as isize
    }

    #[inline]
    fn lua_toboolean(&self, idx: isize) -> bool {
        stack_api_value(self, idx)
            .is_some_and(|value| !value.is_nil() && value.as_boolean() != Some(false))
    }

    #[inline]
    fn lua_tointegerx(&self, idx: isize) -> Option<i64> {
        stack_api_numeric_value(self, idx).and_then(|value| value.as_integer())
    }

    #[inline]
    fn lua_tonumberx(&self, idx: isize) -> Option<f64> {
        stack_api_numeric_value(self, idx).and_then(|value| value.as_number())
    }

    #[inline]
    fn lua_tostring(&self, idx: isize) -> Option<&str> {
        stack_api_str(self, idx)
    }

    #[inline]
    fn lua_tolstring(&self, idx: isize) -> Option<&[u8]> {
        stack_api_bytes(self, idx)
    }

    #[inline]
    fn lua_tostring_handle(&mut self, idx: isize) -> Option<LuaString> {
        let value = stack_api_value(self, idx)?;
        self.to_string_ref(value).map(LuaString::new)
    }

    fn lua_l_checkany(&mut self, idx: isize) -> LuaResult<()> {
        if stack_api_value(self, idx).is_some() {
            return Ok(());
        }

        Err(self.error(format!("bad argument #{} (value expected)", idx)))
    }

    fn lua_l_checkinteger(&mut self, idx: isize) -> LuaResult<i64> {
        let Some(value) = stack_api_value(self, idx) else {
            return Err(self.error(format!(
                "bad argument #{} (number expected, got no value)",
                idx
            )));
        };

        value.as_integer().ok_or_else(|| {
            self.error(format!(
                "bad argument #{} (integer expected, got {})",
                idx,
                value.type_name()
            ))
        })
    }

    fn lua_l_checknumber(&mut self, idx: isize) -> LuaResult<f64> {
        let Some(value) = stack_api_value(self, idx) else {
            return Err(self.error(format!(
                "bad argument #{} (number expected, got no value)",
                idx
            )));
        };

        value.as_number().ok_or_else(|| {
            self.error(format!(
                "bad argument #{} (number expected, got {})",
                idx,
                value.type_name()
            ))
        })
    }

    fn lua_l_checkstring(&mut self, idx: isize) -> LuaResult<String> {
        let Some(value) = stack_api_value(self, idx) else {
            return Err(self.error(format!(
                "bad argument #{} (string expected, got no value)",
                idx
            )));
        };

        value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
            self.error(format!(
                "bad argument #{} (string expected, got {})",
                idx,
                value.type_name()
            ))
        })
    }

    fn lua_l_checklstring(&mut self, idx: isize) -> LuaResult<Vec<u8>> {
        let Some(value) = stack_api_value(self, idx) else {
            return Err(self.error(format!(
                "bad argument #{} (string expected, got no value)",
                idx
            )));
        };

        value.as_bytes().map(|bytes| bytes.to_vec()).ok_or_else(|| {
            self.error(format!(
                "bad argument #{} (string expected, got {})",
                idx,
                value.type_name()
            ))
        })
    }

    #[inline]
    fn lua_l_optinteger(&mut self, idx: isize, default: i64) -> LuaResult<i64> {
        if self.lua_isnoneornil(idx) {
            Ok(default)
        } else {
            self.lua_l_checkinteger(idx)
        }
    }

    #[inline]
    fn lua_l_optnumber(&mut self, idx: isize, default: f64) -> LuaResult<f64> {
        if self.lua_isnoneornil(idx) {
            Ok(default)
        } else {
            self.lua_l_checknumber(idx)
        }
    }

    #[inline]
    fn lua_l_optstring(&mut self, idx: isize, default: &str) -> LuaResult<String> {
        if self.lua_isnoneornil(idx) {
            Ok(default.to_owned())
        } else {
            self.lua_l_checkstring(idx)
        }
    }

    #[inline]
    fn lua_l_optlstring(&mut self, idx: isize, default: &[u8]) -> LuaResult<Vec<u8>> {
        if self.lua_isnoneornil(idx) {
            Ok(default.to_vec())
        } else {
            self.lua_l_checklstring(idx)
        }
    }

    fn lua_createtable(&mut self, narr: usize, nrec: usize) -> LuaResult<()> {
        let value = self.create_table(narr, nrec)?;
        self.push_value(value)
    }

    #[inline]
    fn lua_newtable(&mut self) -> LuaResult<()> {
        self.lua_createtable(0, 0)
    }

    fn lua_upvaluecount(&self, idx: isize) -> usize {
        stack_api_function(self, idx).map_or(0, stack_api_upvaluecount)
    }

    fn lua_getupvalue(&mut self, func_idx: isize, n: usize) -> Option<String> {
        if n == 0 {
            return None;
        }

        let func = stack_api_function(self, func_idx)?;
        let up_idx = n - 1;

        if let Some(lua_func) = func.as_lua_function() {
            let upvalue_ptr = *lua_func.upvalues().get(up_idx)?;
            let name = lua_func
                .chunk()
                .upvalue_descs
                .get(up_idx)
                .map(|desc| desc.name.to_string())
                .unwrap_or_default();
            let value = upvalue_ptr.as_ref().data.get_value();
            self.push_value(value).ok()?;
            return Some(name);
        }

        if let Some(cclosure) = func.as_cclosure() {
            let value = *cclosure.upvalues().get(up_idx)?;
            self.push_value(value).ok()?;
            return Some(String::new());
        }

        if let Some(rclosure) = func.as_rclosure() {
            let value = *rclosure.upvalues().get(up_idx)?;
            self.push_value(value).ok()?;
            return Some(String::new());
        }

        None
    }

    fn lua_setupvalue(&mut self, func_idx: isize, n: usize) -> LuaResult<Option<String>> {
        if n == 0 {
            return Ok(None);
        }

        let func = stack_api_function(self, func_idx).ok_or_else(|| {
            self.error(format!(
                "lua_setupvalue: invalid function index {}",
                func_idx
            ))
        })?;
        let top = self.get_top();
        if top == 0 {
            return Err(self.error("lua_setupvalue: missing value on stack".to_string()));
        }

        let value = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_setupvalue: missing value on stack".to_string()))?;
        self.set_top_raw(top - 1);

        let up_idx = n - 1;

        if let Some(lua_func) = func.as_lua_function() {
            let Some(upvalue_ptr) = lua_func.upvalues().get(up_idx).copied() else {
                return Ok(None);
            };
            let name = lua_func
                .chunk()
                .upvalue_descs
                .get(up_idx)
                .map(|desc| desc.name.to_string())
                .unwrap_or_default();

            upvalue_ptr.as_mut_ref().data.set_value(value);
            if value.is_collectable()
                && let Some(value_gc_ptr) = value.as_gc_ptr()
            {
                self.gc_barrier(upvalue_ptr, value_gc_ptr);
            }
            return Ok(Some(name));
        }

        if let Some(cclosure) = func.as_cclosure_mut() {
            let Some(slot) = cclosure.upvalues_mut().get_mut(up_idx) else {
                return Ok(None);
            };
            *slot = value;
            if value.is_collectable()
                && let Some(owner) = func.as_cclosure_ptr()
            {
                self.gc_barrier_back(owner.into());
            }
            return Ok(Some(String::new()));
        }

        if let Some(rclosure) = func.as_rclosure_mut() {
            let Some(slot) = rclosure.upvalues_mut().get_mut(up_idx) else {
                return Ok(None);
            };
            *slot = value;
            if value.is_collectable()
                && let Some(owner) = func.as_rclosure_ptr()
            {
                self.gc_barrier_back(owner.into());
            }
            return Ok(Some(String::new()));
        }

        Ok(None)
    }

    fn lua_pushuserdata<T: UserDataTrait + 'static>(&mut self, data: T) -> LuaResult<()> {
        let value = self.create_userdata(LuaUserdata::new(data))?;
        self.push_value(value)
    }

    unsafe fn lua_pushuserdata_ref<T: UserDataTrait + 'static>(
        &mut self,
        reference: &mut T,
    ) -> LuaResult<()> {
        let value = unsafe { LuaState::create_userdata_ref(self, reference)? };
        self.push_value(value)
    }

    fn lua_touserdata_ref<T: 'static>(&mut self, idx: isize) -> Option<UserDataRef<T>> {
        let value = stack_api_value(self, idx)?;
        self.to_userdata_ref(value)
    }

    fn lua_createthread(&mut self, func_idx: isize) -> LuaResult<()> {
        let func = stack_api_function(self, func_idx).ok_or_else(|| {
            self.error(format!(
                "lua_createthread: invalid function index {}",
                func_idx
            ))
        })?;
        let value = self.global_state_mut().create_thread(func)?;
        self.push_value(value)
    }

    fn lua_pushthread(&mut self) -> LuaResult<bool> {
        self.push_value(LuaValue::thread(self.thread_ptr()))?;
        Ok(self.is_main_thread())
    }

    #[inline]
    fn lua_pushglobaltable(&mut self) -> LuaResult<()> {
        self.lua_pushvalue(LUA_GLOBALSINDEX)
    }

    fn lua_call(&mut self, nargs: usize, nresults: isize) -> LuaResult<()> {
        if self.call_depth() == 0 {
            return stack_api_call_fallback(self, nargs, nresults, "lua_call");
        }

        let top = self.get_top();
        let needed = nargs + 1;
        if top < needed {
            return Err(self.error(format!(
                "lua_call: expected function plus {} arguments on stack",
                nargs
            )));
        }

        let func_idx = top - needed;
        let result_count = self.call_stack_based(func_idx, nargs)?;
        stack_api_adjust_call_results(self, func_idx, result_count, nresults, "lua_call")
    }

    fn lua_pcall(&mut self, nargs: usize, nresults: isize) -> LuaResult<bool> {
        if self.call_depth() == 0 {
            return stack_api_pcall_fallback(self, nargs, nresults);
        }

        let top = self.get_top();
        let needed = nargs + 1;
        if top < needed {
            return Err(self.error(format!(
                "lua_pcall: expected function plus {} arguments on stack",
                nargs
            )));
        }

        let func_idx = top - needed;
        let (success, result_count) = self.pcall_stack_based(func_idx, nargs)?;
        if success {
            stack_api_adjust_call_results(self, func_idx, result_count, nresults, "lua_pcall")?;
        }
        Ok(success)
    }

    fn lua_rawlen(&mut self, idx: isize) -> LuaResult<usize> {
        let value = stack_api_value(self, idx)
            .ok_or_else(|| self.error(format!("lua_rawlen: invalid stack index {}", idx)))?;

        let len = if let Some(bytes) = value.as_bytes() {
            bytes.len()
        } else if let Some(table) = value.as_table() {
            table.len()
        } else {
            0
        };

        Ok(len)
    }

    fn lua_len(&mut self, idx: isize) -> LuaResult<()> {
        let value = stack_api_expect_value(self, idx, "lua_len")?;
        let len = self.obj_len(&value)?;
        self.push_value(LuaValue::integer(len))
    }

    fn lua_next(&mut self, idx: isize) -> LuaResult<bool> {
        let table = stack_api_expect_value(self, idx, "lua_next")?;
        let table_ref = table.as_table().ok_or_else(|| {
            self.error(format!(
                "lua_next: expected table at index {}, got {}",
                idx,
                table.type_name()
            ))
        })?;

        let top = self.get_top();
        if top == 0 {
            return Err(self.error("lua_next: missing key on stack".to_string()));
        }

        let key = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_next: missing key on stack".to_string()))?;
        let result = table_ref
            .next(&key)
            .map_err(|_| self.error("lua_next: invalid key".to_string()))?;
        self.set_top_raw(top - 1);

        if let Some((next_key, next_value)) = result {
            self.push_value(next_key)?;
            self.push_value(next_value)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    #[inline]
    fn lua_getglobal(&mut self, name: &str) -> LuaResult<()> {
        self.lua_getfield(LUA_GLOBALSINDEX, name)
    }

    #[inline]
    fn lua_setglobal(&mut self, name: &str) -> LuaResult<()> {
        self.lua_setfield(LUA_GLOBALSINDEX, name)
    }

    fn lua_rawgetglobal(&mut self, name: &str) -> LuaResult<()> {
        let global = stack_api_expect_value(self, LUA_GLOBALSINDEX, "lua_rawgetglobal")?;
        let key = self.create_string(name)?;
        let value = self.raw_get(&global, &key).unwrap_or_else(LuaValue::nil);
        self.push_value(value)
    }

    fn lua_rawsetglobal(&mut self, name: &str) -> LuaResult<()> {
        let top = self.get_top();
        if top == 0 {
            return Err(self.error("lua_rawsetglobal: missing value on stack".to_string()));
        }

        let global = stack_api_expect_value(self, LUA_GLOBALSINDEX, "lua_rawsetglobal")?;
        let key = self.create_string(name)?;
        let value = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_rawsetglobal: missing value on stack".to_string()))?;
        self.raw_set(&global, key, value);
        self.set_top_raw(top - 1);
        Ok(())
    }

    #[inline]
    fn lua_registry_geti(&mut self, index: i64) -> LuaResult<()> {
        self.lua_rawgeti(LUA_REGISTRYINDEX, index)
    }

    #[inline]
    fn lua_registry_seti(&mut self, index: i64) -> LuaResult<()> {
        self.lua_rawseti(LUA_REGISTRYINDEX, index)
    }

    fn lua_gettable(&mut self, idx: isize) -> LuaResult<()> {
        let top = self.get_top();
        if top == 0 {
            return Err(self.error("lua_gettable: missing key on stack".to_string()));
        }

        let table = stack_api_expect_value(self, idx, "lua_gettable")?;
        let key = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_gettable: missing key on stack".to_string()))?;
        let value = self.table_get(&table, &key)?.unwrap_or_else(LuaValue::nil);
        self.set_top_raw(top - 1);
        self.push_value(value)
    }

    fn lua_settable(&mut self, idx: isize) -> LuaResult<()> {
        let top = self.get_top();
        if top < 2 {
            return Err(self.error("lua_settable: missing key/value on stack".to_string()));
        }

        let table = stack_api_expect_value(self, idx, "lua_settable")?;
        let key = self
            .stack_get(top - 2)
            .ok_or_else(|| self.error("lua_settable: missing key on stack".to_string()))?;
        let value = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_settable: missing value on stack".to_string()))?;
        self.table_set(&table, key, value)?;
        self.set_top_raw(top - 2);
        Ok(())
    }

    fn lua_getfield(&mut self, idx: isize, key: &str) -> LuaResult<()> {
        let table = stack_api_expect_value(self, idx, "lua_getfield")?;
        let key = self.create_string(key)?;
        let value = self.table_get(&table, &key)?.unwrap_or_else(LuaValue::nil);
        self.push_value(value)
    }

    fn lua_setfield(&mut self, idx: isize, key: &str) -> LuaResult<()> {
        let top = self.get_top();
        if top == 0 {
            return Err(self.error("lua_setfield: missing value on stack".to_string()));
        }

        let table = stack_api_expect_value(self, idx, "lua_setfield")?;
        let key = self.create_string(key)?;
        let value = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_setfield: missing value on stack".to_string()))?;
        self.table_set(&table, key, value)?;
        self.set_top_raw(top - 1);
        Ok(())
    }

    fn lua_geti(&mut self, idx: isize, key: i64) -> LuaResult<()> {
        let table = stack_api_expect_value(self, idx, "lua_geti")?;
        let value = self.table_geti(&table, key)?;
        self.push_value(value)
    }

    fn lua_seti(&mut self, idx: isize, key: i64) -> LuaResult<()> {
        let top = self.get_top();
        if top == 0 {
            return Err(self.error("lua_seti: missing value on stack".to_string()));
        }

        let table = stack_api_expect_value(self, idx, "lua_seti")?;
        let value = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_seti: missing value on stack".to_string()))?;
        self.table_seti(&table, key, value)?;
        self.set_top_raw(top - 1);
        Ok(())
    }

    fn lua_rawget(&mut self, idx: isize) -> LuaResult<()> {
        let top = self.get_top();
        if top == 0 {
            return Err(self.error("lua_rawget: missing key on stack".to_string()));
        }

        let table = stack_api_expect_value(self, idx, "lua_rawget")?;
        let key = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_rawget: missing key on stack".to_string()))?;
        let value = self.raw_get(&table, &key).unwrap_or_else(LuaValue::nil);
        self.set_top_raw(top - 1);
        self.push_value(value)
    }

    fn lua_rawset(&mut self, idx: isize) -> LuaResult<()> {
        let top = self.get_top();
        if top < 2 {
            return Err(self.error("lua_rawset: missing key/value on stack".to_string()));
        }

        let table = stack_api_expect_value(self, idx, "lua_rawset")?;
        let key = self
            .stack_get(top - 2)
            .ok_or_else(|| self.error("lua_rawset: missing key on stack".to_string()))?;
        let value = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_rawset: missing value on stack".to_string()))?;
        self.raw_set(&table, key, value);
        self.set_top_raw(top - 2);
        Ok(())
    }

    fn lua_rawgeti(&mut self, idx: isize, index: i64) -> LuaResult<()> {
        let table = stack_api_expect_value(self, idx, "lua_rawgeti")?;
        let value = self.raw_geti(&table, index).unwrap_or_else(LuaValue::nil);
        self.push_value(value)
    }

    fn lua_rawseti(&mut self, idx: isize, index: i64) -> LuaResult<()> {
        let top = self.get_top();
        if top == 0 {
            return Err(self.error("lua_rawseti: missing value on stack".to_string()));
        }

        let table = stack_api_expect_value(self, idx, "lua_rawseti")?;
        let value = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_rawseti: missing value on stack".to_string()))?;
        self.raw_seti(&table, index, value);
        self.set_top_raw(top - 1);
        Ok(())
    }

    fn lua_copy(&mut self, from_idx: isize, to_idx: isize) -> LuaResult<()> {
        let value = stack_api_expect_value(self, from_idx, "lua_copy")?;
        stack_api_assign_value(self, to_idx, value, "lua_copy")
    }

    fn lua_replace(&mut self, idx: isize) -> LuaResult<()> {
        let top = self.get_top();
        if top == 0 {
            return Err(self.error("lua_replace: missing value on stack".to_string()));
        }

        let value = self
            .stack_get(top - 1)
            .ok_or_else(|| self.error("lua_replace: missing value on stack".to_string()))?;
        self.set_top_raw(top - 1);
        stack_api_assign_value(self, idx, value, "lua_replace")
    }

    fn lua_upvalueid(&mut self, func_idx: isize, n: usize) -> Option<*mut c_void> {
        if n == 0 {
            return None;
        }

        let func = stack_api_function(self, func_idx)?;
        let up_idx = n - 1;

        if let Some(lua_func) = func.as_lua_function() {
            let upvalue = lua_func.upvalues().get(up_idx)?;
            return Some(upvalue.as_ptr() as *mut c_void);
        }

        if let Some(cclosure) = func.as_cclosure() {
            let upvalue = cclosure.upvalues().get(up_idx)?;
            return Some(upvalue as *const _ as *mut c_void);
        }

        if let Some(rclosure) = func.as_rclosure() {
            let upvalue = rclosure.upvalues().get(up_idx)?;
            return Some(upvalue as *const _ as *mut c_void);
        }

        None
    }

    fn lua_upvaluejoin(
        &mut self,
        func1_idx: isize,
        n1: usize,
        func2_idx: isize,
        n2: usize,
    ) -> LuaResult<bool> {
        if n1 == 0 || n2 == 0 {
            return Ok(false);
        }

        let func1 = stack_api_function(self, func1_idx).ok_or_else(|| {
            self.error(format!(
                "lua_upvaluejoin: invalid function index {}",
                func1_idx
            ))
        })?;
        let func2 = stack_api_function(self, func2_idx).ok_or_else(|| {
            self.error(format!(
                "lua_upvaluejoin: invalid function index {}",
                func2_idx
            ))
        })?;

        if !func1.is_lua_function() || !func2.is_lua_function() {
            return Err(self.error("lua_upvaluejoin: Lua function expected".to_string()));
        }

        let shared_upvalue = {
            let lua_func2 = func2.as_lua_function().ok_or_else(|| {
                self.error("lua_upvaluejoin: function 2 is not a Lua function".to_string())
            })?;
            let Some(upvalue) = lua_func2.upvalues().get(n2 - 1).copied() else {
                return Ok(false);
            };
            upvalue
        };

        let lua_func1 = func1.as_lua_function_mut().ok_or_else(|| {
            self.error("lua_upvaluejoin: function 1 is not a Lua function".to_string())
        })?;
        let Some(slot) = lua_func1.upvalues_mut().get_mut(n1 - 1) else {
            return Ok(false);
        };
        *slot = shared_upvalue;

        if let Some(owner) = func1.as_function_ptr() {
            self.gc_barrier_back(owner.into());
        }

        Ok(true)
    }
}

impl StackValueApi for LuaState {
    fn collect_values<T: IntoLua>(&mut self, value: T, api_name: &str) -> LuaResult<Vec<LuaValue>> {
        let base_top = self.get_top();

        let pushed = match value.into_lua(self) {
            Ok(pushed) => pushed,
            Err(err) => {
                self.set_top_raw(base_top);
                return Err(self.error(format!("{}: {}", api_name, err)));
            }
        };

        let mut values = Vec::with_capacity(pushed);
        for index in base_top..base_top + pushed {
            let Some(value) = self.stack_get(index) else {
                self.set_top_raw(base_top);
                return Err(self.error(format!(
                    "{}: internal error: failed to collect Lua values from stack",
                    api_name
                )));
            };
            values.push(value);
        }
        self.set_top_raw(base_top);

        Ok(values)
    }

    fn collect_single_value<T: IntoLua>(
        &mut self,
        value: T,
        api_name: &str,
    ) -> LuaResult<LuaValue> {
        let base_top = self.get_top();

        let pushed = match value.into_lua(self) {
            Ok(pushed) => pushed,
            Err(err) => {
                self.set_top_raw(base_top);
                return Err(self.error(format!("{}: {}", api_name, err)));
            }
        };

        if pushed != 1 {
            self.set_top_raw(base_top);
            return Err(self.error(format!(
                "{} expects exactly one Lua value, got {}",
                api_name, pushed
            )));
        }

        let Some(value) = self.stack_get(base_top) else {
            self.set_top_raw(base_top);
            return Err(self.error(format!(
                "{}: internal error: failed to collect Lua value from stack",
                api_name
            )));
        };
        self.set_top_raw(base_top);

        Ok(value)
    }

    #[inline]
    fn from_value<T: FromLua>(&mut self, value: LuaValue, api_name: &str) -> LuaResult<T> {
        T::from_lua(value, self).map_err(|msg| self.error(format!("{}: {}", api_name, msg)))
    }
}
