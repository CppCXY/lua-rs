use super::*;
use super::families::{
    CarriedFloatGuardValue, CarriedFloatLoopStep, CarriedFloatRhs, CarriedIntegerLoopStep,
    CarriedIntegerRhs, CurrentNumericGuardValues, HoistedNumericGuardSource,
    HoistedNumericGuardValue, HoistedNumericGuardValues, ResolvedCarriedFloatRhs,
    ResolvedCarriedIntegerRhs,
};

pub(super) fn emit_numeric_guard_block(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    block: &NumericJmpLoopGuardBlock,
    continue_block: Block,
    exit_block: Block,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
    current_numeric_values: &mut CurrentNumericGuardValues,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<()> {
    for step in &block.pre_steps {
        emit_numeric_step(
            builder,
            abi,
            native_helpers,
            hits_var,
            current_hits,
            exit_block,
            *step,
            known_value_kinds,
            current_numeric_values,
            carried_float,
            hoisted_numeric,
        )?;
    }

    let (cond, continue_when, continue_preset, exit_preset) = match block.guard {
        NumericJmpLoopGuard::Head {
            cond,
            continue_when,
            continue_preset,
            exit_preset,
            ..
        }
        | NumericJmpLoopGuard::Tail {
            cond,
            continue_when,
            continue_preset,
            exit_preset,
            ..
        } => (cond, continue_when, continue_preset, exit_preset),
    };

    emit_numeric_guard_flow(
        builder,
        abi,
        native_helpers,
        hits_var,
        current_hits,
        fallback_block,
        cond,
        continue_when,
        continue_preset.as_ref(),
        exit_preset.as_ref(),
        continue_block,
        exit_block,
        known_value_kinds,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )
}

#[derive(Clone, Copy)]
pub(super) struct NativeAbi {
    pub(super) pointer_ty: Type,
    pub(super) base_slots: Value,
    pub(super) base_ptr: Value,
    pub(super) constants_ptr: Value,
    pub(super) constants_len: Value,
    pub(super) lua_state_ptr: Value,
    pub(super) upvalue_ptrs: Value,
    pub(super) result_ptr: Value,
}

#[derive(Clone, Copy)]
pub(super) struct NativeHelpers {
    pub(super) get_upval: FuncRef,
    pub(super) set_upval: FuncRef,
    pub(super) get_tabup_field: FuncRef,
    pub(super) set_tabup_field: FuncRef,
    pub(super) get_table_int: FuncRef,
    pub(super) set_table_int: FuncRef,
    pub(super) get_table_field: FuncRef,
    pub(super) set_table_field: FuncRef,
    pub(super) len: FuncRef,
    pub(super) numeric_binary: FuncRef,
    pub(super) numeric_pow: FuncRef,
    pub(super) shift_left: FuncRef,
    pub(super) shift_right: FuncRef,
    pub(super) call: FuncRef,
    pub(super) tfor_call: FuncRef,
}

#[derive(Clone, Copy)]
pub(super) enum NativeReturnKind {
    Return,
    Return0,
    Return1,
}

pub(super) fn init_native_entry(builder: &mut FunctionBuilder<'_>, pointer_ty: Type) -> NativeAbi {
    let entry_block = builder.create_block();
    builder.append_block_params_for_function_params(entry_block);
    builder.switch_to_block(entry_block);
    builder.seal_block(entry_block);

    let params = builder.block_params(entry_block).to_vec();
    let stack_ptr = params[0];
    let base_slots = params[1];
    let constants_ptr = params[2];
    let constants_len = params[3];
    let lua_state_ptr = params[4];
    let upvalue_ptrs = params[5];
    let result_ptr = params[6];
    let slot_scale = builder.ins().iconst(pointer_ty, LUA_VALUE_SIZE);
    let base_bytes = builder.ins().imul(base_slots, slot_scale);
    let base_ptr = builder.ins().iadd(stack_ptr, base_bytes);

    NativeAbi {
        pointer_ty,
        base_slots,
        base_ptr,
        constants_ptr,
        constants_len,
        lua_state_ptr,
        upvalue_ptrs,
        result_ptr,
    }
}

pub(super) fn make_native_context(target_config: TargetFrontendConfig) -> cranelift_codegen::Context {
    let mut context = cranelift_codegen::Context::new();
    context.func.signature.call_conv = target_config.default_call_conv;
    let pointer_ty = target_config.pointer_type();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(pointer_ty));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(pointer_ty));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(pointer_ty));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(pointer_ty));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(pointer_ty));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(pointer_ty));
    context
        .func
        .signature
        .params
        .push(AbiParam::new(pointer_ty));
    context
}

pub(super) fn declare_native_helpers(
    module: &mut JITModule,
    func: &mut cranelift_codegen::ir::Function,
    pointer_ty: Type,
    call_conv: CallConv,
) -> Result<NativeHelpers, String> {
    fn import_helper(
        module: &mut JITModule,
        func: &mut cranelift_codegen::ir::Function,
        name: &str,
        params: &[Type],
        returns: &[Type],
        call_conv: CallConv,
    ) -> Result<FuncRef, String> {
        let mut sig = module.make_signature();
        sig.call_conv = call_conv;
        for param in params {
            sig.params.push(AbiParam::new(*param));
        }
        for ret in returns {
            sig.returns.push(AbiParam::new(*ret));
        }
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|err| err.to_string())?;
        Ok(module.declare_func_in_func(func_id, func))
    }

    Ok(NativeHelpers {
        get_upval: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_GET_UPVAL_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        set_upval: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_SET_UPVAL_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        get_tabup_field: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_GET_TABUP_FIELD_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        set_tabup_field: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_SET_TABUP_FIELD_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        get_table_int: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_GET_TABLE_INT_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        set_table_int: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_SET_TABLE_INT_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        get_table_field: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_GET_TABLE_FIELD_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        set_table_field: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_SET_TABLE_FIELD_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        len: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_LEN_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        numeric_binary: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_BINARY_SYMBOL,
            &[
                pointer_ty,
                pointer_ty,
                pointer_ty,
                pointer_ty,
                types::I32,
                types::I64,
                types::I32,
                types::I64,
                types::I32,
            ],
            &[types::I32],
            call_conv,
        )?,
        numeric_pow: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_POW_SYMBOL,
            &[types::F64, types::F64],
            &[types::F64],
            call_conv,
        )?,
        shift_left: import_helper(
            module,
            func,
            NATIVE_HELPER_SHIFT_LEFT_SYMBOL,
            &[types::I64, types::I64],
            &[types::I64],
            call_conv,
        )?,
        shift_right: import_helper(
            module,
            func,
            NATIVE_HELPER_SHIFT_RIGHT_SYMBOL,
            &[types::I64, types::I64],
            &[types::I64],
            call_conv,
        )?,
        call: import_helper(
            module,
            func,
            NATIVE_HELPER_CALL_SYMBOL,
            &[
                pointer_ty,
                pointer_ty,
                pointer_ty,
                types::I32,
                types::I32,
                types::I32,
                types::I32,
            ],
            &[types::I32],
            call_conv,
        )?,
        tfor_call: import_helper(
            module,
            func,
            NATIVE_HELPER_TFOR_CALL_SYMBOL,
            &[pointer_ty, pointer_ty, types::I32, types::I32, types::I32],
            &[types::I32],
            call_conv,
        )?,
    })
}

pub(super) unsafe extern "C" fn jit_native_helper_numeric_get_upval(
    dst_ptr: *mut LuaValue,
    upvalue_ptrs: *const UpvaluePtr,
    upvalue_index: usize,
) -> i32 {
    if upvalue_ptrs.is_null() {
        return 0;
    }

    let upvalue_ptr = unsafe { *upvalue_ptrs.add(upvalue_index) };
    let src = upvalue_ptr.as_ref().data.get_value_ref();
    unsafe {
        (*dst_ptr).value = src.value;
        (*dst_ptr).tt = src.tt;
    }
    1
}

pub(super) unsafe extern "C" fn jit_native_helper_numeric_set_upval(
    lua_state: *mut LuaState,
    upvalue_ptrs: *const UpvaluePtr,
    src_ptr: *const LuaValue,
    upvalue_index: usize,
) -> i32 {
    if upvalue_ptrs.is_null() {
        return 0;
    }

    let value = unsafe { *src_ptr };
    let upvalue_ptr = unsafe { *upvalue_ptrs.add(upvalue_index) };
    upvalue_ptr
        .as_mut_ref()
        .data
        .set_value_parts(value.value, value.tt);

    if value.tt & 0x40 != 0 {
        let Some(gc_ptr) = value.as_gc_ptr() else {
            return 0;
        };
        if lua_state.is_null() {
            return 0;
        }
        unsafe { (*lua_state).gc_barrier(upvalue_ptr, gc_ptr) };
    }

    1
}

pub(super) unsafe extern "C" fn jit_native_helper_numeric_get_tabup_field(
    lua_state: *mut LuaState,
    dst_ptr: *mut LuaValue,
    upvalue_ptrs: *const UpvaluePtr,
    upvalue_index: usize,
    key_ptr: *const LuaValue,
) -> i32 {
    if upvalue_ptrs.is_null() {
        return 0;
    }
    debug_assert!(unsafe { (*key_ptr).is_short_string() });

    let upvalue_ptr = unsafe { *upvalue_ptrs.add(upvalue_index) };
    let table_value = upvalue_ptr.as_ref().data.get_value_ref();
    if !table_value.is_table() {
        return 0;
    }

    let table = table_value.hvalue();
    if table.impl_table.has_hash() {
        let loaded_tt = unsafe {
            table
                .impl_table
                .get_shortstr_tagged_into(&*key_ptr, dst_ptr)
        };
        if loaded_tt != 0 {
            // Accept any non-nil result (table, number, string, etc.) so that
            // intermediate table-reference lookups like `_ENV.some_table` succeed.
            // Downstream steps (Binary, GetTableField) have their own type guards.
            return 1;
        }
    }

    if lua_state.is_null() {
        return 0;
    }

    let lua_state = unsafe { &mut *lua_state };
    let key_value = unsafe { &*key_ptr };
    match finishget_known_miss(lua_state, table_value, key_value) {
        Ok(Some(value)) => {
            unsafe { *dst_ptr = value };
            1
        }
        _ => 0,
    }
}

pub(super) unsafe extern "C" fn jit_native_helper_numeric_set_tabup_field(
    lua_state: *mut LuaState,
    upvalue_ptrs: *const UpvaluePtr,
    upvalue_index: usize,
    key_ptr: *const LuaValue,
    value_ptr: *const LuaValue,
) -> i32 {
    if upvalue_ptrs.is_null() {
        return 0;
    }
    debug_assert!(unsafe { (*key_ptr).is_short_string() });

    let upvalue_ptr = unsafe { *upvalue_ptrs.add(upvalue_index) };
    let table_value = upvalue_ptr.as_ref().data.get_value_ref();
    if !table_value.is_table() {
        return 0;
    }

    let table = table_value.hvalue_mut();
    let value = unsafe { *value_ptr };
    if !table
        .impl_table
        .set_existing_shortstr_parts(unsafe { &*key_ptr }, value.value, value.tt)
    {
        return 0;
    }

    if value.tt & 0x40 != 0 {
        if lua_state.is_null() {
            return 0;
        }
        unsafe { (*lua_state).gc_barrier_back(table_value.as_gc_ptr_table_unchecked()) };
    }

    1
}

pub(super) unsafe extern "C" fn jit_native_helper_numeric_get_table_int(
    lua_state: *mut LuaState,
    dst_ptr: *mut LuaValue,
    table_ptr: *const LuaValue,
    index_ptr: *const LuaValue,
) -> i32 {
    if unsafe { !(*table_ptr).is_table() || !pttisinteger(index_ptr) } {
        return 0;
    }

    let table = unsafe { (*table_ptr).hvalue() };
    let index = unsafe { pivalue(index_ptr) };
    let loaded = unsafe { table.impl_table.fast_geti_into(index, dst_ptr) }
        || unsafe { table.impl_table.get_int_from_hash_into(index, dst_ptr) };
    if loaded {
        let loaded_value = unsafe { &*dst_ptr };
        return i32::from(ttisinteger(loaded_value) || ttisfloat(loaded_value));
    }

    if lua_state.is_null() {
        return 0;
    }

    let lua_state = unsafe { &mut *lua_state };
    let table_value = unsafe { &*table_ptr };
    let index_value = unsafe { &*index_ptr };
    match finishget_known_miss(lua_state, table_value, index_value) {
        Ok(Some(value)) if value.is_integer() || value.is_float() => {
            unsafe { *dst_ptr = value };
            1
        }
        _ => 0,
    }
}

pub(super) unsafe extern "C" fn jit_native_helper_numeric_set_table_int(
    lua_state: *mut LuaState,
    table_ptr: *const LuaValue,
    index_ptr: *const LuaValue,
    value_ptr: *const LuaValue,
) -> i32 {
    if unsafe { !(*table_ptr).is_table() || !pttisinteger(index_ptr) } {
        return 0;
    }

    let table = unsafe { (*table_ptr).hvalue_mut() };
    let index = unsafe { pivalue(index_ptr) };
    let value = unsafe { *value_ptr };
    if !table
        .impl_table
        .fast_seti_parts(index, value.value, value.tt)
    {
        return 0;
    }

    if value.tt & 0x40 != 0 {
        if lua_state.is_null() {
            return 0;
        }
        unsafe { (*lua_state).gc_barrier_back((*table_ptr).as_gc_ptr_table_unchecked()) };
    }

    1
}

pub(super) unsafe extern "C" fn jit_native_helper_numeric_get_table_field(
    lua_state: *mut LuaState,
    dst_ptr: *mut LuaValue,
    table_ptr: *const LuaValue,
    key_ptr: *const LuaValue,
) -> i32 {
    if unsafe { !(*table_ptr).is_table() } {
        return 0;
    }
    debug_assert!(unsafe { (*key_ptr).is_short_string() });

    let table = unsafe { (*table_ptr).hvalue() };
    if table.impl_table.has_hash() {
        let loaded_tt = unsafe {
            table
                .impl_table
                .get_shortstr_tagged_into(&*key_ptr, dst_ptr)
        };
        if loaded_tt != 0 {
            return i32::from(loaded_tt == LUA_VNUMINT || loaded_tt == LUA_VNUMFLT);
        }
    }

    if lua_state.is_null() {
        return 0;
    }

    let lua_state = unsafe { &mut *lua_state };
    let table_value = unsafe { &*table_ptr };
    let key_value = unsafe { &*key_ptr };
    match finishget_known_miss(lua_state, table_value, key_value) {
        Ok(Some(value)) if value.is_integer() || value.is_float() => {
            unsafe { *dst_ptr = value };
            1
        }
        _ => 0,
    }
}

pub(super) unsafe extern "C" fn jit_native_helper_numeric_set_table_field(
    lua_state: *mut LuaState,
    table_ptr: *const LuaValue,
    key_ptr: *const LuaValue,
    value_ptr: *const LuaValue,
) -> i32 {
    if unsafe { !(*table_ptr).is_table() } {
        return 0;
    }
    debug_assert!(unsafe { (*key_ptr).is_short_string() });

    let table = unsafe { (*table_ptr).hvalue_mut() };
    let value = unsafe { *value_ptr };
    if !table
        .impl_table
        .set_existing_shortstr_parts(unsafe { &*key_ptr }, value.value, value.tt)
    {
        return 0;
    }

    if value.tt & 0x40 != 0 {
        if lua_state.is_null() {
            return 0;
        }
        unsafe { (*lua_state).gc_barrier_back((*table_ptr).as_gc_ptr_table_unchecked()) };
    }

    1
}

pub(super) unsafe extern "C" fn jit_native_helper_numeric_len(
    lua_state: *mut LuaState,
    dst_ptr: *mut LuaValue,
    value_ptr: *const LuaValue,
) -> i32 {
    if value_ptr.is_null() {
        return 0;
    }

    let value = unsafe { *value_ptr };
    if let Some(bytes) = value.as_bytes() {
        unsafe { *dst_ptr = LuaValue::integer(bytes.len() as i64) };
        return 1;
    }

    if lua_state.is_null() {
        return 0;
    }

    let lua_state = unsafe { &mut *lua_state };
    match objlen_value(lua_state, value) {
        Ok(result) if result.is_integer() || result.is_float() => {
            unsafe { *dst_ptr = result };
            1
        }
        _ => 0,
    }
}

pub(super) extern "C" fn jit_native_helper_shift_left(lhs: i64, rhs: i64) -> i64 {
    crate::lua_vm::execute::helper::lua_shiftl(lhs, rhs)
}

pub(super) extern "C" fn jit_native_helper_shift_right(lhs: i64, rhs: i64) -> i64 {
    crate::lua_vm::execute::helper::lua_shiftr(lhs, rhs)
}

pub(super) extern "C" fn jit_native_helper_numeric_pow(lhs: f64, rhs: f64) -> f64 {
    luai_numpow(lhs, rhs)
}

pub(super) unsafe extern "C" fn jit_native_helper_tfor_call(
    lua_state: *mut LuaState,
    base: usize,
    a: u32,
    c: u32,
    pc: u32,
) -> i32 {
    if lua_state.is_null() {
        return NATIVE_TFOR_CALL_FALLBACK;
    }

    let lua_state = unsafe { &mut *lua_state };
    if lua_state.hook_mask != 0 {
        return NATIVE_TFOR_CALL_FALLBACK;
    }

    if lua_state.current_frame().is_none() {
        return NATIVE_TFOR_CALL_FALLBACK;
    }

    let ra = base + a as usize;
    let func_idx = ra + 3;
    if func_idx + 2 >= lua_state.stack_len() {
        return NATIVE_TFOR_CALL_FALLBACK;
    }

    unsafe {
        let stack = lua_state.stack_mut();
        *stack.get_unchecked_mut(ra + 5) = *stack.get_unchecked(ra + 3);
        *stack.get_unchecked_mut(ra + 4) = *stack.get_unchecked(ra + 1);
        *stack.get_unchecked_mut(ra + 3) = *stack.get_unchecked(ra);
    }

    lua_state.set_top_raw(func_idx + 3);
    let Some(ci) = lua_state.current_frame_mut() else {
        return NATIVE_TFOR_CALL_FALLBACK;
    };
    ci.save_pc(pc as usize);

    match precall(lua_state, func_idx, 2, c as i32) {
        Ok(true) => NATIVE_TFOR_CALL_LUA_RETURNED,
        Ok(false) => NATIVE_TFOR_CALL_C_CONTINUE,
        Err(_) => NATIVE_TFOR_CALL_FALLBACK,
    }
}

pub(super) unsafe extern "C" fn jit_native_helper_call(
    lua_state: *mut LuaState,
    base_ptr: *const LuaValue,
    base: usize,
    a: u32,
    b: u32,
    c: u32,
    pc: u32,
) -> i32 {
    if lua_state.is_null() {
        return NATIVE_CALL_FALLBACK;
    }

    let lua_state = unsafe { &mut *lua_state };
    if lua_state.hook_mask != 0 {
        return NATIVE_CALL_FALLBACK;
    }

    if lua_state.current_frame().is_none() {
        return NATIVE_CALL_FALLBACK;
    }

    if base_ptr.is_null() {
        return NATIVE_CALL_FALLBACK;
    }

    let func_idx = base + a as usize;
    if func_idx >= lua_state.stack_len() {
        return NATIVE_CALL_FALLBACK;
    }

    let nargs = if b != 0 {
        let top = func_idx + b as usize;
        if top > lua_state.stack_len() {
            return NATIVE_CALL_FALLBACK;
        }
        unsafe {
            std::ptr::copy(
                base_ptr.add(a as usize),
                lua_state.stack_mut().as_mut_ptr().add(func_idx),
                b as usize,
            );
        }
        lua_state.set_top_raw(top);
        b as usize - 1
    } else {
        let top = lua_state.get_top();
        if top <= func_idx {
            return NATIVE_CALL_FALLBACK;
        }
        top - func_idx - 1
    };

    let caller_depth = lua_state.call_depth();
    let Some(ci) = lua_state.current_frame_mut() else {
        return NATIVE_CALL_FALLBACK;
    };
    ci.save_pc(pc as usize);

    let func = unsafe { *lua_state.stack().get_unchecked(func_idx) };
    let c_func = if let Some(c_func) = func.as_cfunction() {
        Some(c_func)
    } else {
        func.as_cclosure().map(|closure| closure.func())
    };

    if let Some(c_func) = c_func
        && let Some(result) = try_call_fast_math(lua_state, c_func, func_idx, nargs, c as i32 - 1)
    {
        return match result {
            Ok(()) => NATIVE_CALL_CONTINUE,
            Err(_) => NATIVE_CALL_FALLBACK,
        };
    }

    match precall(lua_state, func_idx, nargs, c as i32 - 1) {
        Ok(true) => {
            if lua_state.inc_n_ccalls().is_err() {
                return NATIVE_CALL_FALLBACK;
            }
            let result = lua_execute(lua_state, caller_depth);
            lua_state.dec_n_ccalls();
            match result {
                Ok(()) => NATIVE_CALL_CONTINUE,
                Err(_) => NATIVE_CALL_FALLBACK,
            }
        }
        Ok(false) => NATIVE_CALL_CONTINUE,
        Err(_) => NATIVE_CALL_FALLBACK,
    }
}

pub(super) unsafe extern "C" fn jit_native_helper_numeric_binary(
    dst_ptr: *mut LuaValue,
    base_ptr: *const LuaValue,
    constants_ptr: *const LuaValue,
    constants_len: usize,
    lhs_kind: i32,
    lhs_payload: i64,
    rhs_kind: i32,
    rhs_payload: i64,
    op: i32,
) -> i32 {
    unsafe fn operand_ptr(
        base_ptr: *const LuaValue,
        constants_ptr: *const LuaValue,
        constants_len: usize,
        kind: i32,
        payload: i64,
        immediate: &mut LuaValue,
    ) -> Option<*const LuaValue> {
        match kind {
            NATIVE_NUMERIC_OPERAND_REG => {
                let reg = usize::try_from(payload).ok()?;
                Some(unsafe { base_ptr.add(reg) })
            }
            NATIVE_NUMERIC_OPERAND_IMM_I => {
                *immediate = LuaValue::integer(payload);
                Some(immediate as *const LuaValue)
            }
            NATIVE_NUMERIC_OPERAND_CONST => {
                let index = usize::try_from(payload).ok()?;
                if index >= constants_len {
                    return None;
                }
                Some(unsafe { constants_ptr.add(index) })
            }
            _ => None,
        }
    }

    let mut lhs_immediate = LuaValue::nil();
    let mut rhs_immediate = LuaValue::nil();
    let Some(lhs_ptr) = (unsafe {
        operand_ptr(
            base_ptr,
            constants_ptr,
            constants_len,
            lhs_kind,
            lhs_payload,
            &mut lhs_immediate,
        )
    }) else {
        return 0;
    };
    let Some(rhs_ptr) = (unsafe {
        operand_ptr(
            base_ptr,
            constants_ptr,
            constants_len,
            rhs_kind,
            rhs_payload,
            &mut rhs_immediate,
        )
    }) else {
        return 0;
    };

    let lhs = unsafe { &*lhs_ptr };
    let rhs = unsafe { &*rhs_ptr };
    let result = match op {
        NATIVE_NUMERIC_BINARY_ADD => {
            if let (Some(lhs_int), Some(rhs_int)) =
                (lhs.as_integer_strict(), rhs.as_integer_strict())
            {
                LuaValue::integer(lhs_int.wrapping_add(rhs_int))
            } else {
                let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
                let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
                if lhs_num.is_nan() || rhs_num.is_nan() {
                    return 0;
                }
                LuaValue::float(lhs_num + rhs_num)
            }
        }
        NATIVE_NUMERIC_BINARY_SUB => {
            if let (Some(lhs_int), Some(rhs_int)) =
                (lhs.as_integer_strict(), rhs.as_integer_strict())
            {
                LuaValue::integer(lhs_int.wrapping_sub(rhs_int))
            } else {
                let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
                let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
                if lhs_num.is_nan() || rhs_num.is_nan() {
                    return 0;
                }
                LuaValue::float(lhs_num - rhs_num)
            }
        }
        NATIVE_NUMERIC_BINARY_MUL => {
            if let (Some(lhs_int), Some(rhs_int)) =
                (lhs.as_integer_strict(), rhs.as_integer_strict())
            {
                LuaValue::integer(lhs_int.wrapping_mul(rhs_int))
            } else {
                let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
                let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
                if lhs_num.is_nan() || rhs_num.is_nan() {
                    return 0;
                }
                LuaValue::float(lhs_num * rhs_num)
            }
        }
        NATIVE_NUMERIC_BINARY_DIV => {
            let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
            let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
            if lhs_num.is_nan() || rhs_num.is_nan() {
                return 0;
            }
            LuaValue::float(lhs_num / rhs_num)
        }
        NATIVE_NUMERIC_BINARY_IDIV => {
            if let (Some(lhs_int), Some(rhs_int)) =
                (lhs.as_integer_strict(), rhs.as_integer_strict())
            {
                if rhs_int == 0 {
                    return 0;
                }
                LuaValue::integer(lua_idiv(lhs_int, rhs_int))
            } else {
                let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
                let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
                if lhs_num.is_nan() || rhs_num.is_nan() || rhs_num == 0.0 {
                    return 0;
                }
                LuaValue::float((lhs_num / rhs_num).floor())
            }
        }
        NATIVE_NUMERIC_BINARY_MOD => {
            if let (Some(lhs_int), Some(rhs_int)) =
                (lhs.as_integer_strict(), rhs.as_integer_strict())
            {
                if rhs_int == 0 {
                    return 0;
                }
                LuaValue::integer(lua_imod(lhs_int, rhs_int))
            } else {
                let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
                let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
                if lhs_num.is_nan() || rhs_num.is_nan() || rhs_num == 0.0 {
                    return 0;
                }
                LuaValue::float(lua_fmod(lhs_num, rhs_num))
            }
        }
        NATIVE_NUMERIC_BINARY_POW => {
            let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
            let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
            if lhs_num.is_nan() || rhs_num.is_nan() {
                return 0;
            }
            LuaValue::float(luai_numpow(lhs_num, rhs_num))
        }
        _ => return 0,
    };

    unsafe {
        (*dst_ptr).value = result.value;
        (*dst_ptr).tt = result.tt;
    }
    1
}

pub(super) fn slot_addr(builder: &mut FunctionBuilder<'_>, base_ptr: Value, reg: u32) -> Value {
    builder
        .ins()
        .iadd_imm(base_ptr, i64::from(reg).saturating_mul(LUA_VALUE_SIZE))
}

fn const_addr(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    index: u32,
) -> Value {
    let idx_value = builder.ins().iconst(abi.pointer_ty, i64::from(index));
    let in_bounds = builder
        .ins()
        .icmp(IntCC::UnsignedLessThan, idx_value, abi.constants_len);
    let continue_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    builder
        .ins()
        .brif(in_bounds, continue_block, &[], fallback_block, &[]);
    builder.switch_to_block(continue_block);
    builder.seal_block(continue_block);
    builder.ins().iadd_imm(
        abi.constants_ptr,
        i64::from(index).saturating_mul(LUA_VALUE_SIZE),
    )
}

fn emit_copy_luavalue(builder: &mut FunctionBuilder<'_>, dst_ptr: Value, src_ptr: Value) {
    let mem = MemFlags::new();
    let raw_value = builder
        .ins()
        .load(types::I64, mem, src_ptr, LUA_VALUE_VALUE_OFFSET);
    let raw_tag = builder
        .ins()
        .load(types::I8, mem, src_ptr, LUA_VALUE_TT_OFFSET);
    builder
        .ins()
        .store(mem, raw_value, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    builder
        .ins()
        .store(mem, raw_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
}

fn emit_store_boolean_with_known_tag(
    builder: &mut FunctionBuilder<'_>,
    dst_ptr: Value,
    value: bool,
    dst_known_boolean: bool,
) {
    let mem = MemFlags::new();
    let zero = builder.ins().iconst(types::I64, 0);
    builder
        .ins()
        .store(mem, zero, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    if !dst_known_boolean {
        let bool_tag = builder.ins().iconst(
            types::I8,
            if value {
                LUA_VTRUE_TAG as i64
            } else {
                LUA_VFALSE_TAG as i64
            },
        );
        builder
            .ins()
            .store(mem, bool_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
    }
}

fn emit_store_float_with_known_tag(
    builder: &mut FunctionBuilder<'_>,
    dst_ptr: Value,
    value: f64,
    dst_known_float: bool,
) {
    let mem = MemFlags::new();
    let raw = builder.ins().iconst(types::I64, value.to_bits() as i64);
    builder
        .ins()
        .store(mem, raw, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    if !dst_known_float {
        let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
        builder
            .ins()
            .store(mem, float_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
    }
}

fn emit_store_float_value_with_known_tag(
    builder: &mut FunctionBuilder<'_>,
    dst_ptr: Value,
    value: Value,
    dst_known_float: bool,
) {
    let mem = MemFlags::new();
    let raw = builder.ins().bitcast(types::I64, mem, value);
    builder
        .ins()
        .store(mem, raw, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    if !dst_known_float {
        let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
        builder
            .ins()
            .store(mem, float_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
    }
}

fn emit_numeric_tagged_value_to_float(
    builder: &mut FunctionBuilder<'_>,
    tag: Value,
    value: Value,
) -> Value {
    let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
    let is_int = builder.ins().icmp(IntCC::Equal, tag, int_tag);
    let as_float_int = builder.ins().fcvt_from_sint(types::F64, value);
    let as_float_raw = builder.ins().bitcast(types::F64, MemFlags::new(), value);
    builder.ins().select(is_int, as_float_int, as_float_raw)
}

fn emit_numeric_operand_kind_and_payload(
    builder: &mut FunctionBuilder<'_>,
    operand: NumericOperand,
) -> (Value, Value) {
    match operand {
        NumericOperand::Reg(reg) => (
            builder
                .ins()
                .iconst(types::I32, i64::from(NATIVE_NUMERIC_OPERAND_REG)),
            builder.ins().iconst(types::I64, i64::from(reg)),
        ),
        NumericOperand::ImmI(imm) => (
            builder
                .ins()
                .iconst(types::I32, i64::from(NATIVE_NUMERIC_OPERAND_IMM_I)),
            builder.ins().iconst(types::I64, i64::from(imm)),
        ),
        NumericOperand::Const(index) => (
            builder
                .ins()
                .iconst(types::I32, i64::from(NATIVE_NUMERIC_OPERAND_CONST)),
            builder.ins().iconst(types::I64, i64::from(index)),
        ),
    }
}

fn emit_numeric_binary_helper_opcode(
    builder: &mut FunctionBuilder<'_>,
    op: NumericBinaryOp,
) -> Option<Value> {
    let opcode = match op {
        NumericBinaryOp::Add => NATIVE_NUMERIC_BINARY_ADD,
        NumericBinaryOp::Sub => NATIVE_NUMERIC_BINARY_SUB,
        NumericBinaryOp::Mul => NATIVE_NUMERIC_BINARY_MUL,
        NumericBinaryOp::Div => NATIVE_NUMERIC_BINARY_DIV,
        NumericBinaryOp::IDiv => NATIVE_NUMERIC_BINARY_IDIV,
        NumericBinaryOp::Mod => NATIVE_NUMERIC_BINARY_MOD,
        NumericBinaryOp::Pow => NATIVE_NUMERIC_BINARY_POW,
        _ => return None,
    };
    Some(builder.ins().iconst(types::I32, i64::from(opcode)))
}

pub(super) fn emit_integer_guard(
    builder: &mut FunctionBuilder<'_>,
    slot_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
) {
    emit_exact_tag_guard(
        builder,
        slot_ptr,
        LUA_VNUMINT,
        hits_var,
        current_hits,
        bail_block,
    );
}

pub(super) fn emit_exact_tag_guard(
    builder: &mut FunctionBuilder<'_>,
    slot_ptr: Value,
    expected_tag: u8,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
) {
    let mem = MemFlags::new();
    let tt = builder
        .ins()
        .load(types::I8, mem, slot_ptr, LUA_VALUE_TT_OFFSET);
    let tag_matches = builder
        .ins()
        .icmp_imm(IntCC::Equal, tt, i64::from(expected_tag));
    let next_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    builder
        .ins()
        .brif(tag_matches, next_block, &[], bail_block, &[]);
    builder.switch_to_block(next_block);
    builder.seal_block(next_block);
}

pub(super) fn emit_native_terminal_result(
    builder: &mut FunctionBuilder<'_>,
    block: Block,
    result_ptr: Value,
    hits_var: Variable,
    status: NativeTraceStatus,
    exit_pc: Option<u32>,
    exit_index: Option<u16>,
) {
    builder.switch_to_block(block);
    let hits = builder.use_var(hits_var);
    emit_store_native_result(
        builder,
        result_ptr,
        status,
        hits,
        exit_pc.unwrap_or(0),
        exit_index.unwrap_or(0),
    );
    builder.ins().return_(&[]);
    builder.seal_block(block);
}

pub(super) fn emit_store_native_result(
    builder: &mut FunctionBuilder<'_>,
    result_ptr: Value,
    status: NativeTraceStatus,
    hits: Value,
    exit_pc: u32,
    exit_index: u16,
) {
    emit_store_native_result_extended(
        builder,
        result_ptr,
        status,
        hits,
        exit_pc,
        0,
        0,
        u32::from(exit_index),
    );
}

fn emit_store_native_result_extended(
    builder: &mut FunctionBuilder<'_>,
    result_ptr: Value,
    status: NativeTraceStatus,
    hits: Value,
    exit_pc: u32,
    start_reg: u32,
    result_count: u32,
    exit_index: u32,
) {
    let mem = MemFlags::new();
    let status_value = builder.ins().iconst(types::I32, status as i64);
    let hits_value = builder.ins().ireduce(types::I32, hits);
    let exit_pc_value = builder.ins().iconst(types::I32, i64::from(exit_pc));
    let start_reg_value = builder.ins().iconst(types::I32, i64::from(start_reg));
    let result_count_value = builder.ins().iconst(types::I32, i64::from(result_count));
    let exit_index_value = builder.ins().iconst(types::I32, i64::from(exit_index));
    builder.ins().store(
        mem,
        status_value,
        result_ptr,
        NATIVE_TRACE_RESULT_STATUS_OFFSET,
    );
    builder
        .ins()
        .store(mem, hits_value, result_ptr, NATIVE_TRACE_RESULT_HITS_OFFSET);
    builder.ins().store(
        mem,
        exit_pc_value,
        result_ptr,
        NATIVE_TRACE_RESULT_EXIT_PC_OFFSET,
    );
    builder.ins().store(
        mem,
        start_reg_value,
        result_ptr,
        NATIVE_TRACE_RESULT_START_REG_OFFSET,
    );
    builder.ins().store(
        mem,
        result_count_value,
        result_ptr,
        NATIVE_TRACE_RESULT_RESULT_COUNT_OFFSET,
    );
    builder.ins().store(
        mem,
        exit_index_value,
        result_ptr,
        NATIVE_TRACE_RESULT_EXIT_INDEX_OFFSET,
    );
}

pub(super) fn emit_native_return_result(
    builder: &mut FunctionBuilder<'_>,
    result_ptr: Value,
    start_reg: u32,
    result_count: u32,
) {
    let hits = builder.ins().iconst(types::I64, 1);
    emit_store_native_result_extended(
        builder,
        result_ptr,
        NativeTraceStatus::Returned,
        hits,
        0,
        start_reg,
        result_count,
        0,
    );
    builder.ins().return_(&[]);
}

pub(super) fn emit_linear_int_counted_loop_backedge(
    builder: &mut FunctionBuilder<'_>,
    hits_var: Variable,
    next_hits: Value,
    carried_remaining: Value,
    carried_index: Value,
    hoisted_step_value: Option<Value>,
    loop_block: Block,
    loop_exit_block: Block,
) {
    let has_more = builder
        .ins()
        .icmp_imm(IntCC::UnsignedGreaterThan, carried_remaining, 0);
    let continue_block = builder.create_block();
    builder.def_var(hits_var, next_hits);
    builder
        .ins()
        .brif(has_more, continue_block, &[], loop_exit_block, &[]);

    builder.switch_to_block(continue_block);
    let step_val =
        hoisted_step_value.expect("linear-int for-loop invariant path requires hoisted step");
    let updated_remaining = builder.ins().iadd_imm(carried_remaining, -1);
    let updated_index = builder.ins().iadd(carried_index, step_val);
    builder.ins().jump(
        loop_block,
        &[
            cranelift::codegen::ir::BlockArg::Value(updated_remaining),
            cranelift::codegen::ir::BlockArg::Value(updated_index),
        ],
    );
    builder.seal_block(continue_block);
}

pub(super) fn emit_numeric_counted_loop_backedge_with_carried_integer(
    builder: &mut FunctionBuilder<'_>,
    hits_var: Variable,
    next_hits: Value,
    carried_remaining: Value,
    carried_index: Value,
    hoisted_step_value: Option<Value>,
    carried_integer: Value,
    loop_block: Block,
    loop_exit_block: Block,
) {
    let has_more = builder
        .ins()
        .icmp_imm(IntCC::UnsignedGreaterThan, carried_remaining, 0);
    let continue_block = builder.create_block();
    builder.def_var(hits_var, next_hits);
    builder
        .ins()
        .brif(has_more, continue_block, &[], loop_exit_block, &[]);

    builder.switch_to_block(continue_block);
    let step_val = hoisted_step_value.expect("numeric invariant path requires hoisted step");
    let updated_remaining = builder.ins().iadd_imm(carried_remaining, -1);
    let updated_index = builder.ins().iadd(carried_index, step_val);
    builder.ins().jump(
        loop_block,
        &[
            cranelift::codegen::ir::BlockArg::Value(updated_remaining),
            cranelift::codegen::ir::BlockArg::Value(updated_index),
            cranelift::codegen::ir::BlockArg::Value(carried_integer),
        ],
    );
    builder.seal_block(continue_block);
}

pub(super) fn emit_numeric_counted_loop_backedge_with_carried_float(
    builder: &mut FunctionBuilder<'_>,
    hits_var: Variable,
    next_hits: Value,
    carried_remaining: Value,
    carried_index: Value,
    hoisted_step_value: Option<Value>,
    carried_float_raw: Value,
    loop_block: Block,
    loop_exit_block: Block,
) {
    let has_more = builder
        .ins()
        .icmp_imm(IntCC::UnsignedGreaterThan, carried_remaining, 0);
    let continue_block = builder.create_block();
    builder.def_var(hits_var, next_hits);
    builder
        .ins()
        .brif(has_more, continue_block, &[], loop_exit_block, &[]);

    builder.switch_to_block(continue_block);
    let step_val = hoisted_step_value.expect("numeric invariant path requires hoisted step");
    let updated_remaining = builder.ins().iadd_imm(carried_remaining, -1);
    let updated_index = builder.ins().iadd(carried_index, step_val);
    builder.ins().jump(
        loop_block,
        &[
            cranelift::codegen::ir::BlockArg::Value(updated_remaining),
            cranelift::codegen::ir::BlockArg::Value(updated_index),
            cranelift::codegen::ir::BlockArg::Value(carried_float_raw),
        ],
    );
    builder.seal_block(continue_block);
}

pub(super) fn emit_linear_int_materialize_loop_state(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    loop_reg: u32,
    carried_remaining_var: Variable,
    carried_index_var: Variable,
    source_block: Block,
    target_block: Block,
) {
    builder.switch_to_block(source_block);
    let loop_ptr = slot_addr(builder, base_ptr, loop_reg);
    let index_ptr = slot_addr(builder, base_ptr, loop_reg.saturating_add(2));
    let carried_remaining = builder.use_var(carried_remaining_var);
    let carried_index = builder.use_var(carried_index_var);
    emit_store_integer_with_known_tag(builder, loop_ptr, carried_remaining, true);
    emit_store_integer_with_known_tag(builder, index_ptr, carried_index, true);
    builder.ins().jump(target_block, &[]);
    builder.seal_block(source_block);
}

fn emit_store_float_raw_with_known_tag(
    builder: &mut FunctionBuilder<'_>,
    dst_ptr: Value,
    raw: Value,
    dst_known_float: bool,
) {
    let mem = MemFlags::new();
    builder
        .ins()
        .store(mem, raw, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    if !dst_known_float {
        let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
        builder
            .ins()
            .store(mem, float_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
    }
}

pub(super) fn emit_materialize_numeric_loop_state(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    loop_state: Option<(u32, Variable, Variable)>,
    carried_integer: Option<(u32, Variable)>,
    carried_float: Option<(u32, Variable)>,
    source_block: Block,
    target_block: Block,
) {
    builder.switch_to_block(source_block);
    if let Some((loop_reg, carried_remaining_var, carried_index_var)) = loop_state {
        let loop_ptr = slot_addr(builder, base_ptr, loop_reg);
        let index_ptr = slot_addr(builder, base_ptr, loop_reg.saturating_add(2));
        let carried_remaining = builder.use_var(carried_remaining_var);
        let carried_index = builder.use_var(carried_index_var);
        emit_store_integer_with_known_tag(builder, loop_ptr, carried_remaining, true);
        emit_store_integer_with_known_tag(builder, index_ptr, carried_index, true);
    }
    if let Some((reg, carried_integer_var)) = carried_integer {
        let ptr = slot_addr(builder, base_ptr, reg);
        let value = builder.use_var(carried_integer_var);
        emit_store_integer_with_known_tag(builder, ptr, value, true);
    }
    if let Some((reg, carried_float_raw_var)) = carried_float {
        let ptr = slot_addr(builder, base_ptr, reg);
        let raw = builder.use_var(carried_float_raw_var);
        emit_store_float_raw_with_known_tag(builder, ptr, raw, true);
    }
    builder.ins().jump(target_block, &[]);
    builder.seal_block(source_block);
}

pub(super) fn emit_counted_loop_backedge(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    next_hits: Value,
    loop_reg: u32,
    hoisted_step_value: Option<Value>,
    loop_state_is_invariant: bool,
    loop_block: Block,
    loop_exit_block: Block,
    fallback_block: Block,
) {
    let loop_ptr = slot_addr(builder, base_ptr, loop_reg);
    let step_ptr = slot_addr(builder, base_ptr, loop_reg.saturating_add(1));
    let index_ptr = slot_addr(builder, base_ptr, loop_reg.saturating_add(2));
    if !loop_state_is_invariant {
        emit_integer_guard(builder, loop_ptr, hits_var, current_hits, fallback_block);
        emit_integer_guard(builder, step_ptr, hits_var, current_hits, fallback_block);
        emit_integer_guard(builder, index_ptr, hits_var, current_hits, fallback_block);
    }

    let mem = MemFlags::new();
    let remaining = builder
        .ins()
        .load(types::I64, mem, loop_ptr, LUA_VALUE_VALUE_OFFSET);
    let has_more = builder
        .ins()
        .icmp_imm(IntCC::UnsignedGreaterThan, remaining, 0);
    let continue_block = builder.create_block();
    builder.def_var(hits_var, next_hits);
    builder
        .ins()
        .brif(has_more, continue_block, &[], loop_exit_block, &[]);

    builder.switch_to_block(continue_block);
    let step_val = hoisted_step_value.unwrap_or_else(|| {
        builder
            .ins()
            .load(types::I64, mem, step_ptr, LUA_VALUE_VALUE_OFFSET)
    });
    let index_val = builder
        .ins()
        .load(types::I64, mem, index_ptr, LUA_VALUE_VALUE_OFFSET);
    let updated_remaining = builder.ins().iadd_imm(remaining, -1);
    let updated_index = builder.ins().iadd(index_val, step_val);
    builder
        .ins()
        .store(mem, updated_remaining, loop_ptr, LUA_VALUE_VALUE_OFFSET);
    builder
        .ins()
        .store(mem, updated_index, index_ptr, LUA_VALUE_VALUE_OFFSET);
    if !loop_state_is_invariant {
        let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
        builder
            .ins()
            .store(mem, int_tag, loop_ptr, LUA_VALUE_TT_OFFSET);
        builder
            .ins()
            .store(mem, int_tag, index_ptr, LUA_VALUE_TT_OFFSET);
    }
    builder.ins().jump(loop_block, &[]);
    builder.seal_block(continue_block);
}

fn linear_int_step_writes_reg(step: LinearIntStep, reg: u32) -> bool {
    match step {
        LinearIntStep::Move { dst, .. }
        | LinearIntStep::LoadI { dst, .. }
        | LinearIntStep::BNot { dst, .. }
        | LinearIntStep::Add { dst, .. }
        | LinearIntStep::AddI { dst, .. }
        | LinearIntStep::Sub { dst, .. }
        | LinearIntStep::SubI { dst, .. }
        | LinearIntStep::Mul { dst, .. }
        | LinearIntStep::MulI { dst, .. }
        | LinearIntStep::IDiv { dst, .. }
        | LinearIntStep::IDivI { dst, .. }
        | LinearIntStep::Mod { dst, .. }
        | LinearIntStep::ModI { dst, .. }
        | LinearIntStep::BAnd { dst, .. }
        | LinearIntStep::BAndI { dst, .. }
        | LinearIntStep::BOr { dst, .. }
        | LinearIntStep::BOrI { dst, .. }
        | LinearIntStep::BXor { dst, .. }
        | LinearIntStep::BXorI { dst, .. }
        | LinearIntStep::Shl { dst, .. }
        | LinearIntStep::ShlI { dst, .. }
        | LinearIntStep::Shr { dst, .. }
        | LinearIntStep::ShrI { dst, .. } => dst == reg,
    }
}

pub(super) fn linear_int_loop_state_is_invariant(loop_reg: u32, steps: &[LinearIntStep]) -> bool {
    let step_reg = loop_reg.saturating_add(1);
    let index_reg = loop_reg.saturating_add(2);
    !steps.iter().copied().any(|step| {
        linear_int_step_writes_reg(step, loop_reg)
            || linear_int_step_writes_reg(step, step_reg)
            || linear_int_step_writes_reg(step, index_reg)
    })
}

fn numeric_step_writes_reg(step: NumericStep, reg: u32) -> bool {
    match step {
        NumericStep::Move { dst, .. }
        | NumericStep::LoadBool { dst, .. }
        | NumericStep::LoadI { dst, .. }
        | NumericStep::LoadF { dst, .. }
        | NumericStep::Len { dst, .. }
        | NumericStep::GetUpval { dst, .. }
        | NumericStep::GetTabUpField { dst, .. }
        | NumericStep::GetTableInt { dst, .. }
        | NumericStep::GetTableField { dst, .. }
        | NumericStep::Binary { dst, .. } => dst == reg,
        NumericStep::SetUpval { .. }
        | NumericStep::SetTabUpField { .. }
        | NumericStep::SetTableInt { .. }
        | NumericStep::SetTableField { .. } => false,
    }
}

fn numeric_operand_reads_reg(operand: NumericOperand, reg: u32) -> bool {
    matches!(operand, NumericOperand::Reg(operand_reg) if operand_reg == reg)
}

fn numeric_step_reads_reg(step: NumericStep, reg: u32) -> bool {
    match step {
        NumericStep::Move { src, .. } => src == reg,
        NumericStep::LoadBool { .. } | NumericStep::LoadI { .. } | NumericStep::LoadF { .. } => {
            false
        }
        NumericStep::Len { src, .. } => src == reg,
        NumericStep::GetUpval { .. } | NumericStep::GetTabUpField { .. } => false,
        NumericStep::SetUpval { src, .. } => src == reg,
        NumericStep::SetTabUpField { value, .. } => value == reg,
        NumericStep::GetTableInt { table, index, .. } => table == reg || index == reg,
        NumericStep::GetTableField { table, .. } => table == reg,
        NumericStep::SetTableInt {
            table,
            index,
            value,
        } => table == reg || index == reg || value == reg,
        NumericStep::SetTableField { table, value, .. } => table == reg || value == reg,
        NumericStep::Binary { lhs, rhs, .. } => {
            numeric_operand_reads_reg(lhs, reg) || numeric_operand_reads_reg(rhs, reg)
        }
    }
}

pub(super) fn numeric_loop_state_is_invariant(loop_reg: u32, steps: &[NumericStep]) -> bool {
    let step_reg = loop_reg.saturating_add(1);
    let index_reg = loop_reg.saturating_add(2);
    !steps.iter().copied().any(|step| {
        numeric_step_reads_reg(step, loop_reg)
            || numeric_step_reads_reg(step, step_reg)
            || numeric_step_reads_reg(step, index_reg)
            || numeric_step_writes_reg(step, loop_reg)
            || numeric_step_writes_reg(step, step_reg)
            || numeric_step_writes_reg(step, index_reg)
    })
}

pub(super) fn numeric_steps_preserve_reg(steps: &[NumericStep], reg: u32) -> bool {
    !steps
        .iter()
        .copied()
        .any(|step| numeric_step_writes_reg(step, reg))
}

fn numeric_cond_reads_reg(cond: NumericIfElseCond, reg: u32) -> bool {
    match cond {
        NumericIfElseCond::RegCompare { lhs, rhs, .. } => lhs == reg || rhs == reg,
        NumericIfElseCond::Truthy { reg: cond_reg } => cond_reg == reg,
    }
}

pub(super) fn numeric_guard_touches_reg(guard: NumericJmpLoopGuard, reg: u32) -> bool {
    let (cond, continue_preset, exit_preset) = match guard {
        NumericJmpLoopGuard::Head {
            cond,
            continue_preset,
            exit_preset,
            ..
        }
        | NumericJmpLoopGuard::Tail {
            cond,
            continue_preset,
            exit_preset,
            ..
        } => (cond, continue_preset, exit_preset),
    };

    numeric_cond_reads_reg(cond, reg)
        || continue_preset.is_some_and(|step| {
            numeric_step_reads_reg(step, reg) || numeric_step_writes_reg(step, reg)
        })
        || exit_preset.is_some_and(|step| {
            numeric_step_reads_reg(step, reg) || numeric_step_writes_reg(step, reg)
        })
}

pub(super) fn numeric_guard_block_touches_reg(block: &NumericJmpLoopGuardBlock, reg: u32) -> bool {
    block
        .pre_steps
        .iter()
        .copied()
        .any(|step| numeric_step_reads_reg(step, reg) || numeric_step_writes_reg(step, reg))
        || numeric_guard_touches_reg(block.guard, reg)
}

pub(super) fn entry_reg_has_explicit_float_hint(lowered_trace: &LoweredTrace, reg: u32) -> bool {
    lowered_trace.entry_register_value_kind(reg) == Some(TraceValueKind::Float)
        || lowered_trace.entry_stable_register_value_kind(reg) == Some(TraceValueKind::Float)
}

pub(super) fn numeric_guard_writes_reg_outside_condition(guard: NumericJmpLoopGuard, reg: u32) -> bool {
    let (_, continue_preset, exit_preset) = match guard {
        NumericJmpLoopGuard::Head {
            cond: _,
            continue_preset,
            exit_preset,
            ..
        }
        | NumericJmpLoopGuard::Tail {
            cond: _,
            continue_preset,
            exit_preset,
            ..
        } => ((), continue_preset, exit_preset),
    };

    continue_preset.is_some_and(|step| numeric_step_writes_reg(step, reg))
        || exit_preset.is_some_and(|step| numeric_step_writes_reg(step, reg))
}

pub(super) fn numeric_guard_block_writes_reg_outside_condition(
    block: &NumericJmpLoopGuardBlock,
    reg: u32,
) -> bool {
    block
        .pre_steps
        .iter()
        .copied()
        .any(|step| numeric_step_writes_reg(step, reg))
        || numeric_guard_writes_reg_outside_condition(block.guard, reg)
}

fn linear_int_reg_is_known_integer(known_integer_regs: &[u32], reg: u32) -> bool {
    known_integer_regs.contains(&reg)
}

fn mark_linear_int_reg_known_integer(known_integer_regs: &mut Vec<u32>, reg: u32) {
    if !linear_int_reg_is_known_integer(known_integer_regs, reg) {
        known_integer_regs.push(reg);
    }
}

fn numeric_reg_value_kind(
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    reg: u32,
) -> TraceValueKind {
    known_value_kinds
        .iter()
        .rev()
        .find_map(|hint| (hint.reg == reg).then_some(hint.kind))
        .unwrap_or(TraceValueKind::Unknown)
}

pub(super) fn set_numeric_reg_value_kind(
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
    reg: u32,
    kind: TraceValueKind,
) {
    if let Some(existing) = known_value_kinds
        .iter_mut()
        .rev()
        .find(|hint| hint.reg == reg)
    {
        existing.kind = kind;
    } else {
        known_value_kinds.push(crate::lua_vm::jit::lowering::RegisterValueHint { reg, kind });
    }
}

fn trace_value_kind_tag(kind: TraceValueKind) -> Option<u8> {
    match kind {
        TraceValueKind::Integer => Some(LUA_VNUMINT),
        TraceValueKind::Float => Some(LUA_VNUMFLT),
        TraceValueKind::Boolean => Some(LUA_VTRUE_TAG),
        _ => None,
    }
}

fn emit_store_integer_with_known_tag(
    builder: &mut FunctionBuilder<'_>,
    dst_ptr: Value,
    value: Value,
    dst_known_integer: bool,
) {
    let mem = MemFlags::new();
    builder
        .ins()
        .store(mem, value, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    if !dst_known_integer {
        let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
        builder
            .ins()
            .store(mem, int_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
    }
}

fn emit_known_linear_int_reg_value(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &[u32],
    current_integer_values: &[(u32, Value)],
    loop_carried_values: &[(u32, Value)],
    reg: u32,
) -> Value {
    if let Some(value) = current_integer_values
        .iter()
        .rev()
        .find_map(|(current_reg, value)| (*current_reg == reg).then_some(*value))
    {
        return value;
    }
    if let Some(value) = loop_carried_values
        .iter()
        .find_map(|(carried_reg, value)| (*carried_reg == reg).then_some(*value))
    {
        return value;
    }
    let mem = MemFlags::new();
    let reg_ptr = slot_addr(builder, base_ptr, reg);
    if !linear_int_reg_is_known_integer(known_integer_regs, reg) {
        emit_integer_guard(builder, reg_ptr, hits_var, current_hits, bail_block);
    }
    builder
        .ins()
        .load(types::I64, mem, reg_ptr, LUA_VALUE_VALUE_OFFSET)
}

fn set_current_linear_int_reg_value(
    current_integer_values: &mut Vec<(u32, Value)>,
    reg: u32,
    value: Value,
) {
    if let Some((_, current_value)) = current_integer_values
        .iter_mut()
        .rev()
        .find(|(current_reg, _)| *current_reg == reg)
    {
        *current_value = value;
    } else {
        current_integer_values.push((reg, value));
    }
}

pub(super) fn emit_linear_int_guard_condition(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    known_integer_regs: &[u32],
    current_integer_values: &[(u32, Value)],
    loop_carried_values: &[(u32, Value)],
    guard: LinearIntLoopGuard,
) -> Value {
    let (op, lhs_val, rhs_val) = match guard {
        LinearIntLoopGuard::HeadRegReg { op, lhs, rhs, .. }
        | LinearIntLoopGuard::TailRegReg { op, lhs, rhs, .. } => {
            let lhs_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                fallback_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                lhs,
            );
            let rhs_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                fallback_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                rhs,
            );
            (op, lhs_val, rhs_val)
        }
        LinearIntLoopGuard::HeadRegImm { op, reg, imm, .. }
        | LinearIntLoopGuard::TailRegImm { op, reg, imm, .. } => {
            let lhs_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                fallback_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                reg,
            );
            let rhs_val = builder.ins().iconst(types::I64, i64::from(imm));
            (op, lhs_val, rhs_val)
        }
    };

    emit_linear_compare(builder, lhs_val, rhs_val, op)
}

pub(super) fn emit_linear_int_step(
    builder: &mut FunctionBuilder<'_>,
    native_helpers: &NativeHelpers,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    step: LinearIntStep,
    known_integer_regs: &mut Vec<u32>,
    current_integer_values: &mut Vec<(u32, Value)>,
    loop_carried_values: &[(u32, Value)],
) {
    match step {
        LinearIntStep::Move { dst, src } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let src_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                src,
            );
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, src_val, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
            set_current_linear_int_reg_value(current_integer_values, dst, src_val);
        }
        LinearIntStep::LoadI { dst, imm } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let dst_val = builder.ins().iconst(types::I64, i64::from(imm));
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, dst_val, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
            set_current_linear_int_reg_value(current_integer_values, dst, dst_val);
        }
        LinearIntStep::BNot { dst, src } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let src_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                src,
            );
            let result = builder.ins().bnot(src_val);
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
            set_current_linear_int_reg_value(current_integer_values, dst, result);
        }
        LinearIntStep::Add { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().iadd(l, r),
            );
        }
        LinearIntStep::AddI { dst, src, imm } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let src_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                src,
            );
            let imm_val = builder.ins().iconst(types::I64, i64::from(imm));
            let result = builder.ins().iadd(src_val, imm_val);
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
            set_current_linear_int_reg_value(current_integer_values, dst, result);
        }
        LinearIntStep::Sub { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().isub(l, r),
            );
        }
        LinearIntStep::SubI { dst, src, imm } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let src_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                src,
            );
            let imm_val = builder.ins().iconst(types::I64, i64::from(imm));
            let result = builder.ins().isub(src_val, imm_val);
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
            set_current_linear_int_reg_value(current_integer_values, dst, result);
        }
        LinearIntStep::Mul { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().imul(l, r),
            );
        }
        LinearIntStep::MulI { dst, src, imm } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let src_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                src,
            );
            let imm_val = builder.ins().iconst(types::I64, i64::from(imm));
            let result = builder.ins().imul(src_val, imm_val);
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
            set_current_linear_int_reg_value(current_integer_values, dst, result);
        }
        LinearIntStep::IDiv { dst, lhs, rhs } => {
            emit_linear_int_div_mod_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                false,
            );
        }
        LinearIntStep::IDivI { dst, src, imm } => {
            emit_linear_int_div_mod_imm(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                src,
                imm,
                false,
            );
        }
        LinearIntStep::Mod { dst, lhs, rhs } => {
            emit_linear_int_div_mod_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                true,
            );
        }
        LinearIntStep::ModI { dst, src, imm } => {
            emit_linear_int_div_mod_imm(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                src,
                imm,
                true,
            );
        }
        LinearIntStep::BAnd { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().band(l, r),
            );
        }
        LinearIntStep::BAndI { dst, src, imm } => {
            emit_linear_int_imm_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                src,
                imm,
                |b, value, rhs| b.ins().band(value, rhs),
            );
        }
        LinearIntStep::BOr { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().bor(l, r),
            );
        }
        LinearIntStep::BOrI { dst, src, imm } => {
            emit_linear_int_imm_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                src,
                imm,
                |b, value, rhs| b.ins().bor(value, rhs),
            );
        }
        LinearIntStep::BXor { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().bxor(l, r),
            );
        }
        LinearIntStep::BXorI { dst, src, imm } => {
            emit_linear_int_imm_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                src,
                imm,
                |b, value, rhs| b.ins().bxor(value, rhs),
            );
        }
        LinearIntStep::Shl { dst, lhs, rhs } => {
            emit_linear_int_shift_op(
                builder,
                native_helpers,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                true,
            );
        }
        LinearIntStep::ShlI { dst, imm, src } => {
            emit_linear_int_shift_imm_lhs(
                builder,
                native_helpers,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                imm,
                src,
            );
        }
        LinearIntStep::Shr { dst, lhs, rhs } => {
            emit_linear_int_shift_op(
                builder,
                native_helpers,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                false,
            );
        }
        LinearIntStep::ShrI { dst, src, imm } => {
            emit_linear_int_imm_shift_rhs(
                builder,
                native_helpers,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                current_integer_values,
                loop_carried_values,
                dst,
                src,
                imm,
            );
        }
    }
}

fn emit_linear_int_imm_op<F>(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    current_integer_values: &mut Vec<(u32, Value)>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    src: u32,
    imm: i32,
    op: F,
) where
    F: Fn(&mut FunctionBuilder<'_>, Value, Value) -> Value,
{
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let src_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        current_integer_values,
        loop_carried_values,
        src,
    );
    let imm_val = builder.ins().iconst(types::I64, i64::from(imm));
    let result = op(builder, src_val, imm_val);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
    set_current_linear_int_reg_value(current_integer_values, dst, result);
}

fn emit_linear_int_div_mod_op(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    current_integer_values: &mut Vec<(u32, Value)>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    lhs: u32,
    rhs: u32,
    modulo: bool,
) {
    let lhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        current_integer_values,
        loop_carried_values,
        lhs,
    );
    let rhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        current_integer_values,
        loop_carried_values,
        rhs,
    );
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    if modulo {
        emit_integer_mod(
            builder,
            hits_var,
            current_hits,
            bail_block,
            dst_ptr,
            lhs_val,
            rhs_val,
            dst_known_integer,
        );
    } else {
        emit_integer_idiv(
            builder,
            hits_var,
            current_hits,
            bail_block,
            dst_ptr,
            lhs_val,
            rhs_val,
            dst_known_integer,
        );
    }
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
    current_integer_values.retain(|(reg, _)| *reg != dst);
}

fn emit_linear_int_div_mod_imm(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    current_integer_values: &mut Vec<(u32, Value)>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    src: u32,
    imm: i32,
    modulo: bool,
) {
    let lhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        current_integer_values,
        loop_carried_values,
        src,
    );
    let rhs_val = builder.ins().iconst(types::I64, i64::from(imm));
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    if modulo {
        emit_integer_mod(
            builder,
            hits_var,
            current_hits,
            bail_block,
            dst_ptr,
            lhs_val,
            rhs_val,
            dst_known_integer,
        );
    } else {
        emit_integer_idiv(
            builder,
            hits_var,
            current_hits,
            bail_block,
            dst_ptr,
            lhs_val,
            rhs_val,
            dst_known_integer,
        );
    }
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
    current_integer_values.retain(|(reg, _)| *reg != dst);
}

fn emit_linear_int_shift_op(
    builder: &mut FunctionBuilder<'_>,
    native_helpers: &NativeHelpers,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    current_integer_values: &mut Vec<(u32, Value)>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    lhs: u32,
    rhs: u32,
    shift_left: bool,
) {
    let lhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        current_integer_values,
        loop_carried_values,
        lhs,
    );
    let rhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        current_integer_values,
        loop_carried_values,
        rhs,
    );
    let call = if shift_left {
        builder
            .ins()
            .call(native_helpers.shift_left, &[lhs_val, rhs_val])
    } else {
        builder
            .ins()
            .call(native_helpers.shift_right, &[lhs_val, rhs_val])
    };
    let result = builder.inst_results(call)[0];
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
    set_current_linear_int_reg_value(current_integer_values, dst, result);
}

fn emit_linear_int_shift_imm_lhs(
    builder: &mut FunctionBuilder<'_>,
    native_helpers: &NativeHelpers,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    current_integer_values: &mut Vec<(u32, Value)>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    imm: i32,
    src: u32,
) {
    let lhs_val = builder.ins().iconst(types::I64, i64::from(imm));
    let rhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        current_integer_values,
        loop_carried_values,
        src,
    );
    let call = builder
        .ins()
        .call(native_helpers.shift_left, &[lhs_val, rhs_val]);
    let result = builder.inst_results(call)[0];
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
    set_current_linear_int_reg_value(current_integer_values, dst, result);
}

fn emit_linear_int_imm_shift_rhs(
    builder: &mut FunctionBuilder<'_>,
    native_helpers: &NativeHelpers,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    current_integer_values: &mut Vec<(u32, Value)>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    src: u32,
    imm: i32,
) {
    let lhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        current_integer_values,
        loop_carried_values,
        src,
    );
    let rhs_val = builder.ins().iconst(types::I64, i64::from(imm));
    let call = builder
        .ins()
        .call(native_helpers.shift_right, &[lhs_val, rhs_val]);
    let result = builder.inst_results(call)[0];
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
    set_current_linear_int_reg_value(current_integer_values, dst, result);
}

fn emit_binary_int_op<F>(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    current_integer_values: &mut Vec<(u32, Value)>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    lhs: u32,
    rhs: u32,
    op: F,
) where
    F: Fn(&mut FunctionBuilder<'_>, Value, Value) -> Value,
{
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let lhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        current_integer_values,
        loop_carried_values,
        lhs,
    );
    let rhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        current_integer_values,
        loop_carried_values,
        rhs,
    );
    let result = op(builder, lhs_val, rhs_val);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
    set_current_linear_int_reg_value(current_integer_values, dst, result);
}

fn emit_helper_success_guard(
    builder: &mut FunctionBuilder<'_>,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    success: Value,
) {
    let continue_block = builder.create_block();
    let ok = builder.ins().icmp_imm(IntCC::NotEqual, success, 0);
    builder.def_var(hits_var, current_hits);
    builder
        .ins()
        .brif(ok, continue_block, &[], fallback_block, &[]);
    builder.switch_to_block(continue_block);
    builder.seal_block(continue_block);
}

pub(super) fn exact_float_self_update_step(
    steps: &[NumericStep],
    lowered_trace: &LoweredTrace,
) -> Option<CarriedFloatLoopStep> {
    let (dst, lhs, rhs, op) = match steps {
        [NumericStep::Binary { dst, lhs, rhs, op }] => (*dst, *lhs, *rhs, *op),
        [
            NumericStep::Move {
                dst: alias_dst,
                src: alias_src,
            },
            NumericStep::Binary { dst, lhs, rhs, op },
        ] if matches!(rhs, NumericOperand::Reg(reg) if *reg == *alias_dst)
            && *alias_dst != *dst
            && *alias_src != *dst =>
        {
            (*dst, *lhs, NumericOperand::Reg(*alias_src), *op)
        }
        _ => return None,
    };
    let NumericOperand::Reg(lhs_reg) = lhs else {
        return None;
    };
    if dst != lhs_reg
        || !matches!(
            op,
            NumericBinaryOp::Add
                | NumericBinaryOp::Sub
                | NumericBinaryOp::Mul
                | NumericBinaryOp::Div
        )
    {
        return None;
    }
    let entry_kind = lowered_trace.entry_register_value_kind(dst);
    let stable_kind = lowered_trace.entry_stable_register_value_kind(dst);
    if matches!(
        entry_kind,
        Some(
            TraceValueKind::Integer
                | TraceValueKind::Table
                | TraceValueKind::Boolean
                | TraceValueKind::Closure
        )
    ) || matches!(
        stable_kind,
        Some(
            TraceValueKind::Integer
                | TraceValueKind::Table
                | TraceValueKind::Boolean
                | TraceValueKind::Closure
        )
    ) {
        return None;
    }
    let rhs = match rhs {
        NumericOperand::ImmI(imm) => CarriedFloatRhs::Imm(f64::from(imm)),
        NumericOperand::Const(index) => CarriedFloatRhs::Imm(
            lowered_trace
                .float_constant(index)
                .or_else(|| lowered_trace.integer_constant(index).map(f64::from))?,
        ),
        NumericOperand::Reg(rhs_reg) => {
            if rhs_reg == dst {
                return None;
            }
            match lowered_trace.entry_stable_register_value_kind(rhs_reg) {
                Some(TraceValueKind::Float) => CarriedFloatRhs::StableReg {
                    reg: rhs_reg,
                    kind: TraceValueKind::Float,
                },
                Some(TraceValueKind::Integer) => CarriedFloatRhs::StableReg {
                    reg: rhs_reg,
                    kind: TraceValueKind::Integer,
                },
                _ => return None,
            }
        }
    };
    Some(CarriedFloatLoopStep { reg: dst, op, rhs })
}

pub(super) fn carried_float_loop_step_from_value_flow(
    value_flow: NumericSelfUpdateValueFlow,
    lowered_trace: &LoweredTrace,
) -> Option<CarriedFloatLoopStep> {
    if !matches!(value_flow.kind, NumericSelfUpdateValueKind::Float) {
        return None;
    }

    let rhs = match value_flow.rhs {
        NumericValueFlowRhs::ImmI(imm) => CarriedFloatRhs::Imm(f64::from(imm)),
        NumericValueFlowRhs::Const(index) => CarriedFloatRhs::Imm(
            lowered_trace
                .float_constant(index)
                .or_else(|| lowered_trace.integer_constant(index).map(f64::from))?,
        ),
        NumericValueFlowRhs::StableReg { reg, kind } => match kind {
            TraceValueKind::Integer | TraceValueKind::Float => {
                CarriedFloatRhs::StableReg { reg, kind }
            }
            TraceValueKind::Unknown
            | TraceValueKind::Numeric
            | TraceValueKind::Boolean
            | TraceValueKind::Table
            | TraceValueKind::Closure => return None,
        },
    };

    Some(CarriedFloatLoopStep {
        reg: value_flow.reg,
        op: value_flow.op,
        rhs,
    })
}

pub(super) fn carried_integer_loop_step_from_value_flow(
    value_flow: NumericSelfUpdateValueFlow,
) -> Option<CarriedIntegerLoopStep> {
    if !matches!(value_flow.kind, NumericSelfUpdateValueKind::Integer) {
        return None;
    }

    let rhs = match value_flow.rhs {
        NumericValueFlowRhs::ImmI(imm) => CarriedIntegerRhs::Imm(i64::from(imm)),
        NumericValueFlowRhs::StableReg {
            reg,
            kind: TraceValueKind::Integer,
        } => CarriedIntegerRhs::StableReg { reg },
        NumericValueFlowRhs::Const(_) | NumericValueFlowRhs::StableReg { .. } => return None,
    };

    Some(CarriedIntegerLoopStep {
        reg: value_flow.reg,
        op: value_flow.op,
        rhs,
    })
}

pub(super) fn carried_float_rhs_stable_reg(step: CarriedFloatLoopStep) -> Option<u32> {
    match step.rhs {
        CarriedFloatRhs::StableReg { reg, .. } => Some(reg),
        CarriedFloatRhs::Imm(_) => None,
    }
}

pub(super) fn carried_integer_rhs_stable_reg(step: CarriedIntegerLoopStep) -> Option<u32> {
    match step.rhs {
        CarriedIntegerRhs::StableReg { reg } => Some(reg),
        CarriedIntegerRhs::Imm(_) => None,
    }
}

fn numeric_value_flow_rhs_matches_operand(
    rhs: NumericValueFlowRhs,
    operand: NumericOperand,
) -> bool {
    match (rhs, operand) {
        (NumericValueFlowRhs::ImmI(expected), NumericOperand::ImmI(actual)) => expected == actual,
        (NumericValueFlowRhs::Const(expected), NumericOperand::Const(actual)) => expected == actual,
        (NumericValueFlowRhs::StableReg { reg, .. }, NumericOperand::Reg(actual)) => reg == actual,
        _ => false,
    }
}

pub(super) fn integer_self_update_step_span(
    steps: &[NumericStep],
    value_flow: NumericSelfUpdateValueFlow,
) -> Option<(usize, usize)> {
    if !matches!(value_flow.kind, NumericSelfUpdateValueKind::Integer) {
        return None;
    }

    for index in 0..steps.len() {
        match steps[index] {
            NumericStep::Binary { dst, lhs, rhs, op }
                if dst == value_flow.reg
                    && matches!(lhs, NumericOperand::Reg(reg) if reg == value_flow.reg)
                    && op == value_flow.op
                    && numeric_value_flow_rhs_matches_operand(value_flow.rhs, rhs) =>
            {
                return Some((index, 1));
            }
            NumericStep::Move {
                dst: alias_dst,
                src: alias_src,
            } if index + 1 < steps.len() => {
                if let NumericStep::Binary { dst, lhs, rhs, op } = steps[index + 1] {
                    if dst == value_flow.reg
                        && matches!(lhs, NumericOperand::Reg(reg) if reg == value_flow.reg)
                        && matches!(rhs, NumericOperand::Reg(reg) if reg == alias_dst)
                        && op == value_flow.op
                        && matches!(
                            value_flow.rhs,
                            NumericValueFlowRhs::StableReg { reg, .. } if reg == alias_src
                        )
                    {
                        return Some((index, 2));
                    }
                }
            }
            _ => {}
        }
    }

    None
}

pub(super) fn float_self_update_step_span(
    steps: &[NumericStep],
    value_flow: NumericSelfUpdateValueFlow,
) -> Option<(usize, usize)> {
    if !matches!(value_flow.kind, NumericSelfUpdateValueKind::Float) {
        return None;
    }

    for index in 0..steps.len() {
        match steps[index] {
            NumericStep::Binary { dst, lhs, rhs, op }
                if dst == value_flow.reg
                    && matches!(lhs, NumericOperand::Reg(reg) if reg == value_flow.reg)
                    && op == value_flow.op
                    && numeric_value_flow_rhs_matches_operand(value_flow.rhs, rhs) =>
            {
                return Some((index, 1));
            }
            NumericStep::Move {
                dst: alias_dst,
                src: alias_src,
            } if index + 1 < steps.len() => {
                if let NumericStep::Binary { dst, lhs, rhs, op } = steps[index + 1] {
                    if dst == value_flow.reg
                        && matches!(lhs, NumericOperand::Reg(reg) if reg == value_flow.reg)
                        && matches!(rhs, NumericOperand::Reg(reg) if reg == alias_dst)
                        && op == value_flow.op
                        && matches!(
                            value_flow.rhs,
                            NumericValueFlowRhs::StableReg { reg, .. } if reg == alias_src
                        )
                    {
                        return Some((index, 2));
                    }
                }
            }
            _ => {}
        }
    }

    None
}

pub(super) fn emit_numeric_steps_with_carried_integer(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    steps: &[NumericStep],
    carried_integer_var: Variable,
    carried_step: CarriedIntegerLoopStep,
    carried_rhs: ResolvedCarriedIntegerRhs,
    span_start: usize,
    span_len: usize,
    stable_rhs: Option<HoistedNumericGuardValue>,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
    current_numeric_values: &mut CurrentNumericGuardValues,
) -> Option<()> {
    let pre_override = HoistedNumericGuardValues {
        first: Some(HoistedNumericGuardValue {
            reg: carried_step.reg,
            source: HoistedNumericGuardSource::Integer(builder.use_var(carried_integer_var)),
        }),
        second: stable_rhs,
    };

    for step in &steps[..span_start] {
        emit_numeric_step(
            builder,
            abi,
            native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            *step,
            known_value_kinds,
            current_numeric_values,
            None,
            pre_override,
        )?;
    }

    emit_carried_integer_loop_step(
        builder,
        carried_integer_var,
        carried_step,
        carried_rhs,
        fallback_block,
        known_value_kinds,
    );

    let post_override = HoistedNumericGuardValues {
        first: Some(HoistedNumericGuardValue {
            reg: carried_step.reg,
            source: HoistedNumericGuardSource::Integer(builder.use_var(carried_integer_var)),
        }),
        second: stable_rhs,
    };

    for step in &steps[span_start.saturating_add(span_len)..] {
        emit_numeric_step(
            builder,
            abi,
            native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            *step,
            known_value_kinds,
            current_numeric_values,
            None,
            post_override,
        )?;
    }

    Some(())
}

pub(super) fn emit_numeric_steps_with_carried_float(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    steps: &[NumericStep],
    carried_float_raw_var: Variable,
    carried_step: CarriedFloatLoopStep,
    carried_rhs: ResolvedCarriedFloatRhs,
    span_start: usize,
    span_len: usize,
    stable_rhs: Option<HoistedNumericGuardValue>,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
    current_numeric_values: &mut CurrentNumericGuardValues,
) -> Option<()> {
    let pre_carried = Some(CarriedFloatGuardValue {
        reg: carried_step.reg,
        raw_var: carried_float_raw_var,
    });
    let pre_override = HoistedNumericGuardValues {
        first: stable_rhs,
        second: None,
    };

    for step in &steps[..span_start] {
        emit_numeric_step(
            builder,
            abi,
            native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            *step,
            known_value_kinds,
            current_numeric_values,
            pre_carried,
            pre_override,
        )?;
    }

    emit_carried_float_loop_step(
        builder,
        carried_float_raw_var,
        carried_step,
        carried_rhs,
        known_value_kinds,
    );

    let post_carried = Some(CarriedFloatGuardValue {
        reg: carried_step.reg,
        raw_var: carried_float_raw_var,
    });
    let post_override = HoistedNumericGuardValues {
        first: stable_rhs,
        second: None,
    };

    for step in &steps[span_start.saturating_add(span_len)..] {
        emit_numeric_step(
            builder,
            abi,
            native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            *step,
            known_value_kinds,
            current_numeric_values,
            post_carried,
            post_override,
        )?;
    }

    Some(())
}

pub(super) fn resolve_carried_integer_rhs(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    step: CarriedIntegerLoopStep,
) -> ResolvedCarriedIntegerRhs {
    match step.rhs {
        CarriedIntegerRhs::Imm(value) => {
            ResolvedCarriedIntegerRhs::Imm(builder.ins().iconst(types::I64, value))
        }
        CarriedIntegerRhs::StableReg { reg } => {
            let ptr = slot_addr(builder, base_ptr, reg);
            emit_exact_tag_guard(
                builder,
                ptr,
                LUA_VNUMINT,
                hits_var,
                current_hits,
                bail_block,
            );
            ResolvedCarriedIntegerRhs::Integer(builder.ins().load(
                types::I64,
                MemFlags::new(),
                ptr,
                LUA_VALUE_VALUE_OFFSET,
            ))
        }
    }
}

pub(super) fn resolve_carried_float_rhs(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    step: CarriedFloatLoopStep,
) -> ResolvedCarriedFloatRhs {
    match step.rhs {
        CarriedFloatRhs::Imm(value) => ResolvedCarriedFloatRhs::Imm(value),
        CarriedFloatRhs::StableReg { reg, kind } => {
            let ptr = slot_addr(builder, base_ptr, reg);
            match kind {
                TraceValueKind::Float => {
                    emit_exact_tag_guard(
                        builder,
                        ptr,
                        LUA_VNUMFLT,
                        hits_var,
                        current_hits,
                        bail_block,
                    );
                    ResolvedCarriedFloatRhs::FloatRaw(builder.ins().load(
                        types::I64,
                        MemFlags::new(),
                        ptr,
                        LUA_VALUE_VALUE_OFFSET,
                    ))
                }
                TraceValueKind::Integer => {
                    emit_exact_tag_guard(
                        builder,
                        ptr,
                        LUA_VNUMINT,
                        hits_var,
                        current_hits,
                        bail_block,
                    );
                    ResolvedCarriedFloatRhs::Integer(builder.ins().load(
                        types::I64,
                        MemFlags::new(),
                        ptr,
                        LUA_VALUE_VALUE_OFFSET,
                    ))
                }
                _ => unreachable!(),
            }
        }
    }
}

pub(super) fn hoisted_numeric_guard_value_from_carried_rhs(
    step: CarriedFloatLoopStep,
    rhs: ResolvedCarriedFloatRhs,
) -> Option<HoistedNumericGuardValue> {
    match (step.rhs, rhs) {
        (
            CarriedFloatRhs::StableReg {
                reg,
                kind: TraceValueKind::Float,
            },
            ResolvedCarriedFloatRhs::FloatRaw(raw),
        ) => Some(HoistedNumericGuardValue {
            reg,
            source: HoistedNumericGuardSource::FloatRaw(raw),
        }),
        (
            CarriedFloatRhs::StableReg {
                reg,
                kind: TraceValueKind::Integer,
            },
            ResolvedCarriedFloatRhs::Integer(value),
        ) => Some(HoistedNumericGuardValue {
            reg,
            source: HoistedNumericGuardSource::Integer(value),
        }),
        _ => None,
    }
}

pub(super) fn hoisted_numeric_guard_value_from_carried_integer_rhs(
    step: CarriedIntegerLoopStep,
    rhs: ResolvedCarriedIntegerRhs,
) -> Option<HoistedNumericGuardValue> {
    match (step.rhs, rhs) {
        (CarriedIntegerRhs::StableReg { reg }, ResolvedCarriedIntegerRhs::Integer(value)) => {
            Some(HoistedNumericGuardValue {
                reg,
                source: HoistedNumericGuardSource::Integer(value),
            })
        }
        _ => None,
    }
}

fn lookup_hoisted_numeric_guard_value(
    hoisted_numeric: HoistedNumericGuardValues,
    reg: u32,
) -> Option<HoistedNumericGuardSource> {
    hoisted_numeric
        .first
        .filter(|hoisted| hoisted.reg == reg)
        .map(|hoisted| hoisted.source)
        .or_else(|| {
            hoisted_numeric
                .second
                .filter(|hoisted| hoisted.reg == reg)
                .map(|hoisted| hoisted.source)
        })
}

fn lookup_numeric_guard_value(
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    hoisted_numeric: HoistedNumericGuardValues,
    reg: u32,
) -> Option<HoistedNumericGuardSource> {
    current_numeric_values
        .iter()
        .rev()
        .find_map(|(current_reg, source)| (*current_reg == reg).then_some(*source))
        .or_else(|| lookup_hoisted_numeric_guard_value(hoisted_numeric, reg))
}

fn set_current_numeric_guard_value(
    current_numeric_values: &mut CurrentNumericGuardValues,
    reg: u32,
    source: HoistedNumericGuardSource,
) {
    if let Some((_, current_source)) = current_numeric_values
        .iter_mut()
        .find(|(current_reg, _)| *current_reg == reg)
    {
        *current_source = source;
    } else {
        current_numeric_values.push((reg, source));
    }
}

fn clear_current_numeric_guard_value(
    current_numeric_values: &mut CurrentNumericGuardValues,
    reg: u32,
) {
    current_numeric_values.retain(|(current_reg, _)| *current_reg != reg);
}

fn emit_materialize_guard_numeric_override(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    reg: u32,
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> bool {
    let dst_ptr = slot_addr(builder, abi.base_ptr, reg);
    let mem = MemFlags::new();

    if let Some(override_value) =
        lookup_numeric_guard_value(current_numeric_values, hoisted_numeric, reg)
    {
        match override_value {
            HoistedNumericGuardSource::FloatRaw(raw) => {
                let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
                builder
                    .ins()
                    .store(mem, raw, dst_ptr, LUA_VALUE_VALUE_OFFSET);
                builder
                    .ins()
                    .store(mem, float_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
            }
            HoistedNumericGuardSource::Integer(value) => {
                emit_store_integer_with_known_tag(builder, dst_ptr, value, false);
            }
        }
        return true;
    }

    if let Some(carried) = carried_float.filter(|carried| carried.reg == reg) {
        let raw = builder.use_var(carried.raw_var);
        let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
        builder
            .ins()
            .store(mem, raw, dst_ptr, LUA_VALUE_VALUE_OFFSET);
        builder
            .ins()
            .store(mem, float_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
        return true;
    }

    false
}

fn emit_guard_numeric_override_tag_and_value(
    builder: &mut FunctionBuilder<'_>,
    reg: u32,
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<(Value, Value)> {
    if let Some(override_value) =
        lookup_numeric_guard_value(current_numeric_values, hoisted_numeric, reg)
    {
        return Some(match override_value {
            HoistedNumericGuardSource::FloatRaw(raw) => {
                (builder.ins().iconst(types::I8, LUA_VNUMFLT as i64), raw)
            }
            HoistedNumericGuardSource::Integer(value) => {
                (builder.ins().iconst(types::I8, LUA_VNUMINT as i64), value)
            }
        });
    }

    if let Some(carried) = carried_float.filter(|carried| carried.reg == reg) {
        return Some((
            builder.ins().iconst(types::I8, LUA_VNUMFLT as i64),
            builder.use_var(carried.raw_var),
        ));
    }

    None
}

fn emit_guard_numeric_override_integer_value(
    builder: &mut FunctionBuilder<'_>,
    reg: u32,
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<Value> {
    lookup_numeric_guard_value(current_numeric_values, hoisted_numeric, reg).and_then(|hoisted| {
        match hoisted {
            HoistedNumericGuardSource::Integer(value) => Some(value),
            HoistedNumericGuardSource::FloatRaw(_) => {
                let _ = builder;
                None
            }
        }
    })
}

pub(super) fn emit_carried_float_loop_step(
    builder: &mut FunctionBuilder<'_>,
    carried_float_raw_var: Variable,
    step: CarriedFloatLoopStep,
    rhs: ResolvedCarriedFloatRhs,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
) {
    let carried_raw = builder.use_var(carried_float_raw_var);
    let lhs = builder
        .ins()
        .bitcast(types::F64, MemFlags::new(), carried_raw);
    let rhs = match rhs {
        ResolvedCarriedFloatRhs::Imm(value) => {
            let rhs_raw = builder.ins().iconst(types::I64, value.to_bits() as i64);
            builder.ins().bitcast(types::F64, MemFlags::new(), rhs_raw)
        }
        ResolvedCarriedFloatRhs::FloatRaw(raw) => {
            builder.ins().bitcast(types::F64, MemFlags::new(), raw)
        }
        ResolvedCarriedFloatRhs::Integer(value) => builder.ins().fcvt_from_sint(types::F64, value),
    };
    let result = match step.op {
        NumericBinaryOp::Add => builder.ins().fadd(lhs, rhs),
        NumericBinaryOp::Sub => builder.ins().fsub(lhs, rhs),
        NumericBinaryOp::Mul => builder.ins().fmul(lhs, rhs),
        NumericBinaryOp::Div => builder.ins().fdiv(lhs, rhs),
        _ => unreachable!(),
    };
    let raw = builder.ins().bitcast(types::I64, MemFlags::new(), result);
    builder.def_var(carried_float_raw_var, raw);
    set_numeric_reg_value_kind(known_value_kinds, step.reg, TraceValueKind::Float);
}

fn emit_carried_integer_loop_step(
    builder: &mut FunctionBuilder<'_>,
    carried_integer_var: Variable,
    step: CarriedIntegerLoopStep,
    rhs: ResolvedCarriedIntegerRhs,
    fallback_block: Block,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
) {
    let lhs = builder.use_var(carried_integer_var);
    let rhs = match rhs {
        ResolvedCarriedIntegerRhs::Imm(value) | ResolvedCarriedIntegerRhs::Integer(value) => value,
    };

    let next_value = match step.op {
        NumericBinaryOp::Add => {
            let result = builder.ins().iadd(lhs, rhs);
            let lhs_xor_result = builder.ins().bxor(lhs, result);
            let rhs_xor_result = builder.ins().bxor(rhs, result);
            let overflow_bits = builder.ins().band(lhs_xor_result, rhs_xor_result);
            let overflow = builder
                .ins()
                .icmp_imm(IntCC::SignedLessThan, overflow_bits, 0);
            let ok_block = builder.create_block();
            builder
                .ins()
                .brif(overflow, fallback_block, &[], ok_block, &[]);
            builder.switch_to_block(ok_block);
            builder.seal_block(ok_block);
            result
        }
        NumericBinaryOp::Sub => {
            let result = builder.ins().isub(lhs, rhs);
            let lhs_xor_rhs = builder.ins().bxor(lhs, rhs);
            let lhs_xor_result = builder.ins().bxor(lhs, result);
            let overflow_bits = builder.ins().band(lhs_xor_rhs, lhs_xor_result);
            let overflow = builder
                .ins()
                .icmp_imm(IntCC::SignedLessThan, overflow_bits, 0);
            let ok_block = builder.create_block();
            builder
                .ins()
                .brif(overflow, fallback_block, &[], ok_block, &[]);
            builder.switch_to_block(ok_block);
            builder.seal_block(ok_block);
            result
        }
        NumericBinaryOp::Mul => {
            let zero = builder.ins().iconst(types::I64, 0);
            let neg_one = builder.ins().iconst(types::I64, -1);
            let lhs_is_zero = builder.ins().icmp(IntCC::Equal, lhs, zero);
            let rhs_is_zero = builder.ins().icmp(IntCC::Equal, rhs, zero);
            let either_zero = builder.ins().bor(lhs_is_zero, rhs_is_zero);
            let zero_block = builder.create_block();
            let nonzero_block = builder.create_block();
            let done_block = builder.create_block();
            let result_var = builder.declare_var(types::I64);
            builder
                .ins()
                .brif(either_zero, zero_block, &[], nonzero_block, &[]);

            builder.switch_to_block(zero_block);
            builder.def_var(result_var, zero);
            builder.ins().jump(done_block, &[]);
            builder.seal_block(zero_block);

            builder.switch_to_block(nonzero_block);
            let lhs_is_min = builder.ins().icmp_imm(IntCC::Equal, lhs, i64::MIN);
            let rhs_is_min = builder.ins().icmp_imm(IntCC::Equal, rhs, i64::MIN);
            let lhs_is_neg_one = builder.ins().icmp(IntCC::Equal, lhs, neg_one);
            let rhs_is_neg_one = builder.ins().icmp(IntCC::Equal, rhs, neg_one);
            let lhs_min_rhs_neg_one = builder.ins().band(lhs_is_min, rhs_is_neg_one);
            let rhs_min_lhs_neg_one = builder.ins().band(rhs_is_min, lhs_is_neg_one);
            let special_overflow = builder.ins().bor(lhs_min_rhs_neg_one, rhs_min_lhs_neg_one);
            let mul_compute_block = builder.create_block();
            let mul_store_block = builder.create_block();
            builder.ins().brif(
                special_overflow,
                fallback_block,
                &[],
                mul_compute_block,
                &[],
            );

            builder.switch_to_block(mul_compute_block);
            let result = builder.ins().imul(lhs, rhs);
            let quotient = builder.ins().sdiv(result, rhs);
            let overflow = builder.ins().icmp(IntCC::NotEqual, quotient, lhs);
            builder
                .ins()
                .brif(overflow, fallback_block, &[], mul_store_block, &[]);
            builder.seal_block(mul_compute_block);

            builder.switch_to_block(mul_store_block);
            builder.def_var(result_var, result);
            builder.ins().jump(done_block, &[]);
            builder.seal_block(mul_store_block);
            builder.seal_block(nonzero_block);

            builder.switch_to_block(done_block);
            builder.seal_block(done_block);
            builder.use_var(result_var)
        }
        _ => unreachable!(),
    };

    builder.def_var(carried_integer_var, next_value);
    set_numeric_reg_value_kind(known_value_kinds, step.reg, TraceValueKind::Integer);
}

pub(super) fn native_supports_numeric_step(step: &NumericStep) -> bool {
    match step {
        NumericStep::Move { .. }
        | NumericStep::LoadBool { .. }
        | NumericStep::LoadI { .. }
        | NumericStep::LoadF { .. }
        | NumericStep::GetUpval { .. }
        | NumericStep::SetUpval { .. }
        | NumericStep::GetTabUpField { .. }
        | NumericStep::SetTabUpField { .. }
        | NumericStep::GetTableInt { .. }
        | NumericStep::SetTableInt { .. }
        | NumericStep::GetTableField { .. }
        | NumericStep::SetTableField { .. }
        | NumericStep::Len { .. } => true,
        NumericStep::Binary { lhs, rhs, op, .. } => {
            native_supports_numeric_operand(lhs)
                && native_supports_numeric_operand(rhs)
                && matches!(
                    op,
                    NumericBinaryOp::Add
                        | NumericBinaryOp::Sub
                        | NumericBinaryOp::Mul
                        | NumericBinaryOp::Div
                        | NumericBinaryOp::IDiv
                        | NumericBinaryOp::Mod
                        | NumericBinaryOp::Pow
                        | NumericBinaryOp::BAnd
                        | NumericBinaryOp::BOr
                        | NumericBinaryOp::BXor
                        | NumericBinaryOp::Shl
                        | NumericBinaryOp::Shr
                )
        }
    }
}

fn native_supports_numeric_operand(operand: &NumericOperand) -> bool {
    matches!(
        operand,
        NumericOperand::Reg(_) | NumericOperand::ImmI(_) | NumericOperand::Const(_)
    )
}

pub(super) fn native_supports_numeric_cond(cond: NumericIfElseCond) -> bool {
    matches!(
        cond,
        NumericIfElseCond::RegCompare { .. } | NumericIfElseCond::Truthy { .. }
    )
}

pub(super) fn emit_numeric_step(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    step: NumericStep,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
    current_numeric_values: &mut CurrentNumericGuardValues,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<()> {
    match step {
        NumericStep::Move { dst, src } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let src_kind = if let Some((src_tag, src_val)) =
                emit_guard_numeric_override_tag_and_value(
                    builder,
                    src,
                    current_numeric_values,
                    carried_float,
                    hoisted_numeric,
                ) {
                let mem = MemFlags::new();
                builder
                    .ins()
                    .store(mem, src_val, dst_ptr, LUA_VALUE_VALUE_OFFSET);
                builder
                    .ins()
                    .store(mem, src_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
                if let Some(source) =
                    lookup_numeric_guard_value(current_numeric_values, hoisted_numeric, src)
                {
                    set_current_numeric_guard_value(current_numeric_values, dst, source);
                } else {
                    clear_current_numeric_guard_value(current_numeric_values, dst);
                }
                match src_tag {
                    _ if carried_float.is_some_and(|carried| carried.reg == src) => {
                        TraceValueKind::Float
                    }
                    _ => lookup_numeric_guard_value(current_numeric_values, hoisted_numeric, src)
                        .map(|hoisted| match hoisted {
                            HoistedNumericGuardSource::FloatRaw(_) => TraceValueKind::Float,
                            HoistedNumericGuardSource::Integer(_) => TraceValueKind::Integer,
                        })
                        .unwrap_or(TraceValueKind::Unknown),
                }
            } else {
                let src_ptr = slot_addr(builder, abi.base_ptr, src);
                emit_copy_luavalue(builder, dst_ptr, src_ptr);
                clear_current_numeric_guard_value(current_numeric_values, dst);
                numeric_reg_value_kind(known_value_kinds, src)
            };
            set_numeric_reg_value_kind(known_value_kinds, dst, src_kind);
            Some(())
        }
        NumericStep::LoadBool { dst, value } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let dst_known_boolean = matches!(
                numeric_reg_value_kind(known_value_kinds, dst),
                TraceValueKind::Boolean
            );
            emit_store_boolean_with_known_tag(builder, dst_ptr, value, dst_known_boolean);
            clear_current_numeric_guard_value(current_numeric_values, dst);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Boolean);
            Some(())
        }
        NumericStep::LoadI { dst, imm } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let value = builder.ins().iconst(types::I64, i64::from(imm));
            let dst_known_integer = matches!(
                numeric_reg_value_kind(known_value_kinds, dst),
                TraceValueKind::Integer
            );
            emit_store_integer_with_known_tag(builder, dst_ptr, value, dst_known_integer);
            set_current_numeric_guard_value(
                current_numeric_values,
                dst,
                HoistedNumericGuardSource::Integer(value),
            );
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Integer);
            Some(())
        }
        NumericStep::LoadF { dst, imm } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let raw = builder
                .ins()
                .iconst(types::I64, (imm as f64).to_bits() as i64);
            let dst_known_float = matches!(
                numeric_reg_value_kind(known_value_kinds, dst),
                TraceValueKind::Float
            );
            emit_store_float_with_known_tag(builder, dst_ptr, imm as f64, dst_known_float);
            set_current_numeric_guard_value(
                current_numeric_values,
                dst,
                HoistedNumericGuardSource::FloatRaw(raw),
            );
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Float);
            Some(())
        }
        NumericStep::GetUpval { dst, upvalue } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let upvalue_index = builder.ins().iconst(abi.pointer_ty, i64::from(upvalue));
            let call = builder.ins().call(
                native_helpers.get_upval,
                &[dst_ptr, abi.upvalue_ptrs, upvalue_index],
            );
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            clear_current_numeric_guard_value(current_numeric_values, dst);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Unknown);
            Some(())
        }
        NumericStep::SetUpval { src, upvalue } => {
            let src_ptr = slot_addr(builder, abi.base_ptr, src);
            let upvalue_index = builder.ins().iconst(abi.pointer_ty, i64::from(upvalue));
            let call = builder.ins().call(
                native_helpers.set_upval,
                &[abi.lua_state_ptr, abi.upvalue_ptrs, src_ptr, upvalue_index],
            );
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            Some(())
        }
        NumericStep::GetTabUpField { dst, upvalue, key } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let upvalue_index = builder.ins().iconst(abi.pointer_ty, i64::from(upvalue));
            let key_ptr = const_addr(builder, abi, hits_var, current_hits, fallback_block, key);
            let call = builder.ins().call(
                native_helpers.get_tabup_field,
                &[
                    abi.lua_state_ptr,
                    dst_ptr,
                    abi.upvalue_ptrs,
                    upvalue_index,
                    key_ptr,
                ],
            );
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            clear_current_numeric_guard_value(current_numeric_values, dst);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
            Some(())
        }
        NumericStep::SetTabUpField {
            upvalue,
            key,
            value,
        } => {
            emit_materialize_guard_numeric_override(
                builder,
                abi,
                value,
                current_numeric_values,
                carried_float,
                hoisted_numeric,
            );
            let upvalue_index = builder.ins().iconst(abi.pointer_ty, i64::from(upvalue));
            let key_ptr = const_addr(builder, abi, hits_var, current_hits, fallback_block, key);
            let value_ptr = slot_addr(builder, abi.base_ptr, value);
            let call = builder.ins().call(
                native_helpers.set_tabup_field,
                &[
                    abi.lua_state_ptr,
                    abi.upvalue_ptrs,
                    upvalue_index,
                    key_ptr,
                    value_ptr,
                ],
            );
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            Some(())
        }
        NumericStep::GetTableInt { dst, table, index } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let table_ptr = slot_addr(builder, abi.base_ptr, table);
            let index_ptr = slot_addr(builder, abi.base_ptr, index);
            let mem = MemFlags::new();

            // Inline fast path: direct array access without helper call.
            let helper_block = builder.create_block();
            let done_block = builder.create_block();

            // Guard: table tag == LUA_VTABLE
            let table_tt = builder
                .ins()
                .load(types::I8, mem, table_ptr, LUA_VALUE_TT_OFFSET);
            let is_table = builder
                .ins()
                .icmp_imm(IntCC::Equal, table_tt, LUA_VTABLE_TAG);
            let table_ok_block = builder.create_block();
            builder.def_var(hits_var, current_hits);
            builder
                .ins()
                .brif(is_table, table_ok_block, &[], helper_block, &[]);

            builder.switch_to_block(table_ok_block);
            builder.seal_block(table_ok_block);

            // Guard: index tag == LUA_VNUMINT
            let index_tt = builder
                .ins()
                .load(types::I8, mem, index_ptr, LUA_VALUE_TT_OFFSET);
            let is_int = builder
                .ins()
                .icmp_imm(IntCC::Equal, index_tt, i64::from(LUA_VNUMINT));
            let index_ok_block = builder.create_block();
            builder
                .ins()
                .brif(is_int, index_ok_block, &[], helper_block, &[]);

            builder.switch_to_block(index_ok_block);
            builder.seal_block(index_ok_block);

            // Extract table GC pointer → NativeTable
            let gc_ptr = builder
                .ins()
                .load(types::I64, mem, table_ptr, LUA_VALUE_VALUE_OFFSET);
            let native_table_offset = GC_TABLE_DATA_OFFSET + LUA_TABLE_IMPL_OFFSET;
            let native_table_ptr = builder
                .ins()
                .iadd_imm(gc_ptr, i64::from(native_table_offset));

            // Load array pointer and asize
            let array_ptr =
                builder
                    .ins()
                    .load(types::I64, mem, native_table_ptr, NATIVE_TABLE_ARRAY_OFFSET);
            let asize =
                builder
                    .ins()
                    .load(types::I32, mem, native_table_ptr, NATIVE_TABLE_ASIZE_OFFSET);

            // Extract integer key and compute 0-based index k = key - 1
            let key_val = builder
                .ins()
                .load(types::I64, mem, index_ptr, LUA_VALUE_VALUE_OFFSET);
            let k = builder.ins().iadd_imm(key_val, -1);

            // Bounds check: (k as u64) < (asize as u64)
            let asize_64 = builder.ins().uextend(types::I64, asize);
            let in_bounds = builder.ins().icmp(IntCC::UnsignedLessThan, k, asize_64);
            let bounds_ok_block = builder.create_block();
            builder
                .ins()
                .brif(in_bounds, bounds_ok_block, &[], helper_block, &[]);

            builder.switch_to_block(bounds_ok_block);
            builder.seal_block(bounds_ok_block);

            // Load tag: array_ptr + 4 + k (tags at positive offsets)
            let tag_base = builder
                .ins()
                .iadd_imm(array_ptr, i64::from(ARRAY_TAG_BASE_OFFSET));
            let tag_addr = builder.ins().iadd(tag_base, k);
            let tag = builder.ins().load(types::I8, mem, tag_addr, 0);

            // Guard: non-nil and numeric — (tag & 0x0F) == 3
            let tag_i32 = builder.ins().uextend(types::I32, tag);
            let tag_low = builder.ins().band_imm(tag_i32, 0x0F);
            let is_numeric = builder.ins().icmp_imm(IntCC::Equal, tag_low, 3);
            let numeric_ok_block = builder.create_block();
            builder
                .ins()
                .brif(is_numeric, numeric_ok_block, &[], helper_block, &[]);

            builder.switch_to_block(numeric_ok_block);
            builder.seal_block(numeric_ok_block);

            // Load value: array_ptr - 8*(1+k) (values at negative offsets)
            let k_plus_1 = builder.ins().iadd_imm(k, 1);
            let val_offset_neg = builder.ins().imul_imm(k_plus_1, VALUE_SIZE_BYTES);
            let val_addr = builder.ins().isub(array_ptr, val_offset_neg);
            let val = builder.ins().load(types::I64, mem, val_addr, 0);

            // Store result to dst
            builder
                .ins()
                .store(mem, val, dst_ptr, LUA_VALUE_VALUE_OFFSET);
            let tag_for_store = builder.ins().ireduce(types::I8, tag_i32);
            builder
                .ins()
                .store(mem, tag_for_store, dst_ptr, LUA_VALUE_TT_OFFSET);

            builder.ins().jump(done_block, &[]);

            // Helper fallback block
            builder.switch_to_block(helper_block);
            builder.seal_block(helper_block);
            let fallback_hits = builder.use_var(hits_var);
            let call = builder.ins().call(
                native_helpers.get_table_int,
                &[abi.lua_state_ptr, dst_ptr, table_ptr, index_ptr],
            );
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, fallback_hits, fallback_block, success);
            builder.ins().jump(done_block, &[]);

            builder.switch_to_block(done_block);
            builder.seal_block(done_block);

            clear_current_numeric_guard_value(current_numeric_values, dst);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
            Some(())
        }
        NumericStep::GetTableField { dst, table, key } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let table_ptr = slot_addr(builder, abi.base_ptr, table);
            let key_ptr = const_addr(builder, abi, hits_var, current_hits, fallback_block, key);
            let call = builder.ins().call(
                native_helpers.get_table_field,
                &[abi.lua_state_ptr, dst_ptr, table_ptr, key_ptr],
            );
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            clear_current_numeric_guard_value(current_numeric_values, dst);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
            Some(())
        }
        NumericStep::Len { dst, src } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let value_ptr = slot_addr(builder, abi.base_ptr, src);
            let call = builder
                .ins()
                .call(native_helpers.len, &[abi.lua_state_ptr, dst_ptr, value_ptr]);
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            clear_current_numeric_guard_value(current_numeric_values, dst);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
            Some(())
        }
        NumericStep::SetTableInt {
            table,
            index,
            value,
        } => {
            emit_materialize_guard_numeric_override(
                builder,
                abi,
                value,
                current_numeric_values,
                carried_float,
                hoisted_numeric,
            );
            emit_materialize_guard_numeric_override(
                builder,
                abi,
                index,
                current_numeric_values,
                carried_float,
                hoisted_numeric,
            );
            let table_ptr = slot_addr(builder, abi.base_ptr, table);
            let index_ptr = slot_addr(builder, abi.base_ptr, index);
            let value_ptr = slot_addr(builder, abi.base_ptr, value);
            let call = builder.ins().call(
                native_helpers.set_table_int,
                &[abi.lua_state_ptr, table_ptr, index_ptr, value_ptr],
            );
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            Some(())
        }
        NumericStep::SetTableField { table, key, value } => {
            emit_materialize_guard_numeric_override(
                builder,
                abi,
                value,
                current_numeric_values,
                carried_float,
                hoisted_numeric,
            );
            let table_ptr = slot_addr(builder, abi.base_ptr, table);
            let key_ptr = const_addr(builder, abi, hits_var, current_hits, fallback_block, key);
            let value_ptr = slot_addr(builder, abi.base_ptr, value);
            let call = builder.ins().call(
                native_helpers.set_table_field,
                &[abi.lua_state_ptr, table_ptr, key_ptr, value_ptr],
            );
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            Some(())
        }
        NumericStep::Binary { dst, lhs, rhs, op } => {
            let dst_known_kind = numeric_reg_value_kind(known_value_kinds, dst);
            if matches!(
                op,
                NumericBinaryOp::Add | NumericBinaryOp::Sub | NumericBinaryOp::Mul
            ) {
                emit_integer_add_sub_mul_with_helper_fallback(
                    builder,
                    abi,
                    native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    dst,
                    lhs,
                    rhs,
                    op,
                    known_value_kinds,
                    matches!(dst_known_kind, TraceValueKind::Integer),
                    matches!(dst_known_kind, TraceValueKind::Float),
                    current_numeric_values,
                    carried_float,
                    hoisted_numeric,
                )?;
                clear_current_numeric_guard_value(current_numeric_values, dst);
                set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
                return Some(());
            }

            if matches!(op, NumericBinaryOp::Div) {
                emit_numeric_div_with_helper_fallback(
                    builder,
                    abi,
                    native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    dst,
                    lhs,
                    rhs,
                    known_value_kinds,
                    matches!(dst_known_kind, TraceValueKind::Float),
                    current_numeric_values,
                    carried_float,
                    hoisted_numeric,
                )?;
                clear_current_numeric_guard_value(current_numeric_values, dst);
                set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
                return Some(());
            }

            if matches!(op, NumericBinaryOp::Pow) {
                emit_numeric_pow_with_helper_fallback(
                    builder,
                    abi,
                    native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    dst,
                    lhs,
                    rhs,
                    known_value_kinds,
                    matches!(dst_known_kind, TraceValueKind::Float),
                    current_numeric_values,
                    carried_float,
                    hoisted_numeric,
                )?;
                clear_current_numeric_guard_value(current_numeric_values, dst);
                set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
                return Some(());
            }

            if matches!(op, NumericBinaryOp::Mod | NumericBinaryOp::IDiv) {
                let lhs_val = emit_numeric_integer_operand(
                    builder,
                    abi,
                    hits_var,
                    current_hits,
                    fallback_block,
                    lhs,
                    known_value_kinds,
                    current_numeric_values,
                    carried_float,
                    hoisted_numeric,
                )?;
                let rhs_val = emit_numeric_integer_operand(
                    builder,
                    abi,
                    hits_var,
                    current_hits,
                    fallback_block,
                    rhs,
                    known_value_kinds,
                    current_numeric_values,
                    carried_float,
                    hoisted_numeric,
                )?;
                let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
                if matches!(op, NumericBinaryOp::Mod) {
                    emit_integer_mod(
                        builder,
                        hits_var,
                        current_hits,
                        fallback_block,
                        dst_ptr,
                        lhs_val,
                        rhs_val,
                        matches!(dst_known_kind, TraceValueKind::Integer),
                    );
                } else {
                    emit_integer_idiv(
                        builder,
                        hits_var,
                        current_hits,
                        fallback_block,
                        dst_ptr,
                        lhs_val,
                        rhs_val,
                        matches!(dst_known_kind, TraceValueKind::Integer),
                    );
                }
                clear_current_numeric_guard_value(current_numeric_values, dst);
                set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Integer);
                return Some(());
            }

            if matches!(
                op,
                NumericBinaryOp::Add
                    | NumericBinaryOp::Sub
                    | NumericBinaryOp::Mul
                    | NumericBinaryOp::Div
                    | NumericBinaryOp::IDiv
                    | NumericBinaryOp::Mod
                    | NumericBinaryOp::Pow
            ) {
                let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
                let (lhs_kind, lhs_payload) = emit_numeric_operand_kind_and_payload(builder, lhs);
                let (rhs_kind, rhs_payload) = emit_numeric_operand_kind_and_payload(builder, rhs);
                let opcode = emit_numeric_binary_helper_opcode(builder, op)?;
                let call = builder.ins().call(
                    native_helpers.numeric_binary,
                    &[
                        dst_ptr,
                        abi.base_ptr,
                        abi.constants_ptr,
                        abi.constants_len,
                        lhs_kind,
                        lhs_payload,
                        rhs_kind,
                        rhs_payload,
                        opcode,
                    ],
                );
                let success = builder.inst_results(call)[0];
                emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
                clear_current_numeric_guard_value(current_numeric_values, dst);
                return Some(());
            }

            let lhs_val = emit_numeric_integer_operand(
                builder,
                abi,
                hits_var,
                current_hits,
                fallback_block,
                lhs,
                known_value_kinds,
                current_numeric_values,
                carried_float,
                hoisted_numeric,
            )?;
            let rhs_val = emit_numeric_integer_operand(
                builder,
                abi,
                hits_var,
                current_hits,
                fallback_block,
                rhs,
                known_value_kinds,
                current_numeric_values,
                carried_float,
                hoisted_numeric,
            )?;
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let result = match op {
                NumericBinaryOp::Add => unreachable!(),
                NumericBinaryOp::Sub => unreachable!(),
                NumericBinaryOp::Mul => unreachable!(),
                NumericBinaryOp::BAnd => builder.ins().band(lhs_val, rhs_val),
                NumericBinaryOp::BOr => builder.ins().bor(lhs_val, rhs_val),
                NumericBinaryOp::BXor => builder.ins().bxor(lhs_val, rhs_val),
                NumericBinaryOp::Shl => {
                    let call = builder
                        .ins()
                        .call(native_helpers.shift_left, &[lhs_val, rhs_val]);
                    builder.inst_results(call)[0]
                }
                NumericBinaryOp::Shr => {
                    let call = builder
                        .ins()
                        .call(native_helpers.shift_right, &[lhs_val, rhs_val]);
                    builder.inst_results(call)[0]
                }
                NumericBinaryOp::Div
                | NumericBinaryOp::IDiv
                | NumericBinaryOp::Mod
                | NumericBinaryOp::Pow => unreachable!(),
            };
            emit_store_integer_with_known_tag(
                builder,
                dst_ptr,
                result,
                matches!(dst_known_kind, TraceValueKind::Integer),
            );
            clear_current_numeric_guard_value(current_numeric_values, dst);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Integer);
            Some(())
        }
    }
}

fn emit_numeric_integer_operand(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    operand: NumericOperand,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<Value> {
    let mem = MemFlags::new();
    match operand {
        NumericOperand::ImmI(imm) => Some(builder.ins().iconst(types::I64, i64::from(imm))),
        NumericOperand::Reg(reg) => {
            if let Some(value) = emit_guard_numeric_override_integer_value(
                builder,
                reg,
                current_numeric_values,
                hoisted_numeric,
            ) {
                return Some(value);
            }
            let reg_ptr = slot_addr(builder, abi.base_ptr, reg);
            let _ = emit_materialize_guard_numeric_override(
                builder,
                abi,
                reg,
                current_numeric_values,
                carried_float,
                hoisted_numeric,
            );
            if !matches!(
                numeric_reg_value_kind(known_value_kinds, reg),
                TraceValueKind::Integer
            ) {
                emit_integer_guard(builder, reg_ptr, hits_var, current_hits, fallback_block);
            }
            Some(
                builder
                    .ins()
                    .load(types::I64, mem, reg_ptr, LUA_VALUE_VALUE_OFFSET),
            )
        }
        NumericOperand::Const(index) => {
            let const_ptr = const_addr(builder, abi, hits_var, current_hits, fallback_block, index);
            emit_integer_guard(builder, const_ptr, hits_var, current_hits, fallback_block);
            Some(
                builder
                    .ins()
                    .load(types::I64, mem, const_ptr, LUA_VALUE_VALUE_OFFSET),
            )
        }
    }
}

fn emit_numeric_operand_tag_and_value(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    operand: NumericOperand,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<(Value, Value)> {
    let mem = MemFlags::new();
    match operand {
        NumericOperand::ImmI(imm) => Some((
            builder.ins().iconst(types::I8, LUA_VNUMINT as i64),
            builder.ins().iconst(types::I64, i64::from(imm)),
        )),
        NumericOperand::Reg(reg) => {
            if let Some(result) = emit_guard_numeric_override_tag_and_value(
                builder,
                reg,
                current_numeric_values,
                carried_float,
                hoisted_numeric,
            ) {
                return Some(result);
            }
            let reg_ptr = slot_addr(builder, abi.base_ptr, reg);
            let tag = if let Some(tag) =
                trace_value_kind_tag(numeric_reg_value_kind(known_value_kinds, reg))
            {
                builder.ins().iconst(types::I8, i64::from(tag))
            } else {
                builder
                    .ins()
                    .load(types::I8, mem, reg_ptr, LUA_VALUE_TT_OFFSET)
            };
            let value = builder
                .ins()
                .load(types::I64, mem, reg_ptr, LUA_VALUE_VALUE_OFFSET);
            Some((tag, value))
        }
        NumericOperand::Const(index) => {
            let const_ptr = const_addr(builder, abi, hits_var, current_hits, fallback_block, index);
            let tag = builder
                .ins()
                .load(types::I8, mem, const_ptr, LUA_VALUE_TT_OFFSET);
            let value = builder
                .ins()
                .load(types::I64, mem, const_ptr, LUA_VALUE_VALUE_OFFSET);
            Some((tag, value))
        }
    }
}

fn emit_numeric_binary_helper_call(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst: u32,
    lhs: NumericOperand,
    rhs: NumericOperand,
    op: NumericBinaryOp,
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<()> {
    if let NumericOperand::Reg(reg) = lhs {
        let _ = emit_materialize_guard_numeric_override(
            builder,
            abi,
            reg,
            current_numeric_values,
            carried_float,
            hoisted_numeric,
        );
    }
    if let NumericOperand::Reg(reg) = rhs {
        let _ = emit_materialize_guard_numeric_override(
            builder,
            abi,
            reg,
            current_numeric_values,
            carried_float,
            hoisted_numeric,
        );
    }
    let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
    let (lhs_kind, lhs_payload) = emit_numeric_operand_kind_and_payload(builder, lhs);
    let (rhs_kind, rhs_payload) = emit_numeric_operand_kind_and_payload(builder, rhs);
    let opcode = emit_numeric_binary_helper_opcode(builder, op)?;
    let call = builder.ins().call(
        native_helpers.numeric_binary,
        &[
            dst_ptr,
            abi.base_ptr,
            abi.constants_ptr,
            abi.constants_len,
            lhs_kind,
            lhs_payload,
            rhs_kind,
            rhs_payload,
            opcode,
        ],
    );
    let success = builder.inst_results(call)[0];
    emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
    Some(())
}

fn emit_integer_add_sub_mul_with_helper_fallback(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst: u32,
    lhs: NumericOperand,
    rhs: NumericOperand,
    op: NumericBinaryOp,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    dst_known_integer: bool,
    dst_known_float: bool,
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<()> {
    let (lhs_tag, lhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        lhs,
        known_value_kinds,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )?;
    let (rhs_tag, rhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        rhs,
        known_value_kinds,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )?;
    let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
    let int_fast_block = builder.create_block();
    let numeric_fast_block = builder.create_block();
    let helper_block = builder.create_block();
    let done_block = builder.create_block();
    let float_store_block = builder.create_block();
    let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
    let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
    let lhs_is_int = builder.ins().icmp(IntCC::Equal, lhs_tag, int_tag);
    let rhs_is_int = builder.ins().icmp(IntCC::Equal, rhs_tag, int_tag);
    let both_int = builder.ins().band(lhs_is_int, rhs_is_int);
    let lhs_is_float = builder.ins().icmp(IntCC::Equal, lhs_tag, float_tag);
    let rhs_is_float = builder.ins().icmp(IntCC::Equal, rhs_tag, float_tag);
    let lhs_is_numeric = builder.ins().bor(lhs_is_int, lhs_is_float);
    let rhs_is_numeric = builder.ins().bor(rhs_is_int, rhs_is_float);
    let both_numeric = builder.ins().band(lhs_is_numeric, rhs_is_numeric);
    builder.def_var(hits_var, current_hits);
    builder
        .ins()
        .brif(both_int, int_fast_block, &[], numeric_fast_block, &[]);

    builder.switch_to_block(int_fast_block);
    match op {
        NumericBinaryOp::Add => {
            let int_store_block = builder.create_block();
            let result = builder.ins().iadd(lhs_val, rhs_val);
            let lhs_xor_result = builder.ins().bxor(lhs_val, result);
            let rhs_xor_result = builder.ins().bxor(rhs_val, result);
            let overflow_bits = builder.ins().band(lhs_xor_result, rhs_xor_result);
            let overflow = builder
                .ins()
                .icmp_imm(IntCC::SignedLessThan, overflow_bits, 0);
            builder
                .ins()
                .brif(overflow, helper_block, &[], int_store_block, &[]);

            builder.switch_to_block(int_store_block);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            builder.ins().jump(done_block, &[]);
            builder.seal_block(int_store_block);
        }
        NumericBinaryOp::Sub => {
            let int_store_block = builder.create_block();
            let result = builder.ins().isub(lhs_val, rhs_val);
            let lhs_xor_rhs = builder.ins().bxor(lhs_val, rhs_val);
            let lhs_xor_result = builder.ins().bxor(lhs_val, result);
            let overflow_bits = builder.ins().band(lhs_xor_rhs, lhs_xor_result);
            let overflow = builder
                .ins()
                .icmp_imm(IntCC::SignedLessThan, overflow_bits, 0);
            builder
                .ins()
                .brif(overflow, helper_block, &[], int_store_block, &[]);

            builder.switch_to_block(int_store_block);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            builder.ins().jump(done_block, &[]);
            builder.seal_block(int_store_block);
        }
        NumericBinaryOp::Mul => {
            let zero = builder.ins().iconst(types::I64, 0);
            let neg_one = builder.ins().iconst(types::I64, -1);
            let lhs_is_zero = builder.ins().icmp(IntCC::Equal, lhs_val, zero);
            let rhs_is_zero = builder.ins().icmp(IntCC::Equal, rhs_val, zero);
            let either_zero = builder.ins().bor(lhs_is_zero, rhs_is_zero);
            let zero_block = builder.create_block();
            let nonzero_block = builder.create_block();
            let mul_store_block = builder.create_block();
            builder
                .ins()
                .brif(either_zero, zero_block, &[], nonzero_block, &[]);

            builder.switch_to_block(zero_block);
            emit_store_integer_with_known_tag(builder, dst_ptr, zero, dst_known_integer);
            builder.ins().jump(done_block, &[]);
            builder.seal_block(zero_block);

            builder.switch_to_block(nonzero_block);
            let lhs_is_min = builder.ins().icmp_imm(IntCC::Equal, lhs_val, i64::MIN);
            let rhs_is_min = builder.ins().icmp_imm(IntCC::Equal, rhs_val, i64::MIN);
            let lhs_is_neg_one = builder.ins().icmp(IntCC::Equal, lhs_val, neg_one);
            let rhs_is_neg_one = builder.ins().icmp(IntCC::Equal, rhs_val, neg_one);
            let lhs_min_rhs_neg_one = builder.ins().band(lhs_is_min, rhs_is_neg_one);
            let rhs_min_lhs_neg_one = builder.ins().band(rhs_is_min, lhs_is_neg_one);
            let special_overflow = builder.ins().bor(lhs_min_rhs_neg_one, rhs_min_lhs_neg_one);
            let mul_compute_block = builder.create_block();
            builder
                .ins()
                .brif(special_overflow, helper_block, &[], mul_compute_block, &[]);

            builder.switch_to_block(mul_compute_block);
            let result = builder.ins().imul(lhs_val, rhs_val);
            let quotient = builder.ins().sdiv(result, rhs_val);
            let overflow = builder.ins().icmp(IntCC::NotEqual, quotient, lhs_val);
            builder
                .ins()
                .brif(overflow, helper_block, &[], mul_store_block, &[]);
            builder.seal_block(mul_compute_block);

            builder.switch_to_block(mul_store_block);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            builder.ins().jump(done_block, &[]);
            builder.seal_block(mul_store_block);
            builder.seal_block(nonzero_block);
        }
        _ => unreachable!(),
    }
    builder.seal_block(int_fast_block);

    builder.switch_to_block(numeric_fast_block);
    builder
        .ins()
        .brif(both_numeric, float_store_block, &[], helper_block, &[]);
    builder.seal_block(numeric_fast_block);

    builder.switch_to_block(float_store_block);
    let lhs_num = emit_numeric_tagged_value_to_float(builder, lhs_tag, lhs_val);
    let rhs_num = emit_numeric_tagged_value_to_float(builder, rhs_tag, rhs_val);
    let result = match op {
        NumericBinaryOp::Add => builder.ins().fadd(lhs_num, rhs_num),
        NumericBinaryOp::Sub => builder.ins().fsub(lhs_num, rhs_num),
        NumericBinaryOp::Mul => builder.ins().fmul(lhs_num, rhs_num),
        _ => unreachable!(),
    };
    emit_store_float_value_with_known_tag(builder, dst_ptr, result, dst_known_float);
    builder.ins().jump(done_block, &[]);
    builder.seal_block(float_store_block);

    builder.switch_to_block(helper_block);
    emit_numeric_binary_helper_call(
        builder,
        abi,
        native_helpers,
        hits_var,
        current_hits,
        fallback_block,
        dst,
        lhs,
        rhs,
        op,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )?;
    builder.ins().jump(done_block, &[]);
    builder.seal_block(helper_block);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);
    Some(())
}

fn emit_numeric_div_with_helper_fallback(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst: u32,
    lhs: NumericOperand,
    rhs: NumericOperand,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    dst_known_float: bool,
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<()> {
    let (lhs_tag, lhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        lhs,
        known_value_kinds,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )?;
    let (rhs_tag, rhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        rhs,
        known_value_kinds,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )?;
    let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
    let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
    let lhs_is_int = builder.ins().icmp(IntCC::Equal, lhs_tag, int_tag);
    let lhs_is_float = builder.ins().icmp(IntCC::Equal, lhs_tag, float_tag);
    let rhs_is_int = builder.ins().icmp(IntCC::Equal, rhs_tag, int_tag);
    let rhs_is_float = builder.ins().icmp(IntCC::Equal, rhs_tag, float_tag);
    let lhs_is_numeric = builder.ins().bor(lhs_is_int, lhs_is_float);
    let rhs_is_numeric = builder.ins().bor(rhs_is_int, rhs_is_float);
    let both_numeric = builder.ins().band(lhs_is_numeric, rhs_is_numeric);
    let fast_block = builder.create_block();
    let helper_block = builder.create_block();
    let done_block = builder.create_block();
    let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
    builder.def_var(hits_var, current_hits);
    builder
        .ins()
        .brif(both_numeric, fast_block, &[], helper_block, &[]);

    builder.switch_to_block(fast_block);
    let lhs_as_float_int = builder.ins().fcvt_from_sint(types::F64, lhs_val);
    let lhs_as_float_raw = builder.ins().bitcast(types::F64, MemFlags::new(), lhs_val);
    let lhs_as_float = builder
        .ins()
        .select(lhs_is_int, lhs_as_float_int, lhs_as_float_raw);
    let rhs_as_float_int = builder.ins().fcvt_from_sint(types::F64, rhs_val);
    let rhs_as_float_raw = builder.ins().bitcast(types::F64, MemFlags::new(), rhs_val);
    let rhs_as_float = builder
        .ins()
        .select(rhs_is_int, rhs_as_float_int, rhs_as_float_raw);
    let result = builder.ins().fdiv(lhs_as_float, rhs_as_float);
    emit_store_float_value_with_known_tag(builder, dst_ptr, result, dst_known_float);
    builder.ins().jump(done_block, &[]);
    builder.seal_block(fast_block);

    builder.switch_to_block(helper_block);
    emit_numeric_binary_helper_call(
        builder,
        abi,
        native_helpers,
        hits_var,
        current_hits,
        fallback_block,
        dst,
        lhs,
        rhs,
        NumericBinaryOp::Div,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )?;
    builder.ins().jump(done_block, &[]);
    builder.seal_block(helper_block);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);
    Some(())
}

fn emit_numeric_pow_with_helper_fallback(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst: u32,
    lhs: NumericOperand,
    rhs: NumericOperand,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    dst_known_float: bool,
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<()> {
    let (lhs_tag, lhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        lhs,
        known_value_kinds,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )?;
    let (rhs_tag, rhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        rhs,
        known_value_kinds,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )?;
    let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
    let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
    let lhs_is_int = builder.ins().icmp(IntCC::Equal, lhs_tag, int_tag);
    let lhs_is_float = builder.ins().icmp(IntCC::Equal, lhs_tag, float_tag);
    let rhs_is_int = builder.ins().icmp(IntCC::Equal, rhs_tag, int_tag);
    let rhs_is_float = builder.ins().icmp(IntCC::Equal, rhs_tag, float_tag);
    let lhs_is_numeric = builder.ins().bor(lhs_is_int, lhs_is_float);
    let rhs_is_numeric = builder.ins().bor(rhs_is_int, rhs_is_float);
    let both_numeric = builder.ins().band(lhs_is_numeric, rhs_is_numeric);
    let fast_block = builder.create_block();
    let helper_block = builder.create_block();
    let done_block = builder.create_block();
    let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
    builder.def_var(hits_var, current_hits);
    builder
        .ins()
        .brif(both_numeric, fast_block, &[], helper_block, &[]);

    builder.switch_to_block(fast_block);
    let lhs_num = emit_numeric_tagged_value_to_float(builder, lhs_tag, lhs_val);
    let rhs_num = emit_numeric_tagged_value_to_float(builder, rhs_tag, rhs_val);
    let call = builder
        .ins()
        .call(native_helpers.numeric_pow, &[lhs_num, rhs_num]);
    let result = builder.inst_results(call)[0];
    emit_store_float_value_with_known_tag(builder, dst_ptr, result, dst_known_float);
    builder.ins().jump(done_block, &[]);
    builder.seal_block(fast_block);

    builder.switch_to_block(helper_block);
    emit_numeric_binary_helper_call(
        builder,
        abi,
        native_helpers,
        hits_var,
        current_hits,
        fallback_block,
        dst,
        lhs,
        rhs,
        NumericBinaryOp::Pow,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )?;
    builder.ins().jump(done_block, &[]);
    builder.seal_block(helper_block);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);
    Some(())
}

fn emit_integer_mod(
    builder: &mut FunctionBuilder<'_>,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst_ptr: Value,
    lhs_val: Value,
    rhs_val: Value,
    dst_known_integer: bool,
) {
    let zero = builder.ins().iconst(types::I64, 0);
    let rhs_is_zero = builder.ins().icmp(IntCC::Equal, rhs_val, zero);
    let compute_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    builder
        .ins()
        .brif(rhs_is_zero, fallback_block, &[], compute_block, &[]);

    builder.switch_to_block(compute_block);
    builder.seal_block(compute_block);
    let remainder = builder.ins().srem(lhs_val, rhs_val);
    let rem_is_zero = builder.ins().icmp(IntCC::Equal, remainder, zero);
    let xor = builder.ins().bxor(remainder, rhs_val);
    let sign_diff = builder.ins().icmp_imm(IntCC::SignedLessThan, xor, 0);
    let adjusted = builder.ins().iadd(remainder, rhs_val);
    let maybe_adjusted = builder.ins().select(sign_diff, adjusted, remainder);
    let result = builder.ins().select(rem_is_zero, remainder, maybe_adjusted);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
}

fn emit_integer_idiv(
    builder: &mut FunctionBuilder<'_>,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst_ptr: Value,
    lhs_val: Value,
    rhs_val: Value,
    dst_known_integer: bool,
) {
    let zero = builder.ins().iconst(types::I64, 0);
    let neg_one = builder.ins().iconst(types::I64, -1);
    let rhs_is_zero = builder.ins().icmp(IntCC::Equal, rhs_val, zero);
    let neg_one_block = builder.create_block();
    let compute_block = builder.create_block();
    let normal_block = builder.create_block();
    let done_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    builder
        .ins()
        .brif(rhs_is_zero, fallback_block, &[], compute_block, &[]);

    builder.switch_to_block(compute_block);
    builder.seal_block(compute_block);
    let rhs_is_neg_one = builder.ins().icmp(IntCC::Equal, rhs_val, neg_one);
    builder.def_var(hits_var, current_hits);
    builder
        .ins()
        .brif(rhs_is_neg_one, neg_one_block, &[], normal_block, &[]);

    builder.switch_to_block(neg_one_block);
    builder.seal_block(neg_one_block);
    let negated = builder.ins().ineg(lhs_val);
    emit_store_integer_with_known_tag(builder, dst_ptr, negated, dst_known_integer);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(normal_block);
    builder.seal_block(normal_block);
    let quotient = builder.ins().sdiv(lhs_val, rhs_val);
    let remainder = builder.ins().srem(lhs_val, rhs_val);
    let rem_is_zero = builder.ins().icmp(IntCC::Equal, remainder, zero);
    let xor = builder.ins().bxor(lhs_val, rhs_val);
    let sign_diff = builder.ins().icmp_imm(IntCC::SignedLessThan, xor, 0);
    let floor_adjust = builder.ins().iadd_imm(quotient, -1);
    let adjusted = builder.ins().select(sign_diff, floor_adjust, quotient);
    let result = builder.ins().select(rem_is_zero, quotient, adjusted);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);
}

pub(super) fn emit_numeric_condition_value(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    cond: NumericIfElseCond,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    current_numeric_values: &[(u32, HoistedNumericGuardSource)],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<Value> {
    let mem = MemFlags::new();
    match cond {
        NumericIfElseCond::RegCompare { op, lhs, rhs } => {
            let lhs_ptr = slot_addr(builder, abi.base_ptr, lhs);
            let rhs_ptr = slot_addr(builder, abi.base_ptr, rhs);
            let lhs_tag = if let Some(override_value) =
                lookup_numeric_guard_value(current_numeric_values, hoisted_numeric, lhs)
            {
                match override_value {
                    HoistedNumericGuardSource::FloatRaw(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
                    }
                    HoistedNumericGuardSource::Integer(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMINT as i64)
                    }
                }
            } else if carried_float.is_some_and(|carried| carried.reg == lhs) {
                builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
            } else if let Some(tag) =
                trace_value_kind_tag(numeric_reg_value_kind(known_value_kinds, lhs))
            {
                builder.ins().iconst(types::I8, i64::from(tag))
            } else {
                builder
                    .ins()
                    .load(types::I8, mem, lhs_ptr, LUA_VALUE_TT_OFFSET)
            };
            let rhs_tag = if let Some(override_value) =
                lookup_numeric_guard_value(current_numeric_values, hoisted_numeric, rhs)
            {
                match override_value {
                    HoistedNumericGuardSource::FloatRaw(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
                    }
                    HoistedNumericGuardSource::Integer(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMINT as i64)
                    }
                }
            } else if carried_float.is_some_and(|carried| carried.reg == rhs) {
                builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
            } else if let Some(tag) =
                trace_value_kind_tag(numeric_reg_value_kind(known_value_kinds, rhs))
            {
                builder.ins().iconst(types::I8, i64::from(tag))
            } else {
                builder
                    .ins()
                    .load(types::I8, mem, rhs_ptr, LUA_VALUE_TT_OFFSET)
            };
            let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
            let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
            let lhs_is_int = builder.ins().icmp(IntCC::Equal, lhs_tag, int_tag);
            let lhs_is_float = builder.ins().icmp(IntCC::Equal, lhs_tag, float_tag);
            let rhs_is_int = builder.ins().icmp(IntCC::Equal, rhs_tag, int_tag);
            let rhs_is_float = builder.ins().icmp(IntCC::Equal, rhs_tag, float_tag);
            let lhs_is_numeric = builder.ins().bor(lhs_is_int, lhs_is_float);
            let rhs_is_numeric = builder.ins().bor(rhs_is_int, rhs_is_float);
            let both_numeric = builder.ins().band(lhs_is_numeric, rhs_is_numeric);
            let compare_block = builder.create_block();
            builder.def_var(hits_var, current_hits);
            builder
                .ins()
                .brif(both_numeric, compare_block, &[], fallback_block, &[]);
            builder.switch_to_block(compare_block);
            builder.seal_block(compare_block);

            let lhs_val = if let Some(carried) = carried_float.filter(|carried| carried.reg == lhs)
            {
                builder.use_var(carried.raw_var)
            } else if let Some(override_value) =
                lookup_numeric_guard_value(current_numeric_values, hoisted_numeric, lhs)
            {
                match override_value {
                    HoistedNumericGuardSource::FloatRaw(raw) => raw,
                    HoistedNumericGuardSource::Integer(value) => value,
                }
            } else {
                builder
                    .ins()
                    .load(types::I64, mem, lhs_ptr, LUA_VALUE_VALUE_OFFSET)
            };
            let rhs_val = if let Some(carried) = carried_float.filter(|carried| carried.reg == rhs)
            {
                builder.use_var(carried.raw_var)
            } else if let Some(override_value) =
                lookup_numeric_guard_value(current_numeric_values, hoisted_numeric, rhs)
            {
                match override_value {
                    HoistedNumericGuardSource::FloatRaw(raw) => raw,
                    HoistedNumericGuardSource::Integer(value) => value,
                }
            } else {
                builder
                    .ins()
                    .load(types::I64, mem, rhs_ptr, LUA_VALUE_VALUE_OFFSET)
            };
            let lhs_num = emit_numeric_tagged_value_to_float(builder, lhs_tag, lhs_val);
            let rhs_num = emit_numeric_tagged_value_to_float(builder, rhs_tag, rhs_val);
            Some(emit_numeric_compare(builder, lhs_num, rhs_num, op))
        }
        NumericIfElseCond::Truthy { reg } => {
            let reg_ptr = slot_addr(builder, abi.base_ptr, reg);
            let tag = if let Some(override_value) =
                lookup_numeric_guard_value(current_numeric_values, hoisted_numeric, reg)
            {
                match override_value {
                    HoistedNumericGuardSource::FloatRaw(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
                    }
                    HoistedNumericGuardSource::Integer(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMINT as i64)
                    }
                }
            } else if carried_float.is_some_and(|carried| carried.reg == reg) {
                builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
            } else {
                builder
                    .ins()
                    .load(types::I8, mem, reg_ptr, LUA_VALUE_TT_OFFSET)
            };
            let is_nil = builder
                .ins()
                .icmp_imm(IntCC::Equal, tag, LUA_VNIL_TAG as i64);
            let is_false = builder
                .ins()
                .icmp_imm(IntCC::Equal, tag, LUA_VFALSE_TAG as i64);
            let is_falsey = builder.ins().bor(is_nil, is_false);
            Some(builder.ins().bnot(is_falsey))
        }
    }
}

fn emit_linear_compare(
    builder: &mut FunctionBuilder<'_>,
    lhs: Value,
    rhs: Value,
    op: LinearIntGuardOp,
) -> Value {
    match op {
        LinearIntGuardOp::Eq => builder.ins().icmp(IntCC::Equal, lhs, rhs),
        LinearIntGuardOp::Lt => builder.ins().icmp(IntCC::SignedLessThan, lhs, rhs),
        LinearIntGuardOp::Le => builder.ins().icmp(IntCC::SignedLessThanOrEqual, lhs, rhs),
        LinearIntGuardOp::Gt => builder.ins().icmp(IntCC::SignedGreaterThan, lhs, rhs),
        LinearIntGuardOp::Ge => builder
            .ins()
            .icmp(IntCC::SignedGreaterThanOrEqual, lhs, rhs),
    }
}

fn emit_numeric_compare(
    builder: &mut FunctionBuilder<'_>,
    lhs: Value,
    rhs: Value,
    op: LinearIntGuardOp,
) -> Value {
    match op {
        LinearIntGuardOp::Eq => builder.ins().fcmp(FloatCC::Equal, lhs, rhs),
        LinearIntGuardOp::Lt => builder.ins().fcmp(FloatCC::LessThan, lhs, rhs),
        LinearIntGuardOp::Le => builder.ins().fcmp(FloatCC::LessThanOrEqual, lhs, rhs),
        LinearIntGuardOp::Gt => builder.ins().fcmp(FloatCC::GreaterThan, lhs, rhs),
        LinearIntGuardOp::Ge => builder.ins().fcmp(FloatCC::GreaterThanOrEqual, lhs, rhs),
    }
}

pub(super) fn emit_numeric_guard_flow(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    cond: NumericIfElseCond,
    continue_when: bool,
    continue_preset: Option<&NumericStep>,
    exit_preset: Option<&NumericStep>,
    continue_block: Block,
    exit_block: Block,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
    current_numeric_values: &mut CurrentNumericGuardValues,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: HoistedNumericGuardValues,
) -> Option<()> {
    let cond_value = emit_numeric_condition_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        cond,
        known_value_kinds,
        current_numeric_values,
        carried_float,
        hoisted_numeric,
    )?;

    let hold_block = builder.create_block();
    let fail_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    if continue_when {
        builder
            .ins()
            .brif(cond_value, hold_block, &[], fail_block, &[]);
    } else {
        builder
            .ins()
            .brif(cond_value, fail_block, &[], hold_block, &[]);
    }

    builder.switch_to_block(hold_block);
    if let Some(step) = continue_preset {
        emit_numeric_step(
            builder,
            abi,
            native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            *step,
            known_value_kinds,
            current_numeric_values,
            carried_float,
            hoisted_numeric,
        )?;
    }
    builder.def_var(hits_var, current_hits);
    builder.ins().jump(continue_block, &[]);
    builder.seal_block(hold_block);

    builder.switch_to_block(fail_block);
    if let Some(step) = exit_preset {
        let mut exit_numeric_values = current_numeric_values.clone();
        emit_numeric_step(
            builder,
            abi,
            native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            *step,
            known_value_kinds,
            &mut exit_numeric_values,
            carried_float,
            hoisted_numeric,
        )?;
    }
    builder.def_var(hits_var, current_hits);
    builder.ins().jump(exit_block, &[]);
    builder.seal_block(fail_block);
    Some(())
}
