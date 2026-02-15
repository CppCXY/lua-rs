/*----------------------------------------------------------------------
  Lua 5.5 VM Execution Engine - Pointer-Based High-Performance Implementation

  Design Philosophy (Lua 5.5 Style):
  1. **Pointer-Based**: Direct pointer manipulation like Lua C (avoids borrow checker)
  2. **Minimal Indirection**: Cache pointers to stack, constants, code in locals
  3. **No Allocation in Loop**: All errors via lua_state.error(), no String construction
  4. **CPU Register Optimization**: base, pc, stack_ptr kept in CPU registers
  5. **Unsafe but Sound**: Use raw pointers with invariant guarantees

  Key Invariants (maintained by caller):
  - Stack pointer valid throughout execution (no reallocation)
  - CallInfo valid and matches current frame
  - Chunk lifetime extends through execution
  - base + register < stack.len() (validated at call time)

  This matches Lua's lvm.c design where everything is pointer-based
----------------------------------------------------------------------*/

pub mod call;
mod closure_handler;
mod cold;
mod concat;
pub(crate) mod helper;
mod metamethod;
mod return_handler;

// Extracted opcode modules to reduce main loop size
mod closure_vararg_ops;
mod comparison_ops;
mod table_ops;

use call::FrameAction;

use crate::{
    GcTable,
    lua_value::{LUA_VFALSE, LUA_VTABLE, LuaValue},
    lua_vm::{
        LuaResult, LuaState, OpCode,
        call_info::call_status::{CIST_C, CIST_PENDING_FINISH},
        execute::{
            closure_handler::handle_closure,
            cold::{
                handle_close, handle_errnil, handle_forprep_float, handle_getvarg, handle_len,
                handle_loadkx,
            },
            concat::handle_concat,
            helper::{
                chgfltvalue, chgivalue, fltvalue, handle_pending_ops, ivalue, lua_idiv, lua_imod,
                lua_shiftl, lua_shiftr, pfltvalue, pivalue, psetfltvalue, psetivalue, pttisfloat,
                pttisinteger, setbfvalue, setbtvalue, setfltvalue, setivalue, setnilvalue,
                tointeger, tointegerns, tonumber, tonumberns, ttisinteger,
            },
        },
    },
};
pub use helper::{get_metamethod_event, get_metatable};
pub use metamethod::TmKind;
pub use metamethod::call_tm_res;

use crate::lua_vm::LuaError;

/// Execute until call depth reaches target_depth
/// Used for protected calls (pcall) to execute only the called function
/// without affecting caller frames
///
/// ARCHITECTURE: Single-loop execution like Lua C's luaV_execute
/// - Uses labeled loops instead of goto for context switching
/// - Function calls/returns just update pointers and continue
/// - Zero Rust function call overhead
///
/// NOTE: n_ccalls tracking is NOT done here (unlike the wrapper approach).
/// Instead, each recursive CALL SITE (metamethods, pcall, resume, __close)
/// increments/decrements n_ccalls around its call to lua_execute, mirroring
/// Lua 5.5's luaD_call pattern.
pub fn lua_execute(lua_state: &mut LuaState, target_depth: usize) -> LuaResult<()> {
    // STARTFUNC: Function context switching point (like Lua C's startfunc label)
    'startfunc: loop {
        // Check if we've returned past target depth.
        let current_depth = lua_state.call_depth();
        if current_depth <= target_depth {
            return Ok(());
        }

        let frame_idx = current_depth - 1;
        // ===== LOAD FRAME CONTEXT =====
        // Safety: frame_idx < call_depth (guaranteed by check above)
        let ci = lua_state.get_call_info(frame_idx);

        // Clear stale stack slots between current top and the frame's
        // register extent (ci.top = base + maxstacksize).
        // After a CALL returns, the return handler lowers stack_top to
        // func_pos + nresults. Slots above this new top may contain stale
        // GC pointers from the previous frame or from before the call.
        // Without clearing, a later push_lua_frame could raise top past
        // these stale slots, bringing dangling pointers into GC marking
        // range and causing crashes during sweep.
        // We nil them here instead of raising top (which would break
        // RETURN B=0 MULTRET semantics that rely on top for counting).
        {
            let current_top = lua_state.get_top();
            let ci_top = ci.top;
            if current_top < ci_top {
                let stack = lua_state.stack_mut();
                for i in current_top..ci_top {
                    stack[i] = LuaValue::nil();
                }
            }
        }
        let ci = lua_state.get_call_info(frame_idx);

        // Cold-path check: C frame or pending metamethod finish.
        // Normal Lua function entry: call_status == CIST_LUA, so this is always
        // predicted not-taken.
        if ci.call_status & (CIST_C | CIST_PENDING_FINISH) != 0 {
            if handle_pending_ops(lua_state, frame_idx)? {
                continue 'startfunc;
            }
        }

        // Hot path: read CI fields for Lua function dispatch.
        let ci = lua_state.get_call_info(frame_idx);
        let func_value = ci.func;
        let mut pc = ci.pc as usize;
        let mut base = ci.base;

        let lua_func = unsafe { func_value.as_lua_function_unchecked() };

        let chunk = lua_func.chunk();
        let upvalue_ptrs = lua_func.upvalues();
        // Stack already grown by push_lua_frame — no need for grow_stack here.
        // Only the very first entry (top-level chunk) needs this check.
        debug_assert!(lua_state.stack_len() >= base + chunk.max_stack_size + 5);

        // Cache pointers
        let constants = &chunk.constants;
        let code = &chunk.code;

        // Macro to save PC before operations that may call functions
        macro_rules! save_pc {
            () => {
                lua_state.set_frame_pc(frame_idx, pc as u32);
            };
        }

        // Macro to restore state after operations that may change frames
        macro_rules! restore_state {
            () => {
                debug_assert!(frame_idx < lua_state.call_depth());
                base = lua_state.get_frame_base(frame_idx);
            };
        }

        // MAINLOOP: Main instruction dispatch loop
        loop {
            // Fetch instruction and advance PC
            let instr = unsafe { *code.get_unchecked(pc) };
            pc += 1;

            // Dispatch instruction (continues in next replacement...)
            match instr.get_opcode() {
                OpCode::Move => {
                    // R[A] := R[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let stack = lua_state.stack_mut();
                    unsafe {
                        *stack.get_unchecked_mut(base + a) = *stack.get_unchecked(base + b);
                    }
                }
                OpCode::LoadI => {
                    // R[A] := sBx
                    let a = instr.get_a() as usize;
                    let sbx = instr.get_sbx();
                    let stack = lua_state.stack_mut();
                    unsafe {
                        *stack.get_unchecked_mut(base + a) = LuaValue::integer(sbx as i64);
                    }
                }
                OpCode::LoadF => {
                    // R[A] := (float)sBx
                    let a = instr.get_a() as usize;
                    let sbx = instr.get_sbx();
                    let stack = lua_state.stack_mut();
                    unsafe {
                        *stack.get_unchecked_mut(base + a) = LuaValue::float(sbx as f64);
                    }
                }
                OpCode::LoadK => {
                    // R[A] := K[Bx]
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;
                    let stack = lua_state.stack_mut();
                    unsafe {
                        *stack.get_unchecked_mut(base + a) = *constants.get_unchecked(bx);
                    }
                }
                OpCode::LoadKX => {
                    handle_loadkx(lua_state, instr, base, frame_idx, code, constants, &mut pc)?;
                }
                OpCode::LoadFalse => {
                    // R[A] := false
                    let a = instr.get_a() as usize;
                    let stack = lua_state.stack_mut();
                    setbfvalue(&mut stack[base + a]);
                }
                OpCode::LFalseSkip => {
                    // R[A] := false; pc++
                    let a = instr.get_a() as usize;
                    let stack = lua_state.stack_mut();
                    setbfvalue(&mut stack[base + a]);
                    pc += 1; // Skip next instruction
                }
                OpCode::LoadTrue => {
                    // R[A] := true
                    let a = instr.get_a() as usize;
                    let stack = lua_state.stack_mut();
                    setbtvalue(&mut stack[base + a]);
                }
                OpCode::LoadNil => {
                    // R[A], R[A+1], ..., R[A+B] := nil
                    let a = instr.get_a() as usize;
                    let mut b = instr.get_b() as usize;

                    let stack = lua_state.stack_mut();
                    let mut idx = base + a;
                    loop {
                        setnilvalue(&mut stack[idx]);
                        if b == 0 {
                            break;
                        }
                        b -= 1;
                        idx += 1;
                    }
                }
                OpCode::Add => {
                    // op_arith(L, l_addi, luai_numadd)
                    // R[A] := R[B] + R[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    // OPTIMIZATION: Use raw pointers to eliminate bounds checking and borrowing overhead
                    let stack = lua_state.stack_mut();
                    unsafe {
                        let v1_ptr = stack.as_ptr().add(base + b);
                        let v2_ptr = stack.as_ptr().add(base + c);
                        let ra_ptr = stack.as_mut_ptr().add(base + a);

                        // Fast path: both integers (most common case)
                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            psetivalue(ra_ptr, i1.wrapping_add(i2));
                            pc += 1; // Skip metamethod on success
                        }
                        // Slow path: try float conversion
                        else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if tonumberns(&*v1_ptr, &mut n1) && tonumberns(&*v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 + n2);
                                pc += 1; // Skip metamethod on success
                            }
                            // else: fall through to MMBIN (next instruction)
                        }
                    }
                }
                OpCode::AddI => {
                    // op_arithI(L, l_addi, luai_numadd)
                    // R[A] := R[B] + sC
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let sc = instr.get_sc();

                    // OPTIMIZATION: Use raw pointers for zero-cost abstraction
                    let stack = lua_state.stack_mut();
                    unsafe {
                        let v1_ptr = stack.as_ptr().add(base + b);
                        let ra_ptr = stack.as_mut_ptr().add(base + a);

                        // Fast path: integer (most common)
                        if pttisinteger(v1_ptr) {
                            let iv1 = pivalue(v1_ptr);
                            psetivalue(ra_ptr, iv1.wrapping_add(sc as i64));
                            pc += 1; // Skip metamethod on success
                        }
                        // Slow path: float
                        else if pttisfloat(v1_ptr) {
                            let nb = pfltvalue(v1_ptr);
                            psetfltvalue(ra_ptr, nb + (sc as f64));
                            pc += 1; // Skip metamethod on success
                        }
                        // else: fall through to MMBINI (next instruction)
                    }
                }
                OpCode::Sub => {
                    // op_arith(L, l_subi, luai_numsub)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    unsafe {
                        let v1_ptr = stack.as_ptr().add(base + b);
                        let v2_ptr = stack.as_ptr().add(base + c);
                        let ra_ptr = stack.as_mut_ptr().add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            psetivalue(ra_ptr, i1.wrapping_sub(i2));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if tonumberns(&*v1_ptr, &mut n1) && tonumberns(&*v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 - n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Mul => {
                    // op_arith(L, l_muli, luai_nummul)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    unsafe {
                        let v1_ptr = stack.as_ptr().add(base + b);
                        let v2_ptr = stack.as_ptr().add(base + c);
                        let ra_ptr = stack.as_mut_ptr().add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            psetivalue(ra_ptr, i1.wrapping_mul(i2));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if tonumberns(&*v1_ptr, &mut n1) && tonumberns(&*v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 * n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Div => {
                    // op_arithf(L, luai_numdiv) - 浮点除法
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &stack[base + c];

                    let mut n1 = 0.0;
                    let mut n2 = 0.0;
                    if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                        pc += 1;
                        setfltvalue(&mut stack[base + a], n1 / n2);
                    }
                }
                OpCode::IDiv => {
                    // op_arith(L, luaV_idiv, luai_numidiv) - 整数除法
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &stack[base + c];

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        if i2 != 0 {
                            pc += 1;
                            setivalue(&mut stack[base + a], lua_idiv(i1, i2));
                        } else {
                            save_pc!();
                            return Err(lua_state.error("attempt to divide by zero".to_string()));
                        }
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(&mut stack[base + a], (n1 / n2).floor());
                        }
                    }
                }
                OpCode::Mod => {
                    // op_arith(L, luaV_mod, luaV_modf)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &stack[base + c];

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        if i2 != 0 {
                            pc += 1;
                            setivalue(&mut stack[base + a], lua_imod(i1, i2));
                        } else {
                            save_pc!();
                            return Err(lua_state.error("attempt to perform 'n%0'".to_string()));
                        }
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(&mut stack[base + a], n1 - (n1 / n2).floor() * n2);
                        }
                    }
                }
                OpCode::Pow => {
                    // op_arithf(L, luai_numpow)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &stack[base + c];

                    let mut n1 = 0.0;
                    let mut n2 = 0.0;
                    if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                        pc += 1;
                        setfltvalue(&mut stack[base + a], n1.powf(n2));
                    }
                }
                OpCode::Unm => {
                    // 取负: -value
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let stack = lua_state.stack_mut();
                    let rb = stack[base + b];

                    if ttisinteger(&rb) {
                        let ib = ivalue(&rb);
                        setivalue(&mut stack[base + a], ib.wrapping_neg());
                    } else {
                        let mut nb = 0.0;
                        if tonumberns(&rb, &mut nb) {
                            setfltvalue(&mut stack[base + a], -nb);
                        } else {
                            // Try __unm metamethod with Protect pattern
                            save_pc!();
                            match metamethod::try_unary_tm(
                                lua_state,
                                rb,
                                base + a,
                                metamethod::TmKind::Unm,
                            ) {
                                Ok(_) => {}
                                Err(LuaError::Yield) => {
                                    let ci = lua_state.get_call_info_mut(frame_idx);
                                    ci.call_status |= CIST_PENDING_FINISH;
                                    return Err(LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                            restore_state!();
                        }
                    }
                }
                OpCode::AddK => {
                    // op_arithK(L, l_addi, luai_numadd)
                    // R[A] := R[B] + K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &constants[c]; // K[C]

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        pc += 1;
                        setivalue(&mut stack[base + a], i1.wrapping_add(i2));
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(&mut stack[base + a], n1 + n2);
                        }
                    }
                }
                OpCode::SubK => {
                    // R[A] := R[B] - K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &constants[c];

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        pc += 1;
                        setivalue(&mut stack[base + a], i1.wrapping_sub(i2));
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(&mut stack[base + a], n1 - n2);
                        }
                    }
                }
                OpCode::MulK => {
                    // R[A] := R[B] * K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &constants[c];

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        pc += 1;
                        setivalue(&mut stack[base + a], i1.wrapping_mul(i2));
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(&mut stack[base + a], n1 * n2);
                        }
                    }
                }
                OpCode::ModK => {
                    // R[A] := R[B] % K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &constants[c];

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        if i2 != 0 {
                            pc += 1;
                            setivalue(&mut stack[base + a], lua_imod(i1, i2));
                        } else {
                            save_pc!();
                            return Err(lua_state.error("attempt to perform 'n%0'".to_string()));
                        }
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(&mut stack[base + a], n1 - (n1 / n2).floor() * n2);
                        }
                    }
                }
                OpCode::PowK => {
                    // R[A] := R[B] ^ K[C] (always float)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &constants[c];

                    let mut n1 = 0.0;
                    let mut n2 = 0.0;
                    if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                        pc += 1;
                        setfltvalue(&mut stack[base + a], n1.powf(n2));
                    }
                }
                OpCode::DivK => {
                    // R[A] := R[B] / K[C] (float division)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &constants[c];

                    let mut n1 = 0.0;
                    let mut n2 = 0.0;
                    if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                        pc += 1;
                        setfltvalue(&mut stack[base + a], n1 / n2);
                    }
                }
                OpCode::IDivK => {
                    // R[A] := R[B] // K[C] (floor division)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &constants[c];

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        if i2 != 0 {
                            pc += 1;
                            setivalue(&mut stack[base + a], lua_idiv(i1, i2));
                        } else {
                            save_pc!();
                            return Err(lua_state.error("attempt to divide by zero".to_string()));
                        }
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(&mut stack[base + a], (n1 / n2).floor());
                        }
                    }
                }
                OpCode::BAndK => {
                    // R[A] := R[B] & K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &constants[c];

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(&mut stack[base + a], i1 & i2);
                    }
                }
                OpCode::BOrK => {
                    // R[A] := R[B] | K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &constants[c];

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(&mut stack[base + a], i1 | i2);
                    }
                }
                OpCode::BXorK => {
                    // R[A] := R[B] ^ K[C] (bitwise xor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &constants[c];

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(&mut stack[base + a], i1 ^ i2);
                    }
                }
                OpCode::BAnd => {
                    // op_bitwise(L, l_band)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &stack[base + c];

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(&mut stack[base + a], i1 & i2);
                    }
                }
                OpCode::BOr => {
                    // op_bitwise(L, l_bor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &stack[base + c];

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(&mut stack[base + a], i1 | i2);
                    }
                }
                OpCode::BXor => {
                    // op_bitwise(L, l_bxor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &stack[base + c];

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(&mut stack[base + a], i1 ^ i2);
                    }
                }
                OpCode::Shl => {
                    // op_bitwise(L, luaV_shiftl)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &stack[base + c];

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(&mut stack[base + a], lua_shiftl(i1, i2));
                    }
                }
                OpCode::Shr => {
                    // op_bitwise(L, luaV_shiftr)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = &stack[base + b];
                    let v2 = &stack[base + c];

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(&mut stack[base + a], lua_shiftr(i1, i2));
                    }
                }
                OpCode::BNot => {
                    // 按位非: ~value
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = stack[base + b];

                    let mut ib = 0i64;
                    if tointegerns(&v1, &mut ib) {
                        setivalue(&mut stack[base + a], !ib);
                    } else {
                        // Try __bnot metamethod with Protect pattern
                        save_pc!();
                        match metamethod::try_unary_tm(
                            lua_state,
                            v1,
                            base + a,
                            metamethod::TmKind::Bnot,
                        ) {
                            Ok(_) => {}
                            Err(LuaError::Yield) => {
                                let ci = lua_state.get_call_info_mut(frame_idx);
                                ci.call_status |= CIST_PENDING_FINISH;
                                return Err(LuaError::Yield);
                            }
                            Err(e) => return Err(e),
                        }
                        restore_state!();
                    }
                }
                OpCode::ShlI => {
                    // R[A] := sC << R[B]
                    // Note: In Lua 5.5, SHLI is immediate << register (not register << immediate)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc(); // shift amount from immediate

                    let stack = lua_state.stack_mut();
                    let rb = &stack[base + b];

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftl(ic, ib): shift ic left by ib
                        setivalue(&mut stack[base + a], lua_shiftl(ic as i64, ib));
                    }
                    // else: metamethod
                }
                OpCode::ShrI => {
                    // R[A] := R[B] >> sC
                    // Logical right shift (Lua 5.5: luaV_shiftr)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc(); // shift amount

                    let stack = lua_state.stack_mut();
                    let rb = &stack[base + b];

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftr(ib, ic) = luaV_shiftl(ib, -ic)
                        setivalue(&mut stack[base + a], lua_shiftr(ib, ic as i64));
                    }
                    // else: metamethod
                }
                OpCode::Jmp => {
                    // pc += sJ
                    let sj = instr.get_sj();
                    let new_pc = (pc as i32 + sj) as usize;

                    if new_pc >= code.len() {
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(lua_state.error(format!("JMP: invalid target pc={}", new_pc)));
                    }

                    pc = new_pc;
                }
                OpCode::Return => {
                    // return R[A], ..., R[A+B-2]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    // Update PC before returning
                    save_pc!();

                    // Handle return
                    return_handler::handle_return(lua_state, base, frame_idx, a, b, c, k)?;
                    continue 'startfunc;
                }
                OpCode::Return0 => {
                    // return (no values)
                    return_handler::handle_return0(lua_state, frame_idx);
                    continue 'startfunc;
                }
                OpCode::Return1 => {
                    // return R[A] — hottest return path
                    let a = instr.get_a() as usize;
                    return_handler::handle_return1(lua_state, base, frame_idx, a);
                    continue 'startfunc;
                }
                OpCode::GetUpval => {
                    // R[A] := UpValue[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let value = unsafe { upvalue_ptrs.get_unchecked(b) }
                        .as_ref()
                        .data
                        .get_value();
                    let stack = lua_state.stack_mut();
                    unsafe {
                        *stack.get_unchecked_mut(base + a) = value;
                    }
                }
                OpCode::SetUpval => {
                    // UpValue[B] := R[A]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let value = unsafe { *lua_state.stack().get_unchecked(base + a) };
                    let upval_ptr = unsafe { *upvalue_ptrs.get_unchecked(b) };
                    upval_ptr.as_mut_ref().data.set_value(value);
                    // GC barrier (only for collectable values)
                    if value.is_collectable() {
                        if let Some(gc_ptr) = value.as_gc_ptr() {
                            lua_state.gc_barrier(upval_ptr, gc_ptr);
                        }
                    }
                }
                OpCode::Close => {
                    handle_close(lua_state, instr, base, frame_idx, pc)?;
                }
                OpCode::Tbc => {
                    // Mark variable as to-be-closed
                    let a = instr.get_a() as usize;
                    let stack_idx = base + a;
                    lua_state.mark_tbc(stack_idx)?;
                }
                OpCode::NewTable => {
                    // R[A] := {} (new table) — table ops should be inlined
                    let a = instr.get_a() as usize;
                    let vb = instr.get_vb() as usize;
                    let mut vc = instr.get_vc() as usize;
                    let k = instr.get_k();

                    let hash_size = if vb > 0 {
                        if vb > 31 { 0 } else { 1usize << (vb - 1) }
                    } else {
                        0
                    };

                    if k {
                        if pc < code.len() {
                            let extra_instr = code[pc];
                            if extra_instr.get_opcode() == OpCode::ExtraArg {
                                vc += extra_instr.get_ax() as usize * 1024;
                            }
                        }
                    }

                    pc += 1; // skip EXTRAARG

                    let value = lua_state.create_table(vc, hash_size)?;
                    let stack = lua_state.stack_mut();
                    stack[base + a] = value;

                    let new_top = base + a + 1;
                    save_pc!();
                    lua_state.set_top(new_top)?;
                    lua_state.check_gc()?;

                    let frame_top = lua_state.get_call_info(frame_idx).top;
                    lua_state.set_top(frame_top)?;
                }
                OpCode::GetTable => {
                    // GETTABLE: R[A] := R[B][R[C]]
                    // HOT PATH: inline fast path for integer keys into tables
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let rb = unsafe { *stack.get_unchecked(base + b) };
                    let rc = unsafe { *stack.get_unchecked(base + c) };

                    // Inline fast path: table[integer_key]
                    if let Some(table_ref) = rb.as_table() {
                        let result = if rc.ttisinteger() {
                            table_ref.impl_table.fast_geti(rc.ivalue())
                        } else {
                            table_ref.impl_table.raw_get(&rc)
                        };
                        if let Some(val) = result {
                            unsafe {
                                *stack.get_unchecked_mut(base + a) = val;
                            }
                            continue;
                        }
                    }

                    // Slow path: metamethod
                    table_ops::exec_gettable(lua_state, instr, base, frame_idx, &mut pc)?;
                }
                OpCode::GetI => {
                    // GETI: R[A] := R[B][C] (integer key)
                    // HOT PATH: Unsafe stack access, single stack_mut() call
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as i64;

                    let stack = lua_state.stack_mut();
                    let rb = unsafe { *stack.get_unchecked(base + b) };

                    // Try fast path via inline fast_geti
                    let result = if let Some(table_ref) = rb.as_table() {
                        table_ref.impl_table.fast_geti(c)
                    } else {
                        None
                    };

                    if let Some(val) = result {
                        // Fast path succeeded - write directly, no second stack_mut() needed
                        unsafe {
                            *stack.get_unchecked_mut(base + a) = val;
                        }
                    } else {
                        // Slow path: metamethod lookup
                        save_pc!();
                        table_ops::exec_geti(lua_state, instr, base, frame_idx, &mut pc)?;
                        restore_state!();
                    }
                }
                OpCode::GetField => {
                    // GETFIELD: R[A] := R[B][K[C]:string]
                    // HOT PATH: Unsafe stack access, single stack_mut()
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let rb = unsafe { *stack.get_unchecked(base + b) };
                    let key = unsafe { constants.get_unchecked(c) };

                    // Try fast path: table with string key
                    let result = if let Some(table_ref) = rb.as_table() {
                        table_ref.impl_table.fast_getfield(key)
                    } else {
                        None
                    };

                    if let Some(val) = result {
                        // Fast path succeeded - no second stack_mut()
                        unsafe {
                            *stack.get_unchecked_mut(base + a) = val;
                        }
                    } else {
                        // Slow path: metamethod lookup
                        save_pc!();
                        table_ops::exec_getfield(
                            lua_state, instr, constants, base, frame_idx, &mut pc,
                        )?;
                        restore_state!();
                    }
                }
                OpCode::SetTable => {
                    table_ops::exec_settable(
                        lua_state, instr, constants, base, frame_idx, &mut pc,
                    )?;
                }
                OpCode::SetI => {
                    // SETI: R[A][B] := RK(C) (integer key)
                    // HOT PATH: Uses fast_seti() for zero-cost abstraction
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let stack = lua_state.stack();
                    let ra = stack[base + a];
                    let value = if k { constants[c] } else { stack[base + c] };

                    // Try fast path: table with array access
                    let fast_path_ok = if let Some(table_ref) = ra.as_table_mut() {
                        if !table_ref.has_metatable() {
                            table_ref.impl_table.fast_seti(b as i64, value)
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if fast_path_ok {
                        // GC write barrier: if the table (BLACK) now references
                        // a new WHITE value, the GC must be notified.
                        if value.is_collectable() {
                            if let Some(gc_ptr) = ra.as_gc_ptr() {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                        }
                    } else {
                        // Slow path: metamethod or hash part
                        save_pc!();
                        table_ops::exec_seti(
                            lua_state, instr, constants, base, frame_idx, &mut pc,
                        )?;
                        restore_state!();
                    }
                }
                OpCode::SetField => {
                    // SETFIELD: R[A][K[B]:string] := RK(C)
                    // HOT PATH: Uses fast_setfield() for zero-cost abstraction
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let stack = lua_state.stack();
                    let ra = stack[base + a];
                    let key = &constants[b];
                    let value = if k { constants[c] } else { stack[base + c] };

                    // Try fast path: table without metatable
                    let fast_path_ok = if let Some(table_ref) = ra.as_table_mut() {
                        if !table_ref.has_metatable() {
                            table_ref.impl_table.fast_setfield(key, value)
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if fast_path_ok {
                        // GC write barrier: if the table (BLACK) now references
                        // a new WHITE value, the GC must be notified.
                        if value.is_collectable() {
                            if let Some(gc_ptr) = ra.as_gc_ptr() {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                        }
                    } else {
                        // Slow path: metamethod, new key insertion, or non-table
                        save_pc!();
                        table_ops::exec_setfield(
                            lua_state, instr, constants, base, frame_idx, &mut pc,
                        )?;
                        restore_state!();
                    }
                }
                OpCode::Self_ => {
                    table_ops::exec_self(lua_state, instr, constants, base, frame_idx, &mut pc)?;
                }
                OpCode::Call => {
                    // R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    save_pc!();
                    match call::handle_call(lua_state, base, a, b, c, 0) {
                        Ok(FrameAction::Continue) => {
                            restore_state!();
                        }
                        Ok(FrameAction::Call) => {
                            continue 'startfunc;
                        }
                        Ok(FrameAction::TailCall) => {
                            continue 'startfunc;
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::TailCall => {
                    // Tail call optimization: return R[A](R[A+1], ... ,R[A+B-1])
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    // Save PC before call
                    save_pc!();

                    // Delegate to tailcall handler
                    match call::handle_tailcall(lua_state, base, a, b) {
                        Ok(FrameAction::Continue) => {
                            // Continue execution
                            restore_state!();
                        }
                        Ok(FrameAction::TailCall) => {
                            // Tail call replaced frame
                            continue 'startfunc;
                        }
                        Ok(FrameAction::Call) => {
                            // Shouldn't happen from handle_tailcall
                            continue 'startfunc;
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::Not => {
                    // R[A] := not R[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let stack = lua_state.stack_mut();
                    let rb = &stack[base + b];

                    // l_isfalse: nil or false
                    let is_false = rb.tt() == LUA_VFALSE || rb.is_nil();
                    if is_false {
                        setbtvalue(&mut stack[base + a]);
                    } else {
                        setbfvalue(&mut stack[base + a]);
                    }
                }
                OpCode::ForLoop => {
                    // Numeric for loop
                    // If integer: check counter, decrement, add step, jump back
                    // If float: add step, check limit, jump back
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    unsafe {
                        let ra = base + a;

                        // Check if integer loop
                        if ttisinteger(stack.get_unchecked(ra + 1)) {
                            // Integer loop (most common for numeric loops)
                            // ra: counter (count of iterations left)
                            // ra+1: step
                            // ra+2: control variable (idx)
                            let count = ivalue(stack.get_unchecked(ra)) as u64; // unsigned count
                            if count > 0 {
                                // More iterations
                                let step = ivalue(stack.get_unchecked(ra + 1));
                                let idx = ivalue(stack.get_unchecked(ra + 2));

                                // Update counter (decrement) - only write value, tag unchanged
                                chgivalue(stack.get_unchecked_mut(ra), (count - 1) as i64);

                                // Update control variable: idx += step - only write value
                                chgivalue(stack.get_unchecked_mut(ra + 2), idx.wrapping_add(step));

                                // Jump back (no error check - validated at compile time)
                                pc -= bx;
                            }
                            // else: counter expired, exit loop
                        } else {
                            // Float loop
                            // ra: limit
                            // ra+1: step
                            // ra+2: idx (control variable)
                            let step = fltvalue(stack.get_unchecked(ra + 1));
                            let limit = fltvalue(stack.get_unchecked(ra));
                            let idx = fltvalue(stack.get_unchecked(ra + 2));

                            // idx += step
                            let new_idx = idx + step;

                            // Check if should continue
                            let should_continue = if step > 0.0 {
                                new_idx <= limit
                            } else {
                                new_idx >= limit
                            };

                            if should_continue {
                                // Update control variable - only write value, tag unchanged
                                chgfltvalue(stack.get_unchecked_mut(ra + 2), new_idx);

                                // Jump back
                                if bx > pc {
                                    lua_state.set_frame_pc(frame_idx, pc as u32);
                                    return Err(
                                        lua_state.error("FORLOOP: invalid jump".to_string())
                                    );
                                }
                                pc -= bx;
                            }
                            // else: exit loop
                        }
                    }
                }
                OpCode::ForPrep => {
                    // Prepare numeric for loop — MUST be inline (hot path)
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    let ra = base + a;

                    if ttisinteger(&stack[ra]) && ttisinteger(&stack[ra + 2]) {
                        // Integer loop (init and step are integers)
                        let init = ivalue(&stack[ra]);
                        let step = ivalue(&stack[ra + 2]);

                        if step == 0 {
                            save_pc!();
                            return Err(lua_state.error("'for' step is zero".to_string()));
                        }

                        // forlimit: convert limit to integer per C Lua 5.5 logic
                        let (limit, should_skip) = 'forlimit: {
                            // Try integer limit directly
                            if ttisinteger(&stack[ra + 1]) {
                                let lim = ivalue(&stack[ra + 1]);
                                let skip = if step > 0 { init > lim } else { init < lim };
                                break 'forlimit (lim, skip);
                            }
                            // Try converting to float (handles float and string)
                            let mut flimit = 0.0;
                            if !tonumberns(&stack[ra + 1], &mut flimit) {
                                save_pc!();
                                return Err(
                                    lua_state.error("'for' limit must be a number".to_string())
                                );
                            }
                            // Try rounding the float to integer
                            let nl = if step < 0 {
                                flimit.ceil()
                            } else {
                                flimit.floor()
                            };
                            // Check if the rounded float fits in i64
                            if nl >= (i64::MIN as f64) && nl <= (i64::MAX as f64) && nl == nl {
                                let lim = nl as i64;
                                let skip = if step > 0 { init > lim } else { init < lim };
                                break 'forlimit (lim, skip);
                            }
                            // Float is out of integer range — use C Lua overflow logic
                            if flimit > 0.0 {
                                // Positive float out of range
                                if step < 0 {
                                    // Descending loop can't reach large positive limit
                                    break 'forlimit (0, true);
                                }
                                // Ascending loop: truncate to MAXINTEGER
                                let skip = init > i64::MAX; // always false, but matches pattern
                                break 'forlimit (i64::MAX, skip);
                            } else {
                                // Negative float out of range (or -inf, NaN)
                                if step > 0 {
                                    // Ascending loop can't reach very negative limit
                                    break 'forlimit (0, true);
                                }
                                // Descending loop: truncate to MININTEGER
                                let skip = init < i64::MIN; // always false
                                break 'forlimit (i64::MIN, skip);
                            }
                        };

                        if should_skip {
                            pc += bx + 1;
                        } else {
                            let count = if step > 0 {
                                ((limit as u64).wrapping_sub(init as u64)) / (step as u64)
                            } else {
                                let step_abs = if step == i64::MIN {
                                    i64::MAX as u64 + 1
                                } else {
                                    (-step) as u64
                                };
                                ((init as u64).wrapping_sub(limit as u64)) / step_abs
                            };

                            setivalue(&mut stack[ra], count as i64);
                            setivalue(&mut stack[ra + 1], step);
                            setivalue(&mut stack[ra + 2], init);
                        }
                    } else {
                        // Float loop — cold path
                        handle_forprep_float(lua_state, base + a, bx, frame_idx, &mut pc)?;
                    }
                }
                OpCode::TForPrep => {
                    // Prepare generic for loop — inline (for loop related)
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    let ra = base + a;

                    // Swap control and closing variables
                    let temp = stack[ra + 3];
                    stack[ra + 3] = stack[ra + 2];
                    stack[ra + 2] = temp;

                    // Mark ra+2 as to-be-closed if not nil
                    lua_state.mark_tbc(ra + 2)?;

                    pc += bx;
                }
                OpCode::TForCall => {
                    // Generic for loop call — HOT PATH for ipairs/pairs/next iterators
                    // Call: ra+3,ra+4,...,ra+2+C := ra(ra+1, ra+2)
                    // ra=iterator, ra+1=state, ra+2=closing, ra+3=control
                    let a = instr.get_a() as usize;
                    let c = instr.get_c() as usize;

                    let ra_base = base + a;

                    // Setup call args using unsafe (stack is guaranteed large enough by push_frame)
                    let (iterator, c_func_opt) = unsafe {
                        let stack = lua_state.stack_mut();
                        let iterator = *stack.get_unchecked(ra_base);
                        let state = *stack.get_unchecked(ra_base + 1);
                        let control = *stack.get_unchecked(ra_base + 3);

                        // ra+3: function, ra+4: state, ra+5: control
                        *stack.get_unchecked_mut(ra_base + 3) = iterator;
                        *stack.get_unchecked_mut(ra_base + 4) = state;
                        *stack.get_unchecked_mut(ra_base + 5) = control;

                        // Extract C function pointer while we have the value
                        let c_func_opt = if let Some(cf) = iterator.as_cfunction() {
                            Some(cf)
                        } else if let Some(cc) = iterator.as_cclosure() {
                            Some(cc.func())
                        } else {
                            None
                        };

                        (iterator, c_func_opt)
                    };

                    // Save PC before call
                    lua_state.set_frame_pc(frame_idx, pc as u32);

                    if let Some(c_func) = c_func_opt {
                        // FAST PATH: C function iterator (ipairs_next, lua_next, etc.)
                        call::call_c_function_fast(
                            lua_state,
                            &iterator,
                            c_func,
                            ra_base + 3,
                            2, // always 2 args (state, control)
                            c as i32 + 1,
                        )?;
                        restore_state!();
                    } else {
                        // Slow path: Lua function or __call metamethod
                        match call::handle_call(lua_state, base, a + 3, 3, c + 1, 0) {
                            Ok(FrameAction::Continue) => {
                                restore_state!();
                            }
                            Ok(FrameAction::Call) => {
                                continue 'startfunc;
                            }
                            Ok(FrameAction::TailCall) => {
                                continue 'startfunc;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
                OpCode::TForLoop => {
                    // Generic for loop test
                    // If ra+3 (control variable) != nil then continue loop (jump back)
                    // After TForPrep swap: ra+2=closing(TBC), ra+3=control
                    // TFORCALL places first result at ra+3, automatically updating control
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    let ra = base + a;

                    // Check if ra+3 (control value from iterator) is not nil
                    if !unsafe { stack.get_unchecked(ra + 3) }.is_nil() {
                        // Continue loop: jump back
                        pc -= bx;
                    }
                    // else: exit loop (control variable is nil)
                }
                OpCode::MmBin => {
                    // Call metamethod over R[A] and R[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    // Protect metamethod call
                    save_pc!();
                    metamethod::handle_mmbin(lua_state, base, a, b, c, pc, code, frame_idx)?;
                    restore_state!();
                }
                OpCode::MmBinI => {
                    // Call metamethod over R[A] and immediate sB
                    let a = instr.get_a() as usize;
                    let sb = instr.get_sb();
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    // Protect metamethod call
                    save_pc!();
                    metamethod::handle_mmbini(lua_state, base, a, sb, c, k, pc, code, frame_idx)?;
                    restore_state!();
                }
                OpCode::MmBinK => {
                    // Call metamethod over R[A] and K[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    // Protect metamethod call
                    save_pc!();
                    metamethod::handle_mmbink(
                        lua_state, base, a, b, c, k, pc, code, constants, frame_idx,
                    )?;
                    restore_state!();
                }

                // ============================================================
                // UPVALUE TABLE ACCESS
                // ============================================================
                OpCode::GetTabUp => {
                    // R[A] := UpValue[B][K[C]:shortstring]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let upval = &upvalue_ptrs[b].as_ref().data;
                    let key = &constants[c];
                    let table_value = upval.get_value_ref();

                    // Fast path: direct hash lookup for short string keys
                    let result = if table_value.tt == LUA_VTABLE {
                        let table = unsafe { &*(table_value.value.ptr as *const GcTable) };
                        let native = &table.data.impl_table;
                        if native.has_hash() {
                            native.get_shortstr_unchecked(key)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(val) = result {
                        let stack = lua_state.stack_mut();
                        stack[base + a] = val;
                    } else {
                        // Slow path: metamethod lookup
                        let table_value = upval.get_value_ref().clone();
                        let write_pos = base + a;
                        let call_info = lua_state.get_call_info_mut(frame_idx);
                        if write_pos + 1 > call_info.top {
                            call_info.top = write_pos + 1;
                            lua_state.set_top(write_pos + 1)?;
                        }
                        save_pc!();
                        match helper::lookup_from_metatable(lua_state, &table_value, key) {
                            Ok(result) => {
                                restore_state!();
                                let stack = lua_state.stack_mut();
                                stack[base + a] = result.unwrap_or(LuaValue::nil());
                            }
                            Err(LuaError::Yield) => {
                                // Metamethod yielded — save destination register
                                // so we can finish the operation on resume.
                                let ci = lua_state.get_call_info_mut(frame_idx);
                                ci.pending_finish_get = a as i32;
                                ci.call_status |= CIST_PENDING_FINISH;
                                return Err(LuaError::Yield);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }

                OpCode::SetTabUp => {
                    // UpValue[A][K[B]:shortstring] := RK(C)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let key = constants[b];
                    let value = if k {
                        constants[c]
                    } else {
                        lua_state.stack_mut()[base + c]
                    };

                    // Fast path: direct set for existing short string key
                    let upval = &upvalue_ptrs[a].as_ref().data;
                    let table_value = upval.get_value_ref();
                    if table_value.tt == LUA_VTABLE {
                        let table = unsafe { &mut *(table_value.value.ptr as *mut GcTable) };
                        let native = &mut table.data.impl_table;
                        if native.has_hash() && native.set_shortstr_unchecked(&key, value) {
                            if value.is_collectable() {
                                if let Some(gc_ptr) = table_value.as_gc_ptr() {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                            }
                            continue;
                        }
                    }

                    // Slow path: handle metamethods (__newindex)
                    let table_value = upval.get_value_ref().clone();
                    save_pc!();
                    match helper::finishset(lua_state, &table_value, &key, value) {
                        Ok(_) => {
                            restore_state!();
                        }
                        Err(LuaError::Yield) => {
                            // __newindex yielded — mark for top restoration on resume
                            let ci = lua_state.get_call_info_mut(frame_idx);
                            ci.pending_finish_get = -2;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }

                // ============================================================
                // LENGTH AND CONCATENATION
                // ============================================================
                OpCode::Len => {
                    handle_len(lua_state, instr, &mut base, frame_idx, pc)?;
                }

                OpCode::Concat => {
                    handle_concat(lua_state, instr, &mut base, frame_idx, pc)?;
                }

                // ============================================================
                // COMPARISON OPERATIONS (register-register)
                // ============================================================
                OpCode::Eq => {
                    comparison_ops::exec_eq(lua_state, instr, base, frame_idx, &mut pc)?;
                }

                OpCode::Lt => {
                    comparison_ops::exec_lt(lua_state, instr, base, frame_idx, &mut pc)?;
                }

                OpCode::Le => {
                    comparison_ops::exec_le(lua_state, instr, base, frame_idx, &mut pc)?;
                }

                // ============================================================
                // COMPARISON WITH CONSTANT/IMMEDIATE
                // ============================================================
                OpCode::EqK => {
                    comparison_ops::exec_eqk(lua_state, instr, constants, base, &mut pc)?;
                }

                OpCode::EqI => {
                    comparison_ops::exec_eqi(lua_state, instr, base, &mut pc)?;
                }

                OpCode::LtI => {
                    // LTI fast path: if (R[A] < sB) ~= k then pc++
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();

                    let stack = lua_state.stack_mut();
                    let ra = unsafe { stack.get_unchecked(base + a) };
                    if ra.ttisinteger() {
                        let cond = ra.ivalue() < (im as i64);
                        if cond != k {
                            pc += 1;
                        }
                    } else if ra.ttisfloat() {
                        let cond = ra.fltvalue() < (im as f64);
                        if cond != k {
                            pc += 1;
                        }
                    } else {
                        comparison_ops::exec_lti(lua_state, instr, base, frame_idx, &mut pc)?;
                    }
                }

                OpCode::LeI => {
                    // LEI fast path: if (R[A] <= sB) ~= k then pc++
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();

                    let stack = lua_state.stack_mut();
                    let ra = unsafe { stack.get_unchecked(base + a) };
                    if ra.ttisinteger() {
                        let cond = ra.ivalue() <= (im as i64);
                        if cond != k {
                            pc += 1;
                        }
                    } else if ra.ttisfloat() {
                        let cond = ra.fltvalue() <= (im as f64);
                        if cond != k {
                            pc += 1;
                        }
                    } else {
                        comparison_ops::exec_lei(lua_state, instr, base, frame_idx, &mut pc)?;
                    }
                }

                OpCode::GtI => {
                    // GTI fast path
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();

                    let stack = lua_state.stack_mut();
                    let ra = unsafe { stack.get_unchecked(base + a) };
                    if ra.ttisinteger() {
                        let cond = ra.ivalue() > (im as i64);
                        if cond != k {
                            pc += 1;
                        }
                    } else if ra.ttisfloat() {
                        let cond = ra.fltvalue() > (im as f64);
                        if cond != k {
                            pc += 1;
                        }
                    } else {
                        comparison_ops::exec_gti(lua_state, instr, base, frame_idx, &mut pc)?;
                    }
                }

                OpCode::GeI => {
                    // GEI fast path
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();

                    let stack = lua_state.stack_mut();
                    let ra = unsafe { stack.get_unchecked(base + a) };
                    if ra.ttisinteger() {
                        let cond = ra.ivalue() >= (im as i64);
                        if cond != k {
                            pc += 1;
                        }
                    } else if ra.ttisfloat() {
                        let cond = ra.fltvalue() >= (im as f64);
                        if cond != k {
                            pc += 1;
                        }
                    } else {
                        comparison_ops::exec_gei(lua_state, instr, base, frame_idx, &mut pc)?;
                    }
                }

                // ============================================================
                // CONDITIONAL TESTS
                // ============================================================
                OpCode::Test => {
                    // docondjump(): if (cond != k) then pc++ else donextjump
                    let a = instr.get_a() as usize;
                    let k = instr.get_k();

                    let stack = lua_state.stack_mut();
                    let ra = unsafe { stack.get_unchecked(base + a) };

                    // l_isfalse: nil or false
                    let is_false = ra.is_nil() || ra.tt() == LUA_VFALSE;
                    let cond = !is_false;

                    if cond != k {
                        pc += 1; // Skip next instruction (JMP)
                    } else {
                        // Execute next instruction (must be JMP)
                        let next_instr = unsafe { *chunk.code.get_unchecked(pc) };
                        pc += 1;
                        let sj = next_instr.get_sj();
                        pc = (pc as i32 + sj) as usize;
                    }
                }

                OpCode::TestSet => {
                    // if (l_isfalse(R[B]) == k) then pc++ else R[A] := R[B]; donextjump
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();

                    let stack = lua_state.stack_mut();
                    let rb = &stack[base + b];
                    let is_false = rb.is_nil() || (rb.is_boolean() && rb.tt() == LUA_VFALSE);

                    if is_false == k {
                        pc += 1; // Condition failed - skip next instruction (JMP)
                    } else {
                        // Condition succeeded - copy value and EXECUTE next instruction (must be JMP)
                        stack[base + a] = *rb;
                        // donextjump: fetch and execute next JMP instruction
                        let next_instr = unsafe { *chunk.code.get_unchecked(pc) };
                        debug_assert!(next_instr.get_opcode() == OpCode::Jmp);
                        pc += 1; // Move past the JMP instruction
                        let sj = next_instr.get_sj();
                        pc = (pc as i32 + sj) as usize; // Execute the jump
                    }
                }

                // ============================================================
                // TABLE OPERATIONS
                // ============================================================
                OpCode::SetList => {
                    closure_vararg_ops::exec_setlist(lua_state, instr, code, base, &mut pc)?;
                }

                // ============================================================
                // CLOSURE AND VARARG
                // ============================================================
                OpCode::Closure => {
                    handle_closure(lua_state, instr, base, frame_idx, &chunk, &upvalue_ptrs, pc)?;
                }

                OpCode::Vararg => {
                    closure_vararg_ops::exec_vararg(lua_state, instr, base, frame_idx, &chunk)?;
                }

                OpCode::GetVarg => {
                    handle_getvarg(lua_state, instr, base, frame_idx)?;
                }

                OpCode::ErrNNil => {
                    handle_errnil(lua_state, instr, base, constants, frame_idx, pc)?;
                }

                OpCode::VarargPrep => {
                    closure_vararg_ops::exec_varargprep(lua_state, frame_idx, &chunk, &mut base)?;
                }

                OpCode::ExtraArg => {
                    // Extra argument for previous opcode
                    // This instruction should never be executed directly
                    // It's always consumed by the previous instruction (NEWTABLE, SETLIST, etc.)
                    // If we reach here, it's a compiler error
                    save_pc!();
                    return Err(lua_state.error("unexpected EXTRAARG instruction".to_string()));
                }
            } // end match
        } // end 'mainloop
    } // end 'startfunc
}
