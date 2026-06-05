/*----------------------------------------------------------------------
  Lua 5.5 VM Execution Engine

  Design Philosophy:
  1. **Slice-Based**: Code and constants accessed via `&[T]` slices with
     `noalias` guarantees — LLVM keeps slice base pointers in registers
     across function calls (raw pointers must be reloaded after `&mut` calls)
  2. **Minimal Indirection**: Use get_unchecked for stack access (no bounds checks)
  4. **CPU Register Optimization**: code, constants, pc, base, trap in CPU registers
  5. **Unsafe but Sound**: Use raw pointers with invariant guarantees for stack

  Key Invariants (maintained by caller):
  - Stack pointer valid throughout execution (no reallocation)
  - CallInfo valid and matches current frame
  - Chunk lifetime extends through execution
  - base + register < stack.len() (validated at call time)

  This leverages Rust's type system for LLVM optimization opportunities
----------------------------------------------------------------------*/

use crate::{
    Instruction, LUA_MASKCALL, LUA_MASKCOUNT, LUA_MASKLINE, LUA_MASKRET, LuaResult, LuaState,
    LuaValue, OpCode,
    gc::TablePtr,
    lua_value::{BIT_ISCOLLECTABLE, LuaProto},
    lua_vm::{
        LuaError, StkId, TmKind,
        call_info::call_status::{CIST_C, CIST_CLSRET, CIST_PENDING_FINISH},
        execute::{
            call::{poscall, precall, pretailcall},
            closure::push_closure,
            concat::{concat, try_concat_pair_utf8},
            helper::{
                bin_tm_fallback, eq_fallback, error_div_by_zero, error_global, error_mod_by_zero,
                finishget_fallback, finishset_fallback, float_for_loop, fltvalue, forprep,
                handle_pending_ops, instr_at, ivalue, k_val, lua_fmod, lua_idiv, lua_imod,
                lua_shiftl, lua_shiftr, luai_numpow, objlen, order_tm_fallback, pivalue, pk_val,
                ptonumberns, pttisinteger, return0_with_hook, return1_with_hook,
                self_shortstr_index_chain_fast, tointegerns, tonumberns, ttisfloat, ttisinteger,
                ttisstring, unary_tm_fallback,
            },
            hook::{hook_check_instruction, hook_on_call},
            metamethod::call_newindex_tm_fast,
            number::{le_num, lt_num},
            vararg::{exec_varargprep, get_vararg, get_varargs},
        },
        lua_limits::EXTRA_STACK,
    },
};

#[inline(always)]
fn init_oldpc(lua_state: &mut LuaState, pc: usize, chunk: &LuaProto) {
    if lua_state.hook_mask & LUA_MASKLINE != 0 {
        lua_state.oldpc = if pc > 0 {
            (pc - 1) as u32
        } else if chunk.is_vararg {
            0
        } else {
            u32::MAX
        };
    }
}

#[inline(always)]
fn current_trap(lua_state: &LuaState) -> bool {
    #[cfg(not(feature = "sandbox"))]
    {
        lua_state.hook_mask != 0
    }

    #[cfg(feature = "sandbox")]
    {
        lua_state.has_active_instruction_watch()
    }
}

macro_rules! op_arithI {
    ($instr:expr, $base_stk:expr, $pc:expr, $iop:expr, $fop:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let sc = $instr.get_sc();
        let v1 = $base_stk.offset(b);
        if v1.is_integer() {
            $pc += 1;
            $base_stk.offset(a).set_integer($iop(v1.ivalue(), sc));
        } else if v1.is_float() {
            $pc += 1;
            $base_stk
                .offset(a)
                .set_float($fop(v1.fltvalue(), sc as f64));
        }
    }};
}

macro_rules! op_arithf_aux {
    ($ra_stk:expr, $pc:expr, $v1_stk:expr, $v2_stk:expr, $fop:expr) => {{
        let mut n1 = 0.0;
        let mut n2 = 0.0;
        if unsafe {
            ptonumberns($v1_stk.as_const_ptr(), &mut n1)
                && ptonumberns($v2_stk.as_const_ptr(), &mut n2)
        } {
            $pc += 1;
            $ra_stk.set_float($fop(n1, n2));
        }
    }};
}

macro_rules! op_arith {
    ($instr:expr, $base_stk:expr, $pc:expr, $iop:expr, $fop:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;
        let v1 = $base_stk.offset(b);
        let v2 = $base_stk.offset(c);
        if v1.is_integer() && v2.is_integer() {
            $pc += 1;
            $base_stk
                .offset(a)
                .set_integer($iop(v1.ivalue(), v2.ivalue()));
        } else {
            op_arithf_aux!($base_stk.offset(a), $pc, v1, v2, $fop);
        }
    }};
}

macro_rules! op_arithf {
    ($instr:expr, $base_stk:expr, $pc:expr, $fop:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;
        op_arithf_aux!(
            $base_stk.offset(a),
            $pc,
            $base_stk.offset(b),
            $base_stk.offset(c),
            $fop
        );
    }};
}

macro_rules! op_arithK {
    ($instr:expr, $base_stk:expr, $pc:expr, $constants:expr, $iop:expr, $fop:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;
        let v1 = $base_stk.offset(b);
        let v2 = pk_val($constants, c);
        if v1.is_integer() && v2.is_integer() {
            $pc += 1;
            $base_stk
                .offset(a)
                .set_integer($iop(v1.ivalue(), v2.ivalue()));
        } else {
            op_arithf_aux!($base_stk.offset(a), $pc, v1, v2, $fop);
        }
    }};
}

macro_rules! op_arithfK {
    ($instr:expr, $base_stk:expr, $pc:expr, $constants:expr, $fop:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;
        op_arithf_aux!(
            $base_stk.offset(a),
            $pc,
            $base_stk.offset(b),
            pk_val($constants, c),
            $fop
        );
    }};
}

macro_rules! op_arith_check_zero {
    ($instr:expr, $lua_state:expr, $ci:expr, $base_stk:expr, $pc:expr, $iop:expr, $fop:expr, $err_fn:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;
        let v1 = $base_stk.offset(b);
        let v2 = $base_stk.offset(c);
        if v1.is_integer() && v2.is_integer() {
            let i1 = v1.ivalue();
            let i2 = v2.ivalue();
            if i2 != 0 {
                $pc += 1;
                $base_stk.offset(a).set_integer($iop(i1, i2));
            } else {
                $ci.save_pc($pc);
                return Err($err_fn($lua_state));
            }
        } else {
            op_arithf_aux!($base_stk.offset(a), $pc, v1, v2, $fop);
        }
    }};
}

macro_rules! op_arithK_check_zero {
    ($instr:expr, $lua_state:expr, $ci:expr, $base_stk:expr, $pc:expr, $constants:expr, $iop:expr, $fop:expr, $err_fn:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;
        let v1 = $base_stk.offset(b);
        let v2 = pk_val($constants, c);
        if v1.is_integer() && v2.is_integer() {
            let i1 = v1.ivalue();
            let i2 = v2.ivalue();
            if i2 != 0 {
                $pc += 1;
                $base_stk.offset(a).set_integer($iop(i1, i2));
            } else {
                $ci.save_pc($pc);
                return Err($err_fn($lua_state));
            }
        } else {
            op_arithf_aux!($base_stk.offset(a), $pc, v1, v2, $fop);
        }
    }};
}

macro_rules! op_bitwise {
    ($instr:expr, $base_stk:expr, $pc:expr, $op:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;
        let mut i1 = 0i64;
        let mut i2 = 0i64;
        if tointegerns($base_stk.offset(b).get_ref(), &mut i1)
            && tointegerns($base_stk.offset(c).get_ref(), &mut i2)
        {
            $pc += 1;
            $base_stk.offset(a).set_integer($op(i1, i2));
        }
    }};
}

macro_rules! op_bitwiseK {
    ($instr:expr, $base_stk:expr, $pc:expr, $constants:expr, $op:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;
        let mut i1 = 0i64;
        let i2 = pk_val($constants, c).ivalue();
        if tointegerns($base_stk.offset(b).get_ref(), &mut i1) {
            $pc += 1;
            $base_stk.offset(a).set_integer($op(i1, i2));
        }
    }};
}

/// Execute until call depth reaches target_depth
/// Used for protected calls (pcall) to execute only the called function
/// without affecting caller frames
///
/// NOTE: n_ccalls tracking is NOT done here (unlike the wrapper approach).
/// Instead, each recursive CALL SITE (metamethods, pcall, resume, __close)
/// increments/decrements n_ccalls around its call to lua_execute, mirroring
/// Lua 5.5's luaD_call pattern.
pub fn lua_execute(lua_state: &mut LuaState, target_depth: usize) -> LuaResult<()> {
    loop {
        let current_depth = lua_state.call_depth();
        if current_depth <= target_depth {
            return Ok(());
        }

        let frame_idx = current_depth - 1;
        let ci_ptr = lua_state.get_call_info_ptr(frame_idx);
        let mut ci = unsafe { &mut *ci_ptr };
        if ci.call_status & (CIST_C | CIST_PENDING_FINISH) != 0
            && handle_pending_ops(lua_state, ci)?
        {
            continue;
        }

        let mut base_stk = ci.base_stk;
        let mut pc = ci.pc as usize;
        let mut chunk = unsafe { &*ci.chunk_ptr };
        debug_assert!(lua_state.stack_len() >= ci.base + chunk.max_stack_size + EXTRA_STACK);

        let mut code: &[Instruction] = &chunk.code;
        let mut constants: &[LuaValue] = &chunk.constants;
        init_oldpc(lua_state, pc, chunk);

        // CALL HOOK: fire when entering a new Lua function (pc == 0)
        let mut trap = current_trap(lua_state);
        if pc == 0 && trap {
            let hook_mask = lua_state.hook_mask;
            if hook_mask & LUA_MASKCALL != 0 && lua_state.allow_hook {
                ci.save_pc(pc);
                hook_on_call(lua_state, hook_mask, ci.call_status, chunk)?;
            }
            if hook_mask & LUA_MASKCOUNT != 0 {
                lua_state.hook_count = lua_state.base_hook_count;
            }
        }

        // Lean reload after RETURN (Return0/Return1/Return).
        // We know: pc != 0 (returning to existing frame).
        // Checks: depth guard, C frame / pending finish (caller might be C frame).
        macro_rules! reload_after_return {
            () => {
                let current_depth = lua_state.call_depth();
                if current_depth <= target_depth {
                    return Ok(());
                }
                let frame_idx = current_depth - 1;
                let next_ci_ptr = lua_state.get_call_info_ptr(frame_idx);
                ci = unsafe { &mut *next_ci_ptr };
                if ci.call_status & (CIST_C | CIST_PENDING_FINISH) != 0 {
                    break;
                }
                base_stk = ci.base_stk;
                pc = ci.pc as usize;
                chunk = unsafe { &*ci.chunk_ptr };
                code = &chunk.code;
                constants = &chunk.constants;
                trap = current_trap(lua_state);
                init_oldpc(lua_state, pc, chunk);
            };
        }

        // Lean reload after CALL (entering new Lua function).
        // We know: pc == 0, it's a fresh Lua frame (no CIST_C / PENDING_FINISH).
        // Still need: hook_on_call for new function entry.
        macro_rules! reload_after_call {
            () => {
                let frame_idx = lua_state.call_depth() - 1;
                let ci_ptr = lua_state.get_call_info_ptr(frame_idx);
                ci = unsafe { &mut *ci_ptr };
                base_stk = ci.base_stk;
                pc = 0;
                chunk = unsafe { &*ci.chunk_ptr };
                code = &chunk.code;
                constants = &chunk.constants;
                trap = current_trap(lua_state);
                if trap {
                    let hook_mask = lua_state.hook_mask;
                    if hook_mask & LUA_MASKCALL != 0 && lua_state.allow_hook {
                        ci.save_pc(0);
                        hook_on_call(lua_state, hook_mask, ci.call_status, chunk)?;
                    }
                    if hook_mask & LUA_MASKCOUNT != 0 {
                        lua_state.hook_count = lua_state.base_hook_count;
                    }
                }
                init_oldpc(lua_state, 0, chunk);
            };
        }

        macro_rules! updatetrap {
            () => {
                // Always reload base_stk — the stack may have been reallocated
                // by a metamethod call or other operation since our last reload.
                base_stk = ci.base_stk;

                #[cfg(not(feature = "sandbox"))]
                {
                    trap = lua_state.hook_mask != 0;
                }

                #[cfg(feature = "sandbox")]
                {
                    trap = lua_state.has_active_instruction_watch();
                }
            };
        }

        macro_rules! savestate {
            () => {
                ci.save_pc(pc);
                lua_state.set_top_raw(ci.top as usize);
            };
        }

        // MAINLOOP: Main instruction dispatch loop
        loop {
            let instr = instr_at(code, pc); // vmfetch
            pc += 1;

            if trap {
                ci.save_pc(pc);
                trap = hook_check_instruction(lua_state, pc, chunk)?;

                base_stk = ci.base_stk;
            }

            match instr.get_opcode() {
                OpCode::Move => {
                    // R[A] := R[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    base_stk.offset(a).set(base_stk.offset(b));
                }
                OpCode::LoadI => {
                    // R[A] := sBx
                    let a = instr.get_a() as usize;
                    base_stk.offset(a).set_integer(instr.get_sbx() as i64);
                }
                OpCode::LoadF => {
                    // R[A] := (float)sBx
                    let a = instr.get_a();
                    let sbx = instr.get_sbx();
                    base_stk.offset(a as usize).set_float(sbx as f64);
                }
                OpCode::LoadK => {
                    // R[A] := K[Bx]
                    let a = instr.get_a();
                    let bx = instr.get_bx();
                    base_stk
                        .offset(a as usize)
                        .set(pk_val(constants, bx as usize));
                }
                OpCode::LoadKX => {
                    // R[A] := K[extra arg]
                    let a = instr.get_a();
                    let next_instr = instr_at(code, pc);
                    debug_assert_eq!(next_instr.get_opcode(), OpCode::ExtraArg);
                    let rb = next_instr.get_ax();
                    pc += 1;
                    base_stk
                        .offset(a as usize)
                        .set(pk_val(constants, rb as usize));
                }
                OpCode::LoadFalse => {
                    // R[A] := false
                    let a = instr.get_a();
                    base_stk.offset(a as usize).set_bool(false);
                }
                OpCode::LFalseSkip => {
                    // R[A] := false; pc++
                    let a = instr.get_a();
                    base_stk.offset(a as usize).set_bool(false);
                    pc += 1; // Skip next instruction
                }
                OpCode::LoadTrue => {
                    // R[A] := true
                    let a = instr.get_a();
                    base_stk.offset(a as usize).set_bool(true);
                }
                OpCode::LoadNil => {
                    // R[A], R[A+1], ..., R[A+B] := nil
                    let mut a = instr.get_a();
                    let mut b = instr.get_b();
                    loop {
                        base_stk.offset(a as usize).set_nil();
                        if b == 0 {
                            break;
                        }
                        b -= 1;
                        a += 1;
                    }
                }
                OpCode::GetUpval => {
                    // R[A] := UpValue[B]
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(b as usize) };
                    let src = upvalue_ptr.as_ref().data.get_v_stk_id();
                    base_stk.offset(a as usize).set(src);
                }
                OpCode::SetUpval => {
                    // UpValue[B] := R[A]
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(b as usize) };
                    let value = base_stk.offset(a as usize).get();
                    upvalue_ptr
                        .as_mut_ref()
                        .data
                        .set_value_parts(value.value, value.tt);

                    // GC barrier (only for collectable values)
                    if value.tt & BIT_ISCOLLECTABLE != 0
                        && let Some(gc_ptr) = value.as_gc_ptr()
                    {
                        lua_state.gc_barrier(upvalue_ptr, gc_ptr);
                    }
                }
                OpCode::GetTabUp => {
                    // R[A] := UpValue[B][K[C]:shortstring]
                    let a = instr.get_a();
                    let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(instr.get_b() as usize) };
                    let upval_value = upvalue_ptr.as_ref().data.get_value_ref();
                    let key = k_val(constants, instr.get_c());
                    let dest = base_stk.offset(a as usize);
                    debug_assert!(
                        key.is_short_string(),
                        "GetTabUp key must be short string for fast path"
                    );
                    if upval_value.is_table() {
                        let table = upval_value.hvalue();
                        if !trap {
                            let next_instr = instr_at(code, pc);
                            if next_instr.get_opcode() == OpCode::GetField
                                && next_instr.get_b() == a
                            {
                                let next_key = k_val(constants, next_instr.get_c());
                                debug_assert!(
                                    next_key.is_short_string(),
                                    "GetField key must be short string for fast path"
                                );

                                if let Some(outer) = table.impl_table.get_shortstr_fast(key) {
                                    if outer.is_table() {
                                        let inner_table = outer.hvalue();
                                        if inner_table.impl_table.has_hash()
                                            && inner_table
                                                .impl_table
                                                .get_shortstr_into(next_key, dest.as_ptr())
                                        {
                                            pc += 1;
                                            continue;
                                        }
                                    }

                                    base_stk.offset(a as usize).write(&outer);
                                    continue;
                                }
                            }
                        }

                        if table.impl_table.has_hash()
                            && table.impl_table.get_shortstr_into(key, dest.as_ptr())
                        {
                            continue;
                        }
                    }
                    savestate!();
                    let upval_value = *upval_value;
                    finishget_fallback(lua_state, ci, &upval_value, key, dest)?;
                    updatetrap!();
                }
                OpCode::GetTable => {
                    // GETTABLE: R[A] := R[B][R[C]]
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let c = instr.get_c();

                    let rb = base_stk.offset(b as usize);
                    let rc = base_stk.offset(c as usize);
                    let dest = base_stk.offset(a as usize);
                    if rb.is_table() {
                        let table = rb.hvalue();

                        // Hot path 1: integer key → array fast path
                        if rc.is_integer() {
                            let key = rc.ivalue();
                            if table.impl_table.fast_geti_into(key, dest.as_ptr()) {
                                continue;
                            }
                            if table.impl_table.get_int_from_hash_into(key, dest.as_ptr()) {
                                continue;
                            }
                        }
                        // Hot path 2: short string key → hash fast path (zero-copy)
                        else if rc.is_short_string()
                            && table.impl_table.has_hash()
                            && table
                                .impl_table
                                .get_shortstr_into(rc.get_ref(), dest.as_ptr())
                        {
                            continue;
                        }
                        // Cold path: other key types, hash fallback for integers
                        if let Some(val) = table.impl_table.raw_get(rc.get_ref()) {
                            dest.write(&val);
                            continue;
                        }

                        savestate!();
                        finishget_fallback(lua_state, ci, rb.get_ref(), rc.get_ref(), dest)?;
                        updatetrap!();
                        continue;
                    }

                    // Metamethod / non-table fallback
                    savestate!();
                    finishget_fallback(lua_state, ci, rb.get_ref(), rc.get_ref(), dest)?;
                    updatetrap!();
                }
                OpCode::GetI => {
                    // GETI: R[A] := R[B][C] (integer key)
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let rc = instr.get_c() as i64;
                    let rb = base_stk.offset(b as usize);
                    let dest = base_stk.offset(a as usize);
                    if rb.is_table() {
                        let table = rb.hvalue();

                        // fast_geti: try array part first
                        let found = table.impl_table.fast_geti_into(rc, dest.as_ptr());
                        if found {
                            continue;
                        }
                        // fallback: direct integer hash lookup (no float/array re-check)
                        let found = table.impl_table.get_int_from_hash_into(rc, dest.as_ptr());
                        if found {
                            continue;
                        }
                    }

                    savestate!();
                    finishget_fallback(lua_state, ci, rb.get_ref(), &LuaValue::integer(rc), dest)?;
                    updatetrap!();
                }
                OpCode::GetField => {
                    // GETFIELD: R[A] := R[B][K[C]:string]
                    let rb = base_stk.offset(instr.get_b() as usize);
                    let key = k_val(constants, instr.get_c());
                    debug_assert!(
                        key.is_short_string(),
                        "GetField key must be short string for fast path"
                    );
                    if rb.is_table() {
                        let table = rb.hvalue();
                        if table.impl_table.has_hash() {
                            let dest = base_stk.offset(instr.get_a() as usize);
                            if table.impl_table.get_shortstr_into(key, dest.as_ptr()) {
                                continue;
                            }
                        }
                    }
                    savestate!();
                    let rb = rb.get();
                    finishget_fallback(
                        lua_state,
                        ci,
                        &rb,
                        key,
                        base_stk.offset(instr.get_a() as usize),
                    )?;
                    updatetrap!();
                }
                OpCode::SetTabUp => {
                    // UpValue[A][K[B]:shortstring] := RK(C)
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let c = instr.get_c();
                    let upvalue_ptr = unsafe { *ci.upvalue_ptrs.add(a as usize) };
                    let upval_value = upvalue_ptr.as_ref().data.get_value_ref();
                    let key = k_val(constants, b);
                    debug_assert!(
                        key.is_short_string(),
                        "GetTabUp key must be short string for fast path"
                    );
                    let mut known_newindex_miss = false;
                    let mut meta = TablePtr::null();
                    if upval_value.is_table() {
                        let table = upval_value.hvalue_mut();
                        let table_ptr = upval_value
                            .as_table_ptr()
                            .expect("SetTabUp fast path requires table");
                        let gc_ptr = upval_value
                            .as_gc_ptr()
                            .expect("SetTabUp fast path requires collectable table");
                        meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let (new_key, delta, is_collectable) = if instr.get_k() {
                                let rc = *k_val(constants, c);
                                let pset_result = table.impl_table.pset_shortstr(key, rc);
                                let (new_key, delta) =
                                    table.impl_table.finish_shortstr_set(key, rc, pset_result);
                                (new_key, delta, rc.is_collectable())
                            } else {
                                let rc = base_stk.offset(c as usize);
                                let pset_result = table.impl_table.pset_shortstr(key, rc.get());
                                let (new_key, delta) = table.impl_table.finish_shortstr_set(
                                    key,
                                    rc.get(),
                                    pset_result,
                                );
                                (new_key, delta, rc.is_collectable())
                            };
                            if new_key {
                                table.invalidate_tm_cache();
                            }
                            if delta != 0 {
                                lua_state.gc_track_table_resize(table_ptr, delta);
                            }
                            if is_collectable {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        } else {
                            let rc = if instr.get_k() {
                                *k_val(constants, c)
                            } else {
                                base_stk.offset(c as usize).get()
                            };
                            if table.impl_table.set_existing_shortstr(key, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            known_newindex_miss = true;
                        }
                    }

                    let upval_value = *upval_value;
                    let rc = if instr.get_k() {
                        *k_val(constants, c)
                    } else {
                        base_stk.offset(c as usize).get()
                    };
                    savestate!();
                    if known_newindex_miss {
                        if call_newindex_tm_fast(lua_state, ci, upval_value, meta, *key, rc)? {
                            updatetrap!();
                            continue;
                        }
                        finishset_fallback(lua_state, ci, &upval_value, key, rc, true)?;
                    } else {
                        finishset_fallback(lua_state, ci, &upval_value, key, rc, false)?;
                    }
                    updatetrap!();
                }
                OpCode::SetTable => {
                    // SETTABLE: R[A][R[B]] := RK(C)
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let c = instr.get_c();
                    // Use StkId-derived raw pointers for fast table access
                    let ra = base_stk.offset(a as usize);
                    let rb = base_stk.offset(b as usize);

                    // Hot path: table + integer key in array range, no __newindex
                    // Deferred computation: table_ptr and gc barrier only when needed
                    if ra.is_table() && rb.is_integer() {
                        let table = ra.hvalue_mut();
                        let key = rb.ivalue();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            if !instr.get_k() {
                                let rc = base_stk.offset(c as usize);
                                if table.impl_table.fast_seti(key, rc.get()) {
                                    if rc.is_collectable() {
                                        lua_state.gc_barrier_back(ra.as_gc_ptr());
                                    }
                                    continue;
                                }

                                let rc = rc.get();
                                let delta = table.impl_table.set_int_slow(key, rc);
                                if delta != 0 {
                                    lua_state.gc_track_table_resize(ra.as_table_ptr(), delta);
                                }
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                                }
                                continue;
                            }

                            let rc = *k_val(constants, c);
                            if table.impl_table.fast_seti(key, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                                }
                                continue;
                            }

                            let delta = table.impl_table.set_int_slow(key, rc);
                            if delta != 0 {
                                lua_state.gc_track_table_resize(ra.as_table_ptr(), delta);
                            }
                            if rc.is_collectable() {
                                lua_state.gc_barrier_back(ra.as_gc_ptr());
                            }
                            continue;
                        } else {
                            let rc = if instr.get_k() {
                                *k_val(constants, c)
                            } else {
                                base_stk.offset(c as usize).get()
                            };
                            if table.impl_table.set_existing_int(key, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                                }
                                continue;
                            }
                            // Fall through to finishset fallback (known miss)
                            let rc = if instr.get_k() {
                                *k_val(constants, c)
                            } else {
                                base_stk.offset(c as usize).get()
                            };
                            savestate!();
                            if call_newindex_tm_fast(lua_state, ci, ra.get(), meta, rb.get(), rc)? {
                                updatetrap!();
                                continue;
                            }
                            finishset_fallback(
                                lua_state,
                                ci,
                                ra.get_ref(),
                                rb.get_ref(),
                                rc,
                                true,
                            )?;
                            updatetrap!();
                            continue;
                        }
                    }

                    // Slow path: shortstr, generic key, non-table, or metamethod
                    if ra.is_table() {
                        let table = ra.hvalue_mut();
                        let meta = table.meta_ptr();
                        if (meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()))
                            && rb.is_short_string()
                        {
                            let key = rb.get_ref();
                            let (new_key, delta, needs_barrier) = if instr.get_k() {
                                let rc = *k_val(constants, c);
                                let pset_result = table.impl_table.pset_shortstr(key, rc);
                                let (new_key, delta) =
                                    table.impl_table.finish_shortstr_set(key, rc, pset_result);
                                (new_key, delta, rc.is_collectable() || key.is_collectable())
                            } else {
                                let rc = base_stk.offset(c as usize);
                                let pset_result = table.impl_table.pset_shortstr(key, rc.get());
                                let (new_key, delta) = table.impl_table.finish_shortstr_set(
                                    key,
                                    rc.get(),
                                    pset_result,
                                );
                                (new_key, delta, (rc.is_collectable() || rb.is_collectable()))
                            };
                            if new_key {
                                table.invalidate_tm_cache();
                            }
                            if delta != 0 {
                                lua_state.gc_track_table_resize(ra.as_table_ptr(), delta);
                            }
                            if needs_barrier {
                                lua_state.gc_barrier_back(ra.as_gc_ptr());
                            }
                            continue;
                        }
                    }

                    let rc = if instr.get_k() {
                        *k_val(constants, c)
                    } else {
                        base_stk.offset(c as usize).get()
                    };
                    if ra.is_table() {
                        let table = ra.hvalue_mut();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            if !rb.is_nil() && !rb.is_integer() {
                                let (_new_key, delta) = table.impl_table.raw_set(rb.get_ref(), rc);
                                if delta != 0 {
                                    lua_state.gc_track_table_resize(ra.as_table_ptr(), delta);
                                }
                                if rc.is_collectable() || rb.is_collectable() {
                                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                                }
                                continue;
                            }
                        } else if rb.is_short_string() {
                            if table.impl_table.set_existing_shortstr(rb.get_ref(), rc) {
                                if rc.is_collectable() || rb.is_collectable() {
                                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                                }
                                continue;
                            }
                            savestate!();
                            if call_newindex_tm_fast(lua_state, ci, ra.get(), meta, rb.get(), rc)? {
                                updatetrap!();
                                continue;
                            }
                            finishset_fallback(
                                lua_state,
                                ci,
                                ra.get_ref(),
                                rb.get_ref(),
                                rc,
                                true,
                            )?;
                            updatetrap!();
                            continue;
                        }
                    }
                    savestate!();
                    finishset_fallback(lua_state, ci, ra.get_ref(), rb.get_ref(), rc, false)?;
                    updatetrap!();
                }
                OpCode::SetI => {
                    // SETI: R[A][B] := RK(C) (integer key)
                    let ra = base_stk.offset(instr.get_a() as usize);
                    let b = instr.get_b() as i64;
                    let c = instr.get_c();

                    // Hot path: table with no __newindex metamethod, key in array range
                    if ra.is_table() {
                        let table = ra.hvalue_mut();
                        // Pre-extract table/gc pointers as Copy values to break borrow chain
                        let table_ptr = ra.as_table_ptr();
                        let gc_ptr = ra.as_gc_ptr();
                        let meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            if !instr.get_k() {
                                let rc = base_stk.offset(c as usize);
                                if table.impl_table.fast_seti(b, rc.get()) {
                                    if rc.is_collectable() {
                                        lua_state.gc_barrier_back(gc_ptr);
                                    }
                                    continue;
                                }

                                let delta = table.impl_table.set_int_slow(b, rc.get());
                                if delta != 0 {
                                    lua_state.gc_track_table_resize(table_ptr, delta);
                                }
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }

                            let rc = *k_val(constants, c);
                            if table.impl_table.fast_seti(b, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }

                            let delta = table.impl_table.set_int_slow(b, rc);
                            if delta != 0 {
                                lua_state.gc_track_table_resize(table_ptr, delta);
                            }
                            if rc.is_collectable() {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        } else {
                            let rc = if instr.get_k() {
                                *k_val(constants, c)
                            } else {
                                base_stk.offset(c as usize).get()
                            };
                            if table.impl_table.set_existing_int(b, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            // Fall through to finishset fallback (known miss)
                            let rb = LuaValue::integer(b);
                            savestate!();
                            if call_newindex_tm_fast(lua_state, ci, ra.get(), meta, rb, rc)? {
                                updatetrap!();
                                continue;
                            }
                            finishset_fallback(lua_state, ci, ra.get_ref(), &rb, rc, true)?;
                            updatetrap!();
                            continue;
                        }
                    }
                    let rc = if instr.get_k() {
                        *k_val(constants, c)
                    } else {
                        base_stk.offset(c as usize).get()
                    };
                    let rb = LuaValue::integer(b);
                    savestate!();
                    finishset_fallback(lua_state, ci, ra.get_ref(), &rb, rc, false)?;
                    updatetrap!();
                }
                OpCode::SetField => {
                    // SETFIELD: R[A][K[B]:string] := RK(C)
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let c = instr.get_c();
                    let ra = base_stk.offset(a as usize);
                    let key = k_val(constants, b);
                    debug_assert!(
                        key.is_short_string(),
                        "SetField key must be short string for fast path"
                    );
                    let mut known_newindex_miss = false;
                    let mut meta = TablePtr::null();
                    if ra.is_table() {
                        let table = ra.hvalue_mut();
                        meta = table.meta_ptr();
                        if meta.is_null() || meta.as_mut_ref().data.no_tm(TmKind::NewIndex.into()) {
                            let (new_key, delta, is_collectable) = if instr.get_k() {
                                let rc = *k_val(constants, c);
                                let pset_result = table.impl_table.pset_shortstr(key, rc);
                                let (new_key, delta) =
                                    table.impl_table.finish_shortstr_set(key, rc, pset_result);
                                (new_key, delta, rc.is_collectable())
                            } else {
                                let rc = base_stk.offset(c as usize);
                                let pset_result = table.impl_table.pset_shortstr(key, rc.get());
                                let (new_key, delta) = table.impl_table.finish_shortstr_set(
                                    key,
                                    rc.get(),
                                    pset_result,
                                );
                                (new_key, delta, rc.is_collectable())
                            };
                            if new_key {
                                table.invalidate_tm_cache();
                            }
                            if delta != 0 {
                                lua_state.gc_track_table_resize(ra.as_table_ptr(), delta);
                            }
                            if is_collectable {
                                lua_state.gc_barrier_back(ra.as_gc_ptr());
                            }
                            continue;
                        } else {
                            let rc = if instr.get_k() {
                                *k_val(constants, c)
                            } else {
                                base_stk.offset(c as usize).get()
                            };
                            if table.impl_table.set_existing_shortstr(key, rc) {
                                if rc.is_collectable() {
                                    lua_state.gc_barrier_back(ra.as_gc_ptr());
                                }
                                continue;
                            }
                            known_newindex_miss = true;
                        }
                    }
                    let rc = if instr.get_k() {
                        *k_val(constants, c)
                    } else {
                        base_stk.offset(c as usize).get()
                    };
                    savestate!();
                    if known_newindex_miss {
                        if call_newindex_tm_fast(lua_state, ci, ra.get(), meta, *key, rc)? {
                            updatetrap!();
                            continue;
                        }
                        finishset_fallback(lua_state, ci, ra.get_ref(), key, rc, true)?;
                    } else {
                        finishset_fallback(lua_state, ci, ra.get_ref(), key, rc, false)?;
                    }
                    updatetrap!();
                }
                OpCode::NewTable => {
                    // R[A] := {} (new table) — table ops should be inlined
                    let a = instr.get_a();
                    let mut vb = instr.get_vb();
                    let mut vc = instr.get_vc();
                    let k = instr.get_k();

                    vb = if vb > 0 {
                        if vb > 31 { 0 } else { 1 << (vb - 1) }
                    } else {
                        0
                    };

                    if k {
                        let extra_instr = instr_at(code, pc);
                        if extra_instr.get_opcode() == OpCode::ExtraArg {
                            vc += extra_instr.get_ax() * 1024;
                        }
                    }

                    pc += 1; // skip EXTRAARG

                    let value = lua_state.create_table(vc as usize, vb as usize)?;
                    base_stk.offset(a as usize).write(&value);

                    let new_top = ci.base + a as usize + 1;
                    // ci.save_pc(pc);
                    // lua_state.set_top_raw(new_top);
                    // lua_state.check_gc()?;
                    // let frame_top = ci.top;
                    // lua_state.set_top_raw(frame_top as usize);
                    lua_state.check_gc_in_loop(pc, new_top, &mut trap);
                }
                OpCode::Self_ => {
                    // SELF: R[A+1] := R[B]; R[A] := R[B][K[C]:string]
                    let a = instr.get_a();
                    let rb = base_stk.offset(instr.get_b() as usize).get();
                    let key = k_val(constants, instr.get_c());

                    debug_assert!(
                        key.is_short_string(),
                        "Self key must be short string for fast path"
                    );
                    base_stk.offset(a as usize + 1).write(&rb);
                    // Fast path: rb is a table with hash part
                    if rb.ttistable() {
                        let table = rb.hvalue();
                        if table.impl_table.has_hash() {
                            let dest = base_stk.offset(a as usize);
                            if table.impl_table.get_shortstr_into(key, dest.as_ptr()) {
                                continue;
                            }
                        }
                        if self_shortstr_index_chain_fast(
                            lua_state,
                            &rb,
                            key,
                            base_stk.offset(a as usize),
                        ) {
                            updatetrap!();
                            continue;
                        }
                    }

                    savestate!();
                    finishget_fallback(lua_state, ci, &rb, key, base_stk.offset(a as usize))?;
                    updatetrap!();
                }
                OpCode::Add => {
                    // R[A] := R[B] + R[C]
                    op_arith!(
                        instr,
                        base_stk,
                        pc,
                        |i1: i64, i2: i64| i1.wrapping_add(i2),
                        |n1: f64, n2: f64| n1 + n2
                    );
                }
                OpCode::AddI => {
                    // R[A] := R[B] + sC
                    op_arithI!(
                        instr,
                        base_stk,
                        pc,
                        |iv1: i64, sc: i32| iv1.wrapping_add(sc as i64),
                        |nb: f64, fimm: f64| nb + fimm
                    );
                }
                OpCode::Sub => {
                    // R[A] := R[B] - R[C]
                    op_arith!(
                        instr,
                        base_stk,
                        pc,
                        |i1: i64, i2: i64| i1.wrapping_sub(i2),
                        |n1: f64, n2: f64| n1 - n2
                    );
                }
                OpCode::Mul => {
                    // R[A] := R[B] * R[C]
                    op_arith!(
                        instr,
                        base_stk,
                        pc,
                        |i1: i64, i2: i64| i1.wrapping_mul(i2),
                        |n1: f64, n2: f64| n1 * n2
                    );
                }
                OpCode::Div => {
                    // R[A] := R[B] / R[C] (float)
                    op_arithf!(instr, base_stk, pc, |n1: f64, n2: f64| n1 / n2);
                }
                OpCode::IDiv => {
                    // R[A] := R[B] // R[C] (floor division)
                    op_arith_check_zero!(
                        instr,
                        lua_state,
                        ci,
                        base_stk,
                        pc,
                        |i1: i64, i2: i64| lua_idiv(i1, i2),
                        |n1: f64, n2: f64| (n1 / n2).floor(),
                        error_div_by_zero
                    );
                }
                OpCode::Mod => {
                    // R[A] := R[B] % R[C]
                    op_arith_check_zero!(
                        instr,
                        lua_state,
                        ci,
                        base_stk,
                        pc,
                        |i1: i64, i2: i64| lua_imod(i1, i2),
                        |n1: f64, n2: f64| lua_fmod(n1, n2),
                        error_mod_by_zero
                    );
                }
                OpCode::Pow => {
                    // R[A] := R[B] ^ R[C]
                    op_arithf!(instr, base_stk, pc, |n1: f64, n2: f64| luai_numpow(n1, n2));
                }
                OpCode::AddK => {
                    // R[A] := R[B] + K[C]
                    op_arithK!(
                        instr,
                        base_stk,
                        pc,
                        constants,
                        |i1: i64, i2: i64| i1.wrapping_add(i2),
                        |n1: f64, n2: f64| n1 + n2
                    );
                }
                OpCode::SubK => {
                    // R[A] := R[B] - K[C]
                    op_arithK!(
                        instr,
                        base_stk,
                        pc,
                        constants,
                        |i1: i64, i2: i64| i1.wrapping_sub(i2),
                        |n1: f64, n2: f64| n1 - n2
                    );
                }
                OpCode::MulK => {
                    // R[A] := R[B] * K[C]
                    op_arithK!(
                        instr,
                        base_stk,
                        pc,
                        constants,
                        |i1: i64, i2: i64| i1.wrapping_mul(i2),
                        |n1: f64, n2: f64| n1 * n2
                    );
                }
                OpCode::ModK => {
                    // R[A] := R[B] % K[C]
                    op_arithK_check_zero!(
                        instr,
                        lua_state,
                        ci,
                        base_stk,
                        pc,
                        constants,
                        |i1: i64, i2: i64| lua_imod(i1, i2),
                        |n1: f64, n2: f64| lua_fmod(n1, n2),
                        error_mod_by_zero
                    );
                }
                OpCode::PowK => {
                    // R[A] := R[B] ^ K[C] (always float)
                    op_arithfK!(instr, base_stk, pc, constants, |n1: f64, n2: f64| {
                        luai_numpow(n1, n2)
                    });
                }
                OpCode::DivK => {
                    // R[A] := R[B] / K[C] (float division)
                    op_arithfK!(instr, base_stk, pc, constants, |n1: f64, n2: f64| n1 / n2);
                }
                OpCode::IDivK => {
                    // R[A] := R[B] // K[C] (floor division)
                    op_arithK_check_zero!(
                        instr,
                        lua_state,
                        ci,
                        base_stk,
                        pc,
                        constants,
                        |i1: i64, i2: i64| lua_idiv(i1, i2),
                        |n1: f64, n2: f64| (n1 / n2).floor(),
                        error_div_by_zero
                    );
                }
                OpCode::BAndK => {
                    // R[A] := R[B] & K[C]
                    op_bitwiseK!(instr, base_stk, pc, constants, |i1: i64, i2: i64| i1 & i2);
                }
                OpCode::BOrK => {
                    // R[A] := R[B] | K[C]
                    op_bitwiseK!(instr, base_stk, pc, constants, |i1: i64, i2: i64| i1 | i2);
                }
                OpCode::BXorK => {
                    // R[A] := R[B] ^ K[C] (bitwise xor)
                    op_bitwiseK!(instr, base_stk, pc, constants, |i1: i64, i2: i64| i1 ^ i2);
                }
                OpCode::BAnd => {
                    // R[A] := R[B] & R[C]
                    op_bitwise!(instr, base_stk, pc, |i1: i64, i2: i64| i1 & i2);
                }
                OpCode::BOr => {
                    // R[A] := R[B] | R[C]
                    op_bitwise!(instr, base_stk, pc, |i1: i64, i2: i64| i1 | i2);
                }
                OpCode::BXor => {
                    // R[A] := R[B] ^ R[C]
                    op_bitwise!(instr, base_stk, pc, |i1: i64, i2: i64| i1 ^ i2);
                }
                OpCode::Shl => {
                    // R[A] := R[B] << R[C]
                    op_bitwise!(instr, base_stk, pc, |i1: i64, i2: i64| lua_shiftl(i1, i2));
                }
                OpCode::Shr => {
                    // R[A] := R[B] >> R[C]
                    op_bitwise!(instr, base_stk, pc, |i1: i64, i2: i64| lua_shiftr(i1, i2));
                }
                OpCode::ShlI => {
                    // R[A] := sC << R[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc();

                    let rb = base_stk.offset(b);

                    let mut ib = 0i64;
                    if tointegerns(rb.get_ref(), &mut ib) {
                        pc += 1;
                        base_stk.offset(a).set_integer(lua_shiftl(ic as i64, ib));
                    }
                    // else: metamethod
                }
                OpCode::ShrI => {
                    // R[A] := R[B] >> sC
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc();

                    let rb = base_stk.offset(b);

                    let mut ib = 0i64;
                    if tointegerns(rb.get_ref(), &mut ib) {
                        pc += 1;
                        base_stk.offset(a).set_integer(lua_shiftr(ib, ic as i64));
                    }
                    // else: metamethod
                }
                OpCode::MmBin => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let ra = base_stk.offset(a).get();
                    let rb = base_stk.offset(b).get();
                    let pi = instr_at(code, pc - 2);
                    let result_reg = (ci.base + pi.get_a() as usize) as u32;

                    let tm = TmKind::from_u8(instr.get_c() as u8);

                    savestate!();
                    bin_tm_fallback(lua_state, ci, ra, rb, result_reg, a as u32, b as u32, tm)?;
                    updatetrap!();
                }
                OpCode::MmBinI => {
                    let a = instr.get_a() as usize;
                    let imm = instr.get_sb();
                    let flip = instr.get_k();

                    let ra = base_stk.offset(a).get();
                    let pi = instr_at(code, pc - 2);
                    let result_reg = (ci.base + pi.get_a() as usize) as u32;

                    let tm = TmKind::from_u8(instr.get_c() as u8);
                    let rb = LuaValue::integer(imm as i64);
                    let r = if flip { (rb, ra) } else { (ra, rb) };
                    savestate!();
                    bin_tm_fallback(lua_state, ci, r.0, r.1, result_reg, a as u32, a as u32, tm)?;
                    updatetrap!();
                }
                OpCode::MmBinK => {
                    let a = instr.get_a();
                    let ra = base_stk.offset(a as usize).get();
                    let pi = instr_at(code, pc - 2);
                    let imm = *k_val(constants, instr.get_b());
                    let tm = TmKind::from_u8(instr.get_c() as u8);
                    let flip = instr.get_k();
                    let result_reg = (ci.base + pi.get_a() as usize) as u32;

                    let a_reg = instr.get_a();
                    savestate!();
                    let r = if flip { (imm, ra) } else { (ra, imm) };
                    bin_tm_fallback(lua_state, ci, r.0, r.1, result_reg, a_reg, a_reg, tm)?;
                    updatetrap!();
                }
                OpCode::Unm => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = base_stk.offset(b as usize).get();

                    if ttisinteger(&rb) {
                        let ib = ivalue(&rb);
                        base_stk.offset(a as usize).set_integer(ib.wrapping_neg());
                    } else {
                        let mut nb = 0.0;
                        if tonumberns(&rb, &mut nb) {
                            base_stk.offset(a as usize).set_float(-nb);
                        } else {
                            savestate!();
                            unary_tm_fallback(
                                lua_state,
                                ci,
                                rb,
                                ci.base + a as usize,
                                TmKind::Unm,
                            )?;
                            updatetrap!();
                        }
                    }
                }
                OpCode::BNot => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = base_stk.offset(b as usize).get();

                    let mut ib = 0i64;
                    if tointegerns(&rb, &mut ib) {
                        base_stk.offset(a as usize).set_integer(!ib);
                    } else {
                        savestate!();
                        unary_tm_fallback(lua_state, ci, rb, ci.base + a as usize, TmKind::Bnot)?;
                        updatetrap!();
                    }
                }
                OpCode::Not => {
                    // R[A] := not R[B]
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let rb = base_stk.offset(b as usize);
                    if rb.is_false_or_nil() {
                        base_stk.offset(a as usize).set_bool(true);
                    } else {
                        base_stk.offset(a as usize).set_bool(false);
                    }
                }
                OpCode::Len => {
                    // HOT PATH: inline table length for no-metatable case
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let rb = base_stk.offset(b as usize).get();
                    savestate!();
                    objlen(lua_state, ci, base_stk.offset(a as usize), rb)?;
                    updatetrap!();
                }
                OpCode::Concat => {
                    let a = instr.get_a();
                    let n = instr.get_b();

                    if n == 2 {
                        let concat_top = ci.base + (a + n) as usize;
                        lua_state.set_top_raw(concat_top);
                        let left = base_stk.offset(a as usize).get();
                        let right = base_stk.offset(a as usize + 1).get();
                        ci.save_pc(pc);

                        if let Some(result) = try_concat_pair_utf8(lua_state, left, right)? {
                            base_stk.offset(a as usize).write(&result);
                            lua_state.set_top_raw(concat_top - 1);
                            updatetrap!();

                            let top = lua_state.get_top();
                            lua_state.check_gc_in_loop(pc, top, &mut trap);
                            continue;
                        }
                    }

                    let concat_top = ci.base + (a + n) as usize;
                    lua_state.set_top_raw(concat_top);

                    // ProtectNT
                    ci.save_pc(pc);
                    match concat(lua_state, n as usize) {
                        Ok(()) => {}
                        Err(LuaError::Yield) => {
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                    updatetrap!();

                    let top = lua_state.get_top();
                    lua_state.check_gc_in_loop(pc, top, &mut trap);
                }
                OpCode::Close => {
                    let a = instr.get_a();
                    let close_from = ci.base + a as usize;

                    ci.save_pc(pc);
                    match lua_state.close_all(close_from) {
                        Ok(()) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.pc -= 1;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::Tbc => {
                    // Mark variable as to-be-closed
                    let a = instr.get_a();
                    ci.save_pc(pc); // save PC so get_local_var_name finds the variable name
                    lua_state.mark_tbc(ci.base + a as usize)?;
                }
                OpCode::Jmp => {
                    let sj = instr.get_sj();
                    pc = (pc as isize + sj as isize) as usize;
                    updatetrap!();
                }
                OpCode::Eq => {
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let ra = base_stk.offset(a as usize).get();
                    let rb = base_stk.offset(b as usize).get();
                    savestate!();
                    let cond = eq_fallback(lua_state, ci, ra, rb)?;
                    updatetrap!();
                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = instr_at(code, pc);
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::Lt => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let cond = {
                        let ra = base_stk.offset(a as usize).get_ref();
                        let rb = base_stk.offset(b as usize).get_ref();

                        if ttisinteger(ra) && ttisinteger(rb) {
                            ivalue(ra) < ivalue(rb)
                        } else if ra.is_number() && rb.is_number() {
                            lt_num(ra, rb)
                        } else if ttisstring(ra) && ttisstring(rb) {
                            let sa = ra.as_bytes();
                            let sb = rb.as_bytes();

                            if let (Some(sa), Some(sb)) = (sa, sb) {
                                sa < sb
                            } else {
                                false
                            }
                        } else {
                            let va = *ra;
                            let vb = *rb;
                            savestate!();
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Lt)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = instr_at(code, pc);
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::Le => {
                    let a = instr.get_a();
                    let b = instr.get_b();

                    let cond = {
                        let ra = base_stk.offset(a as usize).get_ref();
                        let rb = base_stk.offset(b as usize).get_ref();

                        if ttisinteger(ra) && ttisinteger(rb) {
                            ivalue(ra) <= ivalue(rb)
                        } else if ra.is_number() && rb.is_number() {
                            le_num(ra, rb)
                        } else if ttisstring(ra) && ttisstring(rb) {
                            let sa = ra.as_bytes();
                            let sb = rb.as_bytes();

                            if let (Some(sa), Some(sb)) = (sa, sb) {
                                sa <= sb
                            } else {
                                false
                            }
                        } else {
                            let va = *ra;
                            let vb = *rb;
                            savestate!();
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Le)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = instr_at(code, pc);
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::EqK => {
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let k = instr.get_k();

                    let ra = base_stk.offset(a as usize).get_ref();
                    let rb = k_val(constants, b);
                    // Raw equality (no metamethods for constants)
                    let cond = ra == rb;
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = instr_at(code, pc);
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::EqI => {
                    let a = instr.get_a();
                    let im = instr.get_sb();
                    let ra = base_stk.offset(a as usize).get_ref();
                    let cond = if ttisinteger(ra) {
                        ivalue(ra) == im as i64
                    } else if ttisfloat(ra) {
                        fltvalue(ra) == im as f64
                    } else {
                        false
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = instr_at(code, pc);
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::LtI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = {
                        let ra = base_stk.offset(a);

                        if ra.is_integer() {
                            ra.ivalue() < im as i64
                        } else if ra.is_float() {
                            ra.fltvalue() < im as f64
                        } else {
                            let va = ra.get();
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Lt)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = instr_at(code, pc);
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::LeI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = {
                        let ra = base_stk.offset(a);

                        if ra.is_integer() {
                            ra.ivalue() <= im as i64
                        } else if ra.is_float() {
                            ra.fltvalue() <= im as f64
                        } else {
                            let va = ra.get();
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            let result = order_tm_fallback(lua_state, ci, va, vb, TmKind::Le)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = instr_at(code, pc);
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::GtI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = {
                        let ra = base_stk.offset(a);

                        if ra.is_integer() {
                            ra.ivalue() > im as i64
                        } else if ra.is_float() {
                            ra.fltvalue() > im as f64
                        } else {
                            let va = ra.get();
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            // GtI: a > b ≡ b < a → swap args, use Lt
                            let result = order_tm_fallback(lua_state, ci, vb, va, TmKind::Lt)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = instr_at(code, pc);
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::GeI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();

                    let cond = {
                        let ra = base_stk.offset(a);

                        if ra.is_integer() {
                            ra.ivalue() >= im as i64
                        } else if ra.is_float() {
                            ra.fltvalue() >= im as f64
                        } else {
                            let va = ra.get();
                            let isf = instr.get_c() != 0;
                            let vb = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            savestate!();
                            // GeI: a >= b ≡ b <= a → swap args, use Le
                            let result = order_tm_fallback(lua_state, ci, vb, va, TmKind::Le)?;
                            updatetrap!();
                            result
                        }
                    };

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = instr_at(code, pc);
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::Test => {
                    let a = instr.get_a();
                    let ra = base_stk.offset(a as usize).get_ref();
                    // l_isfalse: nil or false => truthy = !nil && !false
                    let cond = !ra.is_nil() && !ra.ttisfalse();

                    let k = instr.get_k();
                    if cond != k {
                        pc += 1;
                    } else {
                        let jmp = instr_at(code, pc);
                        pc = ((pc + 1) as isize + jmp.get_sj() as isize) as usize;
                        updatetrap!();
                    }
                }
                OpCode::TestSet => {
                    // if (l_isfalse(R[B]) == k) then pc++ else R[A] := R[B]; donextjump
                    let a = instr.get_a();
                    let b = instr.get_b();
                    let k = instr.get_k();

                    let rb = base_stk.offset(b as usize).get();
                    let cond = rb.is_nil() || rb.ttisfalse();
                    if cond == k {
                        pc += 1; // Condition failed - skip next instruction (JMP)
                    } else {
                        // Condition succeeded - copy value and EXECUTE next instruction (must be JMP)
                        base_stk.offset(a as usize).write(&rb);
                        // donextjump: fetch and execute next JMP instruction
                        let next_instr = instr_at(code, pc);
                        debug_assert!(next_instr.get_opcode() == OpCode::Jmp);
                        pc += 1; // Move past the JMP instruction
                        let sj = next_instr.get_sj();
                        pc = (pc as isize + sj as isize) as usize; // Execute the jump
                        updatetrap!();
                    }
                }
                OpCode::Call => {
                    let a = instr.get_a();
                    let b = instr.get_b() as usize;
                    let nresults = instr.get_c() as i32 - 1;
                    let func_idx = ci.base + a as usize;
                    let nargs = if b != 0 {
                        lua_state.set_top_raw(func_idx + b);
                        b - 1
                    } else {
                        lua_state.get_top() - func_idx - 1
                    };

                    // Fast path: peek at func to inline the exact-match Lua call.
                    // Avoids the flush→precall→reload round-trip through CallInfo.
                    let func = unsafe { *lua_state.stack().get_unchecked(func_idx) };
                    if func.is_lua_function() {
                        // Extract raw data before borrowing issues
                        let (param_count, max_stack_size, chunk_ptr, new_upvalue_ptrs) = {
                            let lf = func.as_lua_function().unwrap();
                            let c = lf.chunk();
                            (
                                c.param_count,
                                c.max_stack_size,
                                c as *const LuaProto,
                                lf.upvalues().as_ptr(),
                            )
                        };
                        let new_base = func_idx + 1;
                        if nargs == param_count
                            && lua_state.try_push_lua_frame_exact(
                                new_base,
                                nresults,
                                max_stack_size,
                                chunk_ptr,
                                new_upvalue_ptrs,
                            )?
                        {
                            // Save caller state to CallInfo
                            ci.save_pc(pc);
                            // Set locals directly — no CallInfo read-back needed
                            chunk = unsafe { &*chunk_ptr };
                            base_stk =
                                StkId::from_stack(lua_state.stack_mut().as_mut_ptr(), new_base);
                            pc = 0;
                            code = &chunk.code;
                            constants = &chunk.constants;
                            let frame_idx = lua_state.call_depth() - 1;
                            let ci_ptr = lua_state.get_call_info_ptr(frame_idx);
                            ci = unsafe { &mut *ci_ptr };
                            trap = current_trap(lua_state);
                            if trap {
                                let hook_mask = lua_state.hook_mask;
                                if hook_mask & LUA_MASKCALL != 0 && lua_state.allow_hook {
                                    ci.save_pc(0);
                                    hook_on_call(lua_state, hook_mask, ci.call_status, chunk)?;
                                }
                                if hook_mask & LUA_MASKCOUNT != 0 {
                                    lua_state.hook_count = lua_state.base_hook_count;
                                }
                            }
                            init_oldpc(lua_state, 0, chunk);
                            continue;
                        }
                        // Exact match failed (e.g. stack overflow), fall through
                        ci.save_pc(pc);
                        lua_state.push_lua_frame(
                            new_base,
                            nargs,
                            nresults,
                            param_count,
                            max_stack_size,
                            chunk_ptr,
                            new_upvalue_ptrs,
                        )?;
                        reload_after_call!();
                        continue;
                    }

                    // Generic path: C function or metamethod
                    ci.save_pc(pc);
                    if precall(lua_state, func_idx, nargs, nresults)? {
                        reload_after_call!();
                        continue;
                    }

                    // C call completed
                    if lua_state.hook_mask & LUA_MASKLINE != 0 {
                        lua_state.oldpc = (pc - 1) as u32;
                    }
                    updatetrap!();
                }
                OpCode::TailCall => {
                    let a = instr.get_a();
                    let mut b = instr.get_b() as usize;
                    let func_idx = ci.base + a as usize;
                    // let nparams1 = instr.get_c() as usize;
                    if b != 0 {
                        lua_state.set_top_raw(func_idx + b);
                    } else {
                        b = lua_state.get_top() - func_idx;
                    }
                    ci.save_pc(pc);
                    if instr.get_k() {
                        lua_state.close_upvalues(ci.base);
                    }
                    if pretailcall(lua_state, func_idx, b)? {
                        // Lua tail call: reload callee frame, continue dispatch directly
                        reload_after_call!();
                        continue;
                    }

                    // C tail call completed
                    if lua_state.hook_mask & LUA_MASKLINE != 0 {
                        lua_state.oldpc = (pc - 1) as u32;
                    }
                    updatetrap!();
                }
                OpCode::Return => {
                    // return R[A], ..., R[A+B-2]   (lvm.c:1763-1783)
                    let a_pos = ci.base + instr.get_a() as usize;
                    let mut n;

                    // Check if resuming after a yield inside __close during return
                    if ci.call_status & CIST_CLSRET != 0 {
                        // Resuming from yield-in-close: use saved nres and skip close_all
                        // (close_all already ran; remaining TBCs were closed on resume)
                        n = ci.saved_nres();
                        ci.call_status &= !CIST_CLSRET;

                        // Save pc first so re-yield points to RETURN again
                        ci.save_pc(pc);

                        // Continue closing remaining TBC variables (if any)
                        match lua_state.close_all(ci.base) {
                            Ok(()) => {
                                #[cfg(not(feature = "sandbox"))]
                                {
                                    trap = lua_state.hook_mask != 0;
                                }

                                #[cfg(feature = "sandbox")]
                                {
                                    trap = lua_state.has_active_instruction_watch();
                                }
                            }
                            Err(LuaError::Yield) => {
                                ci.call_status |= CIST_CLSRET;
                                ci.save_pc(pc - 1);
                                return Err(LuaError::Yield);
                            }
                            Err(e) => return Err(e),
                        }
                    } else {
                        n = instr.get_b() as i32 - 1;
                        if n < 0 {
                            n = (lua_state.get_top() - a_pos) as i32;
                        }

                        ci.save_pc(pc);
                        if instr.get_k() {
                            // May have open upvalues / TBC variables
                            ci.set_saved_nres(n);
                            let ci_top = ci.top as usize;
                            if lua_state.get_top() < ci_top {
                                lua_state.set_top_raw(ci_top);
                            }
                            match lua_state.close_all(ci.base) {
                                Ok(()) => {
                                    #[cfg(not(feature = "sandbox"))]
                                    {
                                        trap = lua_state.hook_mask != 0;
                                    }

                                    #[cfg(feature = "sandbox")]
                                    {
                                        trap = lua_state.has_active_instruction_watch();
                                    }
                                }
                                Err(LuaError::Yield) => {
                                    ci.call_status |= CIST_CLSRET;
                                    ci.save_pc(pc - 1);
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    }

                    lua_state.set_top_raw(a_pos + n as usize);
                    ci.save_pc(pc);
                    poscall(lua_state, n as usize, pc)?;
                    // Reload caller frame and continue dispatch (avoid outer loop roundtrip)
                    reload_after_return!();
                    continue;
                }
                OpCode::Return0 => {
                    // return (no values)
                    if lua_state.hook_mask & (LUA_MASKRET | LUA_MASKLINE) != 0 {
                        ci.save_pc(pc);
                        return0_with_hook(lua_state, ci.base + instr.get_a() as usize, pc)?;
                        break;
                    }

                    // Inlined fast path: no hook, no moveresults overhead
                    // Follows C Lua OP_RETURN0: L->ci = ci->previous; then goto returning
                    let nresults = ci.nresults();
                    let res = ci.base - ci.func_offset as usize;
                    lua_state.pop_call_frame();
                    lua_state.set_top_raw(res);
                    // nil-fill if caller wanted results
                    if nresults > 0 {
                        unsafe {
                            let sp = lua_state.stack_mut().as_mut_ptr();
                            for i in 0..nresults as usize {
                                *sp.add(res + i) = LuaValue::nil();
                            }
                        }
                        lua_state.set_top_raw(res + nresults as usize);
                    }
                    // Reload caller frame and continue dispatch (avoid outer loop roundtrip)
                    reload_after_return!();
                    continue;
                }
                OpCode::Return1 => {
                    // return R[A]  (single value)
                    if lua_state.hook_mask & (LUA_MASKRET | LUA_MASKLINE) != 0 {
                        ci.save_pc(pc);
                        return1_with_hook(lua_state, ci.base + instr.get_a() as usize, pc)?;
                        break;
                    }

                    // Inlined fast path — raw pointer for single copy
                    // Follows C Lua OP_RETURN1: L->ci = ci->previous; setobjs2s; then goto returning
                    let nresults = ci.nresults();
                    let res = ci.base - ci.func_offset as usize;
                    lua_state.pop_call_frame();
                    if nresults == 0 {
                        // Caller wants no results
                        lua_state.set_top_raw(res);
                    } else {
                        // Copy the single result value using StkId
                        let val = base_stk.offset(instr.get_a() as usize).get();
                        unsafe {
                            let sp = lua_state.stack_mut().as_mut_ptr();
                            *sp.add(res) = val;
                        }
                        lua_state.set_top_raw(res + 1);
                        // nil-fill if caller wanted more than 1
                        if nresults > 1 {
                            unsafe {
                                let sp = lua_state.stack_mut().as_mut_ptr();
                                for i in 1..nresults as usize {
                                    *sp.add(res + i) = LuaValue::nil();
                                }
                            }
                            lua_state.set_top_raw(res + nresults as usize);
                        }
                    }
                    // Reload caller frame and continue dispatch (avoid outer loop roundtrip)
                    reload_after_return!();
                    continue;
                }
                OpCode::ForLoop => {
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    unsafe {
                        let ra = base_stk.offset(a).as_ptr();
                        // Check if integer loop (tag of step at ra+1)
                        if pttisinteger(ra.add(1)) {
                            // Integer loop (most common for numeric loops)
                            // ra: counter (count of iterations left)
                            // ra+1: step
                            // ra+2: control variable (idx)
                            let count = pivalue(ra) as u64;
                            if count > 0 {
                                // More iterations
                                let step = pivalue(ra.add(1));
                                let idx = pivalue(ra.add(2));

                                // Update counter (decrement) - only write value, tag unchanged
                                (*ra).value.i = count as i64 - 1;
                                // Update control variable: idx += step - only write value
                                (*ra.add(2)).value.i = idx.wrapping_add(step);

                                // Jump back
                                pc -= bx;
                            }
                            // else: counter expired, exit loop
                        } else if float_for_loop(lua_state, ci.base + a) {
                            // Float loop with non-integer step
                            // Jump back if loop continues
                            pc -= bx;
                        }
                    }

                    updatetrap!();
                }
                OpCode::ForPrep => {
                    let a = instr.get_a();
                    savestate!();
                    if forprep(lua_state, ci.base + a as usize)? {
                        // Skip the loop body: jump forward past FORLOOP
                        pc += instr.get_bx() as usize + 1;
                    }
                    updatetrap!();
                }
                OpCode::TForPrep => {
                    // Prepare generic for loop — inline (for loop related)
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let ra = ci.base + a;

                    // Swap control and closing variables
                    lua_state.stack_mut().swap(ra + 3, ra + 2);

                    // Mark ra+2 as to-be-closed if not nil (regardless — mark_tbc handles it)
                    lua_state.mark_tbc(ra + 2)?;

                    pc += bx;
                }
                OpCode::TForCall => {
                    // Generic for loop call — matches C Lua's OP_TFORCALL.
                    // Copy iterator+state+control to ra+3..ra+5, then precall.
                    let a = instr.get_a() as usize;
                    let c = instr.get_c() as usize;
                    let func_idx = ci.base + a + 3;
                    base_stk.offset(a + 5).set(base_stk.offset(a + 3));
                    base_stk.offset(a + 4).set(base_stk.offset(a + 1));
                    base_stk.offset(a + 3).set(base_stk.offset(a));
                    lua_state.set_top_raw(func_idx + 3); // func + 2 args
                    ci.save_pc(pc);
                    if precall(lua_state, func_idx, 2, c as i32)? {
                        // Lua call in generic for: reload callee frame, continue dispatch directly
                        reload_after_call!();
                        continue;
                    }

                    if lua_state.hook_mask & LUA_MASKLINE != 0 {
                        lua_state.oldpc = (pc - 1) as u32;
                    }
                    updatetrap!();
                }
                OpCode::TForLoop => {
                    // Generic for loop test
                    // If ra+3 (control variable) != nil then continue loop (jump back)
                    if !base_stk
                        .offset(instr.get_a() as usize + 3)
                        .get_ref()
                        .is_nil()
                    {
                        // Continue loop: jump back
                        pc -= instr.get_bx() as usize;
                    }
                    // else: exit loop (control variable is nil)
                }
                OpCode::SetList => {
                    let a = instr.get_a();
                    let mut n = instr.get_vb() as usize;
                    let stack_idx = instr.get_vc() as usize;
                    let mut last = stack_idx;
                    let a_pos = ci.base + a as usize;
                    if n == 0 {
                        n = lua_state.get_top() - a_pos - 1; // adjust n based on top if vb=0
                    } else {
                        lua_state.set_top_raw(ci.top as usize);
                    }
                    last += n;
                    if instr.get_k() {
                        let next_instr = instr_at(code, pc);
                        debug_assert!(next_instr.get_opcode() == OpCode::ExtraArg);
                        pc += 1; // Consume EXTRAARG
                        let extra = next_instr.get_ax() as usize;
                        // Add extra to starting index
                        last += extra * (1 << Instruction::SIZE_V_C);
                    }
                    let ra = base_stk.offset(a as usize).get();
                    let h = ra.hvalue_mut();
                    if last > h.impl_table.asize as usize {
                        h.impl_table.resize_array(last as u32);
                    }

                    let impl_table = &mut h.impl_table;
                    let stack_base = lua_state.stack().as_ptr();
                    let mut is_collectable = false;
                    // Port of C Lua's SETLIST loop (lvm.c):
                    //   for (; n > 0; n--) { val = s2v(ra+n); obj2arr(h, last, val); last--; }
                    // Reads n values from stack[ra+n..ra+1], writes to table[last..last-n+1]
                    let mut write_idx = last;
                    for i in (1..=n).rev() {
                        let val = unsafe { *stack_base.add(a_pos + i) };
                        if val.iscollectable() {
                            is_collectable = true;
                        }

                        impl_table.write_array(write_idx as i64, val);
                        write_idx -= 1;
                    }

                    if is_collectable {
                        lua_state.gc_barrier_back(
                            ra.as_gc_ptr()
                                .expect("SetList fast path requires collectable table"),
                        );
                    }
                }
                OpCode::Closure => {
                    let a = instr.get_a() as usize;
                    let proto_idx = instr.get_bx() as usize;
                    savestate!();
                    let upvalue_ptrs =
                        unsafe { std::slice::from_raw_parts(ci.upvalue_ptrs, chunk.upvalue_count) };
                    push_closure(lua_state, ci.base, a, proto_idx, chunk, upvalue_ptrs)?;

                    lua_state.check_gc_in_loop(pc, ci.base + a + 1, &mut trap);
                }
                OpCode::Vararg => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let n = instr.get_c() as i32 - 1;
                    let vatab = if instr.get_k() { b as i32 } else { -1 };

                    savestate!();
                    match get_varargs(lua_state, ci.base, a, b, vatab, n, chunk) {
                        Ok(()) => {
                            updatetrap!();
                        }
                        Err(LuaError::Yield) => {
                            ci.call_status |= CIST_PENDING_FINISH;
                            ci.save_pc(pc);
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::GetVarg => {
                    let a = ci.base + instr.get_a() as usize;
                    let c = ci.base + instr.get_c() as usize;
                    get_vararg(lua_state, ci.base, a, c)?;
                }
                OpCode::ErrNNil => {
                    let a = instr.get_a();
                    let ra = base_stk.offset(a as usize).get_ref();

                    if !ra.is_nil() {
                        let bx = instr.get_bx() as usize;
                        let global_name = if bx > 0 && bx - 1 < constants.len() {
                            if let Some(s) = constants[bx - 1].as_str() {
                                s.to_string()
                            } else {
                                "?".to_string()
                            }
                        } else {
                            "?".to_string()
                        };

                        savestate!();
                        return Err(error_global(lua_state, &global_name));
                    }
                }
                OpCode::VarargPrep => {
                    ci.save_pc(pc);
                    let mut base = ci.base;
                    exec_varargprep(lua_state, chunk, &mut base)?;
                    // Re-sync base_stk after potential base change
                    base_stk = StkId::from_stack(lua_state.stack_mut().as_mut_ptr(), base);

                    // After varargprep, hook call if hooks are active
                    let hook_mask = lua_state.hook_mask;
                    if hook_mask != 0 {
                        ci.save_pc(pc);
                        hook_on_call(lua_state, hook_mask, ci.call_status, chunk)?;

                        if hook_mask & LUA_MASKLINE != 0 {
                            lua_state.oldpc = u32::MAX; // force line event on next instruction
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}
