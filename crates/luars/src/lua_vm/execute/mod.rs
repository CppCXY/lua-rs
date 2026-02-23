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
pub(crate) mod metamethod;
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
                handle_pending_ops, ivalue, lua_fmod, lua_idiv, lua_imod, lua_shiftl, lua_shiftr,
                luai_numpow, pfltvalue, pivalue, psetfltvalue, psetivalue, ptonumberns, pttisfloat,
                pttisinteger, setbfvalue, setbtvalue, setfltvalue, setivalue, setnilvalue,
                tointeger, tointegerns, tonumberns, ttisinteger,
            },
        },
        lua_limits::EXTRA_STACK,
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
        // Stale stack slots above stack_top are NOT cleared here.
        // Instead, push_lua_frame nil-fills [stack_top, frame_top) when
        // raising stack_top, which is the only moment stale pointers
        // become visible to the GC (GC scans 0..stack_top).

        // Cold-path check: C frame or pending metamethod finish.
        // Read call_status first (single field), avoids holding a borrow
        // across the mutable handle_pending_ops call.
        let call_status = lua_state.get_call_info(frame_idx).call_status;
        if call_status & (CIST_C | CIST_PENDING_FINISH) != 0
            && handle_pending_ops(lua_state, frame_idx)?
        {
            continue 'startfunc;
        }

        // Hot path: read CI fields for Lua function dispatch.
        let ci = lua_state.get_call_info(frame_idx);
        let func_value = ci.func;
        let mut pc = ci.pc as usize;
        let mut base = ci.base;
        let chunk_ptr = ci.chunk_ptr;

        let lua_func = unsafe { func_value.as_lua_function_unchecked() };

        // Use cached chunk_ptr from CI (avoids Rc deref on every startfunc entry).
        // chunk_ptr is set by push_lua_frame/push_frame/handle_tailcall for all Lua frames.
        let chunk = unsafe { &*chunk_ptr };
        let upvalue_ptrs = lua_func.upvalues();
        // Stack already grown by push_lua_frame — no need for grow_stack here.
        // Only the very first entry (top-level chunk) needs this check.
        debug_assert!(lua_state.stack_len() >= base + chunk.max_stack_size + EXTRA_STACK);

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
                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        *sp.add(base + a) = *sp.add(base + b);
                    }
                }
                OpCode::LoadI => {
                    // R[A] := sBx
                    let a = instr.get_a() as usize;
                    let sbx = instr.get_sbx();
                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        *sp.add(base + a) = LuaValue::integer(sbx as i64);
                    }
                }
                OpCode::LoadF => {
                    // R[A] := (float)sBx
                    let a = instr.get_a() as usize;
                    let sbx = instr.get_sbx();
                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        *sp.add(base + a) = LuaValue::float(sbx as f64);
                    }
                }
                OpCode::LoadK => {
                    // R[A] := K[Bx]
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;
                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        *sp.add(base + a) = *constants.as_ptr().add(bx);
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

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        // Fast path: both integers (most common case)
                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            psetivalue(ra_ptr, i1.wrapping_add(i2));
                            pc += 1; // Skip metamethod on success
                        }
                        // Slow path: try float conversion (no string coercion)
                        else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
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

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

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

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_sub(pivalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
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

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_mul(pivalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
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

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                            psetfltvalue(ra_ptr, n1 / n2);
                            pc += 1;
                        }
                    }
                }
                OpCode::IDiv => {
                    // op_arith(L, luaV_idiv, luai_numidiv) - 整数除法
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_idiv(i1, i2));
                                pc += 1;
                            } else {
                                save_pc!();
                                return Err(
                                    lua_state.error("attempt to divide by zero".to_string())
                                );
                            }
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, (n1 / n2).floor());
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Mod => {
                    // op_arith(L, luaV_mod, luaV_modf)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_imod(i1, i2));
                                pc += 1;
                            } else {
                                save_pc!();
                                return Err(lua_state.error("attempt to perform 'n%0'".to_string()));
                            }
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, lua_fmod(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Pow => {
                    // op_arithf(L, luai_numpow)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = sp.add(base + c) as *const LuaValue;
                        let ra_ptr = sp.add(base + a);

                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                            psetfltvalue(ra_ptr, luai_numpow(n1, n2));
                            pc += 1;
                        }
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

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_add(pivalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 + n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::SubK => {
                    // R[A] := R[B] - K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_sub(pivalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 - n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::MulK => {
                    // R[A] := R[B] * K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_mul(pivalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 * n2);
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::ModK => {
                    // R[A] := R[B] % K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_imod(i1, i2));
                                pc += 1;
                            } else {
                                save_pc!();
                                return Err(lua_state.error("attempt to perform 'n%0'".to_string()));
                            }
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, lua_fmod(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::PowK => {
                    // R[A] := R[B] ^ K[C] (always float)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                            psetfltvalue(ra_ptr, luai_numpow(n1, n2));
                            pc += 1;
                        }
                    }
                }
                OpCode::DivK => {
                    // R[A] := R[B] / K[C] (float division)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                            psetfltvalue(ra_ptr, n1 / n2);
                            pc += 1;
                        }
                    }
                }
                OpCode::IDivK => {
                    // R[A] := R[B] // K[C] (floor division)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let v1_ptr = sp.add(base + b) as *const LuaValue;
                        let v2_ptr = constants.as_ptr().add(c);
                        let ra_ptr = sp.add(base + a);

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            let i1 = pivalue(v1_ptr);
                            let i2 = pivalue(v2_ptr);
                            if i2 != 0 {
                                psetivalue(ra_ptr, lua_idiv(i1, i2));
                                pc += 1;
                            } else {
                                save_pc!();
                                return Err(
                                    lua_state.error("attempt to divide by zero".to_string())
                                );
                            }
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, (n1 / n2).floor());
                                pc += 1;
                            }
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
                    if value.is_collectable()
                        && let Some(gc_ptr) = value.as_gc_ptr()
                    {
                        lua_state.gc_barrier(upval_ptr, gc_ptr);
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

                    if k && pc < code.len() {
                        let extra_instr = code[pc];
                        if extra_instr.get_opcode() == OpCode::ExtraArg {
                            vc += extra_instr.get_ax() as usize * 1024;
                        }
                    }

                    pc += 1; // skip EXTRAARG

                    let value = lua_state.create_table(vc, hash_size)?;
                    let stack = lua_state.stack_mut();
                    stack[base + a] = value;

                    // Lua 5.5's OP_NEWTABLE: lower top to ra+1 then checkGC,
                    // so the GC only scans up to the table (excludes stale
                    // registers above). Then restore top to ci->top.
                    // Use set_top_raw: stack was already grown by push_lua_frame.
                    let new_top = base + a + 1;
                    save_pc!();
                    lua_state.set_top_raw(new_top);
                    lua_state.check_gc()?;
                    let frame_top = lua_state.get_call_info(frame_idx).top;
                    lua_state.set_top_raw(frame_top);
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

                    // Inline fast path: table[key]
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
                        // Key not found — if no metatable, result is nil (skip exec_gettable)
                        if !table_ref.has_metatable() {
                            unsafe {
                                *stack.get_unchecked_mut(base + a) = LuaValue::nil();
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
                    if let Some(table_ref) = rb.as_table() {
                        if let Some(val) = table_ref.impl_table.fast_geti(c) {
                            unsafe {
                                *stack.get_unchecked_mut(base + a) = val;
                            }
                            continue;
                        }
                        // Key not found — if no metatable, result is nil (skip exec_geti)
                        if !table_ref.has_metatable() {
                            unsafe {
                                *stack.get_unchecked_mut(base + a) = LuaValue::nil();
                            }
                            continue;
                        }
                    }

                    // Slow path: metamethod lookup
                    save_pc!();
                    table_ops::exec_geti(lua_state, instr, base, frame_idx, &mut pc)?;
                    restore_state!();
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
                    if let Some(table_ref) = rb.as_table() {
                        if let Some(val) = table_ref.impl_table.fast_getfield(key) {
                            unsafe {
                                *stack.get_unchecked_mut(base + a) = val;
                            }
                            continue;
                        }
                        // Key not found — if no metatable, result is nil (skip exec_getfield)
                        if !table_ref.has_metatable() {
                            unsafe {
                                *stack.get_unchecked_mut(base + a) = LuaValue::nil();
                            }
                            continue;
                        }
                    }

                    // Slow path: metamethod lookup
                    save_pc!();
                    table_ops::exec_getfield(
                        lua_state, instr, constants, base, frame_idx, &mut pc,
                    )?;
                    restore_state!();
                }
                OpCode::SetTable => {
                    // SETTABLE: R[A][R[B]] := RK(C)
                    // HOT PATH: inline fast path for integer keys + no-metatable
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let stack = lua_state.stack();
                    let ra = stack[base + a];
                    let rb = stack[base + b];
                    let val = if k { constants[c] } else { stack[base + c] };

                    if let Some(table_ref) = ra.as_table_mut() {
                        if !table_ref.has_metatable() {
                            // No metatable: try integer fast path first (t[i] = v)
                            if rb.ttisinteger() {
                                if table_ref.impl_table.fast_seti(rb.ivalue(), val) {
                                    // GC write barrier
                                    if val.is_collectable()
                                        && let Some(gc_ptr) = ra.as_gc_ptr()
                                    {
                                        lua_state.gc_barrier_back(gc_ptr);
                                    }
                                    continue;
                                }
                                // Key outside array — skip redundant fast_seti in set_int
                                table_ref.impl_table.set_int_slow(rb.ivalue(), val);
                                if val.is_collectable()
                                    && let Some(gc_ptr) = ra.as_gc_ptr()
                                {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            // Non-integer key: validate then raw_set
                            if rb.is_nil() {
                                return Err(lua_state.error("table index is nil".to_string()));
                            }
                            if rb.ttisfloat() && rb.fltvalue().is_nan() {
                                return Err(lua_state.error("table index is NaN".to_string()));
                            }
                            lua_state.raw_set(&ra, rb, val);
                            continue;
                        }
                        // Has metatable: if integer key with existing non-nil value
                        // in array, __newindex is NOT consulted
                        if rb.ttisinteger()
                            && table_ref.impl_table.fast_seti_existing(rb.ivalue(), val)
                        {
                            if val.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                        // Generic non-integer existing key check
                        if let Some(existing) = table_ref.impl_table.raw_get(&rb)
                            && !existing.is_nil()
                        {
                            lua_state.raw_set(&ra, rb, val);
                            continue;
                        }
                    }

                    // Slow path: metamethod or non-table
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

                    // Try fast path: table with array access.
                    // Without metatable: any in-range array write is fine.
                    // With metatable: only overwrite existing non-nil values
                    // (nil slots require __newindex check).
                    if let Some(table_ref) = ra.as_table_mut() {
                        if !table_ref.has_metatable() {
                            if table_ref.impl_table.fast_seti(b as i64, value) {
                                // GC write barrier
                                if value.is_collectable()
                                    && let Some(gc_ptr) = ra.as_gc_ptr()
                                {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            // No metatable: use set_int_slow (skip redundant fast_seti)
                            table_ref.impl_table.set_int_slow(b as i64, value);
                            if value.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                        if table_ref.impl_table.fast_seti_existing(b as i64, value) {
                            // GC write barrier
                            if value.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                    }

                    // Slow path: metamethod or non-table
                    save_pc!();
                    table_ops::exec_seti(lua_state, instr, constants, base, frame_idx, &mut pc)?;
                    restore_state!();
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

                    // Try fast path: fast_setfield only succeeds when the key
                    // already exists with a non-nil value. Per Lua semantics,
                    // __newindex is NEVER consulted when the key already exists
                    // in the table's own hash part. So this is safe regardless
                    // of whether the table has a metatable.
                    let fast_path_ok = if let Some(table_ref) = ra.as_table_mut() {
                        table_ref.impl_table.fast_setfield(key, value)
                    } else {
                        false
                    };

                    if fast_path_ok {
                        // GC write barrier: if the table (BLACK) now references
                        // a new WHITE value, the GC must be notified.
                        if value.is_collectable()
                            && let Some(gc_ptr) = ra.as_gc_ptr()
                        {
                            lua_state.gc_barrier_back(gc_ptr);
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
                    let func_idx = base + a;

                    // Hot path: inline Lua function call
                    // Avoids handle_call overhead (FrameAction enum, set_top_raw,
                    // cold C/__call code in instruction cache)
                    let func = unsafe { *lua_state.stack().get_unchecked(func_idx) };
                    if func.is_lua_function() {
                        // Compute nargs without set_top_raw
                        // (push_lua_frame handles stack_top directly)
                        let nargs = if b != 0 {
                            b - 1
                        } else {
                            let current_top = lua_state.get_top();
                            if current_top > func_idx + 1 {
                                current_top - func_idx - 1
                            } else {
                                0
                            }
                        };
                        let nresults = if c == 0 { -1 } else { (c - 1) as i32 };

                        let lua_func = unsafe { func.as_lua_function_unchecked() };
                        let chunk = lua_func.chunk();

                        save_pc!();
                        lua_state.push_lua_frame(
                            &func,
                            func_idx + 1,
                            nargs,
                            nresults,
                            chunk.param_count,
                            chunk.max_stack_size,
                            chunk as *const _,
                        )?;
                        continue 'startfunc;
                    }

                    // Cold path: C function or __call metamethod
                    save_pc!();
                    match call::handle_call(lua_state, base, a, b, c, 0) {
                        Ok(FrameAction::Continue) => {
                            restore_state!();
                        }
                        Ok(FrameAction::Call) | Ok(FrameAction::TailCall) => {
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

                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let ra = base + a;

                        // Check if integer loop
                        if pttisinteger(sp.add(ra + 1) as *const LuaValue) {
                            // Integer loop (most common for numeric loops)
                            // ra: counter (count of iterations left)
                            // ra+1: step
                            // ra+2: control variable (idx)
                            let count = pivalue(sp.add(ra) as *const LuaValue) as u64;
                            if count > 0 {
                                // More iterations
                                let step = pivalue(sp.add(ra + 1) as *const LuaValue);
                                let idx = pivalue(sp.add(ra + 2) as *const LuaValue);

                                // Update counter (decrement) - only write value, tag unchanged
                                (*sp.add(ra)).value.i = (count - 1) as i64;

                                // Update control variable: idx += step - only write value
                                (*sp.add(ra + 2)).value.i = idx.wrapping_add(step);

                                // Jump back (no error check - validated at compile time)
                                pc -= bx;
                            }
                            // else: counter expired, exit loop
                        } else {
                            // Float loop
                            // ra: limit
                            // ra+1: step
                            // ra+2: idx (control variable)
                            let step = pfltvalue(sp.add(ra + 1) as *const LuaValue);
                            let limit = pfltvalue(sp.add(ra) as *const LuaValue);
                            let idx = pfltvalue(sp.add(ra + 2) as *const LuaValue);

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
                                (*sp.add(ra + 2)).value.n = new_idx;

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
                            let limit_val = stack[ra + 1]; // Copy to avoid borrow conflict
                            if !tonumberns(&limit_val, &mut flimit) {
                                let t = crate::stdlib::debug::objtypename(lua_state, &limit_val);
                                save_pc!();
                                return Err(lua_state.error(format!(
                                    "bad 'for' limit (number expected, got {})",
                                    t
                                )));
                            }
                            // Try rounding the float to integer
                            let nl = if step < 0 {
                                flimit.ceil()
                            } else {
                                flimit.floor()
                            };
                            // Check if the rounded float fits in i64
                            if nl >= (i64::MIN as f64) && nl < -(i64::MIN as f64) && !nl.is_nan() {
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
                                // Ascending loop with init <= MAX: never skip
                                break 'forlimit (i64::MAX, false);
                            } else {
                                // Negative float out of range (or -inf, NaN)
                                if step > 0 {
                                    // Ascending loop can't reach very negative limit
                                    break 'forlimit (0, true);
                                }
                                // Descending loop: truncate to MININTEGER
                                // Descending loop with init >= MIN: never skip
                                break 'forlimit (i64::MIN, false);
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
                    stack.swap(ra + 3, ra + 2);

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
                        } else {
                            iterator.as_cclosure().map(|cc| cc.func())
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
                        let table_value = *upval.get_value_ref();
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
                            if value.is_collectable()
                                && let Some(gc_ptr) = table_value.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                    }

                    // Slow path: handle metamethods (__newindex)
                    let table_value = *upval.get_value_ref();
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
                    // HOT PATH: inline table length for no-metatable case
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let rb = lua_state.stack_mut()[base + b];
                    if let Some(table) = rb.as_table() {
                        if !table.has_metatable() {
                            setivalue(&mut lua_state.stack_mut()[base + a], table.len() as i64);
                            continue;
                        }
                    } else if let Some(s) = rb.as_str() {
                        setivalue(&mut lua_state.stack_mut()[base + a], s.len() as i64);
                        continue;
                    }
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
                    handle_closure(lua_state, instr, base, frame_idx, chunk, upvalue_ptrs, pc)?;
                }

                OpCode::Vararg => {
                    closure_vararg_ops::exec_vararg(lua_state, instr, base, frame_idx, chunk)?;
                }

                OpCode::GetVarg => {
                    handle_getvarg(lua_state, instr, base, frame_idx)?;
                }

                OpCode::ErrNNil => {
                    handle_errnil(lua_state, instr, base, constants, frame_idx, pc)?;
                }

                OpCode::VarargPrep => {
                    closure_vararg_ops::exec_varargprep(lua_state, frame_idx, chunk, &mut base)?;
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
