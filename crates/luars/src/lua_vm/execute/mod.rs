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
mod hook;
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
        LuaError, LuaResult, LuaState, OpCode,
        call_info::call_status::{CIST_C, CIST_PENDING_FINISH},
        execute::{
            closure_handler::handle_closure,
            cold::{
                handle_call_metamethod, handle_close, handle_errnil, handle_forprep_float,
                handle_getvarg, handle_len, handle_loadkx,
            },
            concat::handle_concat,
            helper::{
                handle_pending_ops, ivalue, lua_fmod, lua_idiv, lua_imod, lua_shiftl, lua_shiftr,
                luai_numpow, pfltvalue, pivalue, psetfltvalue, psetivalue, ptonumberns, pttisfloat,
                pttisinteger, setbfvalue, setbtvalue, setfltvalue, setivalue, setnilvalue,
                tointeger, tointegerns, tonumberns, ttisinteger,
            },
            hook::{hook_check_instruction, hook_on_call, hook_on_return},
        },
        lua_limits::EXTRA_STACK,
    },
};
pub use helper::{get_metamethod_event, get_metatable};
pub use metamethod::TmKind;
pub use metamethod::call_tm_res;

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

        let mut frame_idx = current_depth - 1;
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
        // These are `let mut` so that RETURN1 can restore caller context
        // inline without going through the full 'startfunc reload.
        let ci = lua_state.get_call_info(frame_idx);
        let func_value = ci.func;
        let mut pc = ci.pc as usize;
        let mut base = ci.base;
        let mut chunk_ptr = ci.chunk_ptr;

        let lua_func = unsafe { func_value.as_lua_function_unchecked() };

        // Use cached chunk_ptr from CI (avoids Rc deref on every startfunc entry).
        // chunk_ptr is set by push_lua_frame/push_frame/handle_tailcall for all Lua frames.
        let mut chunk = unsafe { &*chunk_ptr };
        let mut upvalue_ptrs = lua_func.upvalues();
        // Stack already grown by push_lua_frame — no need for grow_stack here.
        // Only the very first entry (top-level chunk) needs this check.
        debug_assert!(lua_state.stack_len() >= base + chunk.max_stack_size + EXTRA_STACK);

        // Cache pointers
        let mut constants = &chunk.constants;
        let mut code = &chunk.code;

        // ===== DEBUG HOOK STATE =====
        // C Lua's trap pattern: a single boolean that controls whether the
        // cold hook-check path runs. Set at function entry from hook_mask,
        // updated only after operations that may change hooks (Protect, jumps,
        // function calls). This keeps the hot loop to a single register test
        // + branch (predicted not-taken), avoiding memory loads for hook_mask,
        // allow_hook, last_line, vm_ptr every instruction.
        let trap = lua_state.hook_mask != 0;

        // Initialise oldpc for this function.
        // New function (pc==0): For vararg functions, VARARGPREP is at
        // instruction 0. C Lua doesn't fire a line event for VARARGPREP
        // (traceexec first fires for instruction 1). Set oldpc = 0 so
        // that npci(0) with changedline(0,0) → same line → no fire.
        // For non-vararg functions, use u32::MAX sentinel to force the
        // first instruction to fire.
        // On resume (pc>0): set to pc-1 (current instruction index).
        lua_state.oldpc = if pc > 0 {
            (pc - 1) as u32 // current instruction index
        } else if chunk.is_vararg {
            0 // skip VARARGPREP line event (matches C Lua)
        } else {
            u32::MAX // sentinel: first instruction always fires
        };

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

        // CALL HOOK: fire when entering a new Lua function (pc == 0)
        if trap && pc == 0 {
            let hook_mask = lua_state.hook_mask;
            if hook_mask & crate::lua_vm::LUA_MASKCALL != 0 && lua_state.allow_hook {
                hook_on_call(lua_state, hook_mask, call_status, chunk)?;
            }
            // Initialise hook_count for count hooks
            if hook_mask & crate::lua_vm::LUA_MASKCOUNT != 0 {
                lua_state.hook_count = lua_state.base_hook_count;
            }
        }

        // MAINLOOP: Main instruction dispatch loop
        loop {
            // Fetch instruction and advance PC
            let instr = unsafe { *code.get_unchecked(pc) };
            pc += 1;

            // ===== DEBUG HOOK CHECK =====
            // C Lua's trap pattern: read hook_mask once per instruction.
            // 3 fewer live local variables vs old approach (no old_mask,
            // last_line, vm_ptr), freeing registers for pc/base/chunk.
            // Cost: one byte load + branch (predicted not-taken) per instruction.
            let trap = lua_state.hook_mask != 0;
            if trap && hook_check_instruction(lua_state, pc, chunk, frame_idx)? {
                // hook_check_instruction returns false if hooks were
                // disabled during the callback — we don't need to act on
                // this since we re-read hook_mask on the next iteration.
            }

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
                    setbfvalue(unsafe { stack.get_unchecked_mut(base + a) });
                }
                OpCode::LFalseSkip => {
                    // R[A] := false; pc++
                    let a = instr.get_a() as usize;
                    let stack = lua_state.stack_mut();
                    setbfvalue(unsafe { stack.get_unchecked_mut(base + a) });
                    pc += 1; // Skip next instruction
                }
                OpCode::LoadTrue => {
                    // R[A] := true
                    let a = instr.get_a() as usize;
                    let stack = lua_state.stack_mut();
                    setbtvalue(unsafe { stack.get_unchecked_mut(base + a) });
                }
                OpCode::LoadNil => {
                    // R[A], R[A+1], ..., R[A+B] := nil
                    let a = instr.get_a() as usize;
                    let mut b = instr.get_b() as usize;

                    let stack = lua_state.stack_mut();
                    let mut idx = base + a;
                    loop {
                        setnilvalue(unsafe { stack.get_unchecked_mut(idx) });
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
                                return Err(cold::error_div_by_zero(lua_state));
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
                                return Err(cold::error_mod_by_zero(lua_state));
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
                    let rb = unsafe { *stack.get_unchecked(base + b) };

                    if ttisinteger(&rb) {
                        let ib = ivalue(&rb);
                        setivalue(
                            unsafe { stack.get_unchecked_mut(base + a) },
                            ib.wrapping_neg(),
                        );
                    } else {
                        let mut nb = 0.0;
                        if tonumberns(&rb, &mut nb) {
                            setfltvalue(unsafe { stack.get_unchecked_mut(base + a) }, -nb);
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
                                return Err(cold::error_mod_by_zero(lua_state));
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
                                return Err(cold::error_div_by_zero(lua_state));
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
                    let v1 = unsafe { stack.get_unchecked(base + b) };
                    let v2 = unsafe { constants.get_unchecked(c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(unsafe { stack.get_unchecked_mut(base + a) }, i1 & i2);
                    }
                }
                OpCode::BOrK => {
                    // R[A] := R[B] | K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = unsafe { stack.get_unchecked(base + b) };
                    let v2 = unsafe { constants.get_unchecked(c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(unsafe { stack.get_unchecked_mut(base + a) }, i1 | i2);
                    }
                }
                OpCode::BXorK => {
                    // R[A] := R[B] ^ K[C] (bitwise xor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = unsafe { stack.get_unchecked(base + b) };
                    let v2 = unsafe { constants.get_unchecked(c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(unsafe { stack.get_unchecked_mut(base + a) }, i1 ^ i2);
                    }
                }
                OpCode::BAnd => {
                    // op_bitwise(L, l_band)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = unsafe { stack.get_unchecked(base + b) };
                    let v2 = unsafe { stack.get_unchecked(base + c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(unsafe { stack.get_unchecked_mut(base + a) }, i1 & i2);
                    }
                }
                OpCode::BOr => {
                    // op_bitwise(L, l_bor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = unsafe { stack.get_unchecked(base + b) };
                    let v2 = unsafe { stack.get_unchecked(base + c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(unsafe { stack.get_unchecked_mut(base + a) }, i1 | i2);
                    }
                }
                OpCode::BXor => {
                    // op_bitwise(L, l_bxor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = unsafe { stack.get_unchecked(base + b) };
                    let v2 = unsafe { stack.get_unchecked(base + c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(unsafe { stack.get_unchecked_mut(base + a) }, i1 ^ i2);
                    }
                }
                OpCode::Shl => {
                    // op_bitwise(L, luaV_shiftl)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = unsafe { stack.get_unchecked(base + b) };
                    let v2 = unsafe { stack.get_unchecked(base + c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(
                            unsafe { stack.get_unchecked_mut(base + a) },
                            lua_shiftl(i1, i2),
                        );
                    }
                }
                OpCode::Shr => {
                    // op_bitwise(L, luaV_shiftr)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = unsafe { stack.get_unchecked(base + b) };
                    let v2 = unsafe { stack.get_unchecked(base + c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(
                            unsafe { stack.get_unchecked_mut(base + a) },
                            lua_shiftr(i1, i2),
                        );
                    }
                }
                OpCode::BNot => {
                    // 按位非: ~value
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let stack = lua_state.stack_mut();
                    let v1 = unsafe { *stack.get_unchecked(base + b) };

                    let mut ib = 0i64;
                    if tointegerns(&v1, &mut ib) {
                        setivalue(unsafe { stack.get_unchecked_mut(base + a) }, !ib);
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
                    let rb = unsafe { stack.get_unchecked(base + b) };

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftl(ic, ib): shift ic left by ib
                        setivalue(
                            unsafe { stack.get_unchecked_mut(base + a) },
                            lua_shiftl(ic as i64, ib),
                        );
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
                    let rb = unsafe { stack.get_unchecked(base + b) };

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftr(ib, ic) = luaV_shiftl(ib, -ic)
                        setivalue(
                            unsafe { stack.get_unchecked_mut(base + a) },
                            lua_shiftr(ib, ic as i64),
                        );
                    }
                    // else: metamethod
                }
                OpCode::Jmp => {
                    // pc += sJ
                    let sj = instr.get_sj();
                    let new_pc = (pc as i32 + sj) as usize;

                    if new_pc >= code.len() {
                        return Err(cold::error_jmp_invalid_pc(lua_state, frame_idx, pc, new_pc));
                    }

                    // Backward jump detection: hook_check_instruction uses
                    // npci <= oldpc which naturally fires on backward jumps.
                    // No need to set oldpc here.
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

                    // Return hook (cold path)
                    if trap
                        && lua_state.hook_mask & crate::lua_vm::LUA_MASKRET != 0
                        && lua_state.allow_hook
                    {
                        let nres = if b != 0 {
                            (b - 1) as i32
                        } else {
                            (lua_state.get_top() - (base + a)) as i32
                        };
                        hook_on_return(lua_state, frame_idx, pc as u32, nres)?;
                    }

                    // Handle return
                    return_handler::handle_return(lua_state, base, frame_idx, a, b, c, k)?;
                    continue 'startfunc;
                }
                OpCode::Return0 => {
                    // return (no values)
                    // Return hook (cold path)
                    if trap
                        && lua_state.hook_mask & crate::lua_vm::LUA_MASKRET != 0
                        && lua_state.allow_hook
                    {
                        hook_on_return(lua_state, frame_idx, pc as u32, 0)?;
                    }
                    return_handler::handle_return0(lua_state, frame_idx);
                    continue 'startfunc;
                }
                OpCode::Return1 => {
                    // return R[A] — hottest return path
                    let a = instr.get_a() as usize;

                    // Return hook (cold path)
                    if trap
                        && lua_state.hook_mask & crate::lua_vm::LUA_MASKRET != 0
                        && lua_state.allow_hook
                    {
                        hook_on_return(lua_state, frame_idx, pc as u32, 1)?;
                    }

                    // Inline return handling + context restore to avoid
                    // the full 'startfunc reload overhead (saves ~12 memory ops
                    // per return, critical for small closures like sort comparators)
                    return_handler::handle_return1(lua_state, base, frame_idx, a);

                    // Check if returned past target depth
                    let new_depth = lua_state.call_depth();
                    if new_depth <= target_depth {
                        return Ok(());
                    }
                    frame_idx = new_depth - 1;

                    // Cold check: C frame or pending finish → full startfunc
                    let cs = lua_state.get_call_info(frame_idx).call_status;
                    if cs & (CIST_C | CIST_PENDING_FINISH) != 0 {
                        continue 'startfunc;
                    }

                    // Hot path: restore caller context directly
                    let ci = lua_state.get_call_info(frame_idx);
                    pc = ci.pc as usize;
                    base = ci.base;
                    chunk_ptr = ci.chunk_ptr;
                    chunk = unsafe { &*chunk_ptr };
                    // Bypass borrow checker: upvalue_ptrs borrows from GC heap,
                    // not from any local. The func is alive on the stack.
                    upvalue_ptrs = unsafe {
                        let lf: *const _ = ci.func.as_lua_function_unchecked();
                        (&*lf).upvalues()
                    };
                    constants = &chunk.constants;
                    code = &chunk.code;
                    // Update oldpc for caller context (rethook equivalent).
                    // Uses pc-1 to match hook_check_instruction's npci = pc-1.
                    lua_state.oldpc = (pc - 1) as u32;
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
                        let extra_instr = unsafe { *code.get_unchecked(pc) };
                        if extra_instr.get_opcode() == OpCode::ExtraArg {
                            vc += extra_instr.get_ax() as usize * 1024;
                        }
                    }

                    pc += 1; // skip EXTRAARG

                    let value = lua_state.create_table(vc, hash_size)?;
                    let stack = lua_state.stack_mut();
                    unsafe { *stack.get_unchecked_mut(base + a) = value };

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

                    let rb;
                    let rc;
                    unsafe {
                        let sp = lua_state.stack_mut().as_ptr();
                        rb = *sp.add(base + b);
                        rc = *sp.add(base + c);
                    }

                    // Inline fast path: table[key]
                    if let Some(table_ref) = rb.as_table() {
                        let result = if rc.ttisinteger() {
                            table_ref.impl_table.fast_geti(rc.ivalue())
                        } else {
                            table_ref.impl_table.raw_get(&rc)
                        };
                        if let Some(val) = result {
                            unsafe {
                                *lua_state.stack_mut().as_mut_ptr().add(base + a) = val;
                            }
                            continue;
                        }
                        // Key not found — check metatable for __index
                        let meta = table_ref.meta_ptr();
                        if meta.is_null() {
                            unsafe {
                                *lua_state.stack_mut().as_mut_ptr().add(base + a) = LuaValue::nil();
                            }
                            continue;
                        }
                        let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
                        if mt.no_tm(TmKind::Index as u8) {
                            unsafe {
                                *lua_state.stack_mut().as_mut_ptr().add(base + a) = LuaValue::nil();
                            }
                            continue;
                        }
                        let event_key =
                            lua_state.vm_mut().const_strings.get_tm_value(TmKind::Index);
                        if let Some(tm) = mt.impl_table.get_shortstr_fast(&event_key) {
                            // __index is a table: direct lookup (no function call)
                            if let Some(fallback) = tm.as_table() {
                                let res = if rc.ttisinteger() {
                                    fallback.impl_table.fast_geti(rc.ivalue())
                                } else if rc.is_short_string() {
                                    fallback.impl_table.get_shortstr_fast(&rc)
                                } else {
                                    fallback.raw_get(&rc)
                                };
                                if let Some(val) = res {
                                    unsafe {
                                        *lua_state.stack_mut().as_mut_ptr().add(base + a) = val;
                                    }
                                    continue;
                                }
                                // Key not found in fallback table — it may have
                                // its own __index chain, fall through to slow path.
                            }
                            // __index is a function: call directly (skip exec_gettable overhead)
                            if tm.is_function() {
                                save_pc!();
                                let ci_top = lua_state.get_call_info(frame_idx).top;
                                lua_state.set_top_raw(ci_top);
                                match metamethod::call_tm_res(lua_state, tm, rb, rc) {
                                    Ok(result) => {
                                        base = lua_state.get_frame_base(frame_idx);
                                        unsafe {
                                            *lua_state.stack_mut().as_mut_ptr().add(base + a) =
                                                result;
                                        }
                                        continue;
                                    }
                                    Err(LuaError::Yield) => {
                                        let ci = lua_state.get_call_info_mut(frame_idx);
                                        ci.pending_finish_get = a as i32;
                                        ci.call_status |= CIST_PENDING_FINISH;
                                        return Err(LuaError::Yield);
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
                            // __index is something else — fall through to slow path
                        } else {
                            mt.set_tm_absent(TmKind::Index as u8);
                            unsafe {
                                *lua_state.stack_mut().as_mut_ptr().add(base + a) = LuaValue::nil();
                            }
                            continue;
                        }
                        // Has __index — fall through to slow path
                    }

                    // Slow path: non-table value or unusual __index chain
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
                        if let Some(val) = table_ref.impl_table.get_shortstr_fast(key) {
                            unsafe {
                                *stack.get_unchecked_mut(base + a) = val;
                            }
                            continue;
                        }
                        // Key not found — check metatable __index (one level inline)
                        let meta = table_ref.meta_ptr();
                        if meta.is_null() {
                            unsafe {
                                *stack.get_unchecked_mut(base + a) = LuaValue::nil();
                            }
                            continue;
                        }
                        let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
                        if mt.no_tm(TmKind::Index as u8) {
                            unsafe {
                                *stack.get_unchecked_mut(base + a) = LuaValue::nil();
                            }
                            continue;
                        }
                        // Get __index value from metatable
                        let event_key =
                            lua_state.vm_mut().const_strings.get_tm_value(TmKind::Index);
                        if let Some(tm) = mt.impl_table.get_shortstr_fast(&event_key) {
                            // __index is a table: direct one-level lookup
                            if let Some(fallback) = tm.as_table() {
                                if let Some(val) = fallback.impl_table.get_shortstr_fast(key) {
                                    unsafe {
                                        *lua_state.stack_mut().as_mut_ptr().add(base + a) = val;
                                    }
                                    continue;
                                }
                                // Deep chain: continue from tm (cold path)
                                save_pc!();
                                table_ops::self_deep_chain(lua_state, tm, key, a, frame_idx)?;
                                restore_state!();
                                continue;
                            }
                            // __index is a function — fall through
                        } else {
                            mt.set_tm_absent(TmKind::Index as u8);
                            unsafe {
                                *lua_state.stack_mut().as_mut_ptr().add(base + a) = LuaValue::nil();
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
                    let ra = unsafe { *stack.get_unchecked(base + a) };
                    let rb = unsafe { *stack.get_unchecked(base + b) };
                    let val = if k {
                        unsafe { *constants.get_unchecked(c) }
                    } else {
                        unsafe { *stack.get_unchecked(base + c) }
                    };

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
                                let delta = table_ref.impl_table.set_int_slow(rb.ivalue(), val);
                                if delta != 0
                                    && let Some(table_ptr) = ra.as_table_ptr()
                                {
                                    lua_state.gc_track_table_resize(table_ptr, delta);
                                }
                                if val.is_collectable()
                                    && let Some(gc_ptr) = ra.as_gc_ptr()
                                {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            // Non-integer key: validate then raw_set
                            if rb.is_nil() {
                                return Err(cold::error_table_index_nil(lua_state));
                            }
                            if rb.ttisfloat() && rb.fltvalue().is_nan() {
                                return Err(cold::error_table_index_nan(lua_state));
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
                        // Key doesn't exist — inline fasttm for __newindex
                        let meta = table_ref.meta_ptr();
                        if !meta.is_null() {
                            let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
                            const TM_NEWINDEX_BIT: u8 = TmKind::NewIndex as u8;
                            if mt.no_tm(TM_NEWINDEX_BIT) {
                                // No __newindex — set directly
                                lua_state.raw_set(&ra, rb, val);
                                continue;
                            }
                            let event_key = lua_state
                                .vm_mut()
                                .const_strings
                                .get_tm_value(TmKind::NewIndex);
                            if let Some(tm) = mt.impl_table.get_shortstr_fast(&event_key) {
                                if tm.is_function() {
                                    // __newindex is a function: call directly
                                    save_pc!();
                                    let ci_top = lua_state.get_call_info(frame_idx).top;
                                    lua_state.set_top_raw(ci_top);
                                    match metamethod::call_tm(lua_state, tm, ra, rb, val) {
                                        Ok(_) => {
                                            let ci_top2 = lua_state.get_call_info(frame_idx).top;
                                            lua_state.set_top_raw(ci_top2);
                                            continue;
                                        }
                                        Err(LuaError::Yield) => {
                                            let ci = lua_state.get_call_info_mut(frame_idx);
                                            ci.pending_finish_get = -2;
                                            ci.call_status |= CIST_PENDING_FINISH;
                                            return Err(LuaError::Yield);
                                        }
                                        Err(e) => return Err(e),
                                    }
                                }
                                // __newindex is a table — fall through to finishset
                            } else {
                                mt.set_tm_absent(TM_NEWINDEX_BIT);
                                lua_state.raw_set(&ra, rb, val);
                                continue;
                            }
                        } else {
                            // No metatable — set directly
                            lua_state.raw_set(&ra, rb, val);
                            continue;
                        }
                    }

                    // Slow path: metamethod chain or non-table
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
                    let ra = unsafe { *stack.get_unchecked(base + a) };
                    let value = if k {
                        unsafe { *constants.get_unchecked(c) }
                    } else {
                        unsafe { *stack.get_unchecked(base + c) }
                    };

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
                            let delta = table_ref.impl_table.set_int_slow(b as i64, value);
                            if delta != 0
                                && let Some(table_ptr) = ra.as_table_ptr()
                            {
                                lua_state.gc_track_table_resize(table_ptr, delta);
                            }
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
                    let ra = unsafe { *stack.get_unchecked(base + a) };
                    let key = unsafe { constants.get_unchecked(b) };
                    let value = if k {
                        unsafe { *constants.get_unchecked(c) }
                    } else {
                        unsafe { *stack.get_unchecked(base + c) }
                    };

                    // Try fast path: fast_setfield only succeeds when the key
                    // already exists with a non-nil value. Per Lua semantics,
                    // __newindex is NEVER consulted when the key already exists
                    // in the table's own hash part. So this is safe regardless
                    // of whether the table has a metatable.
                    if let Some(table_ref) = ra.as_table_mut() {
                        if table_ref.impl_table.fast_setfield(key, value) {
                            // Existing key updated — GC write barrier
                            if value.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                        // Key doesn't exist yet. If no metatable, use fast new-key path
                        // to avoid exec_setfield function call overhead.
                        if !table_ref.has_metatable() {
                            if table_ref.impl_table.fast_setfield_newkey(key, value) {
                                // Must invalidate TM cache: this table may be used as
                                // a metatable for other tables (e.g. mt.__eq = func).
                                table_ref.invalidate_tm_cache();
                                if value.is_collectable()
                                    && let Some(gc_ptr) = ra.as_gc_ptr()
                                {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            // Needs rehash — use raw_set directly
                            let (_, delta) = table_ref.impl_table.raw_set(key, value);
                            table_ref.invalidate_tm_cache();
                            if delta != 0
                                && let Some(table_ptr) = ra.as_table_ptr()
                            {
                                lua_state.gc_track_table_resize(table_ptr, delta);
                            }
                            if value.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                    }

                    // Slow path: metamethod, non-table, or has metatable with new key
                    save_pc!();
                    table_ops::exec_setfield(
                        lua_state, instr, constants, base, frame_idx, &mut pc,
                    )?;
                    restore_state!();
                }
                OpCode::Self_ => {
                    // SELF: R[A+1] := R[B]; R[A] := R[B][K[C]:string]
                    // HOT PATH: Inline fast path for OOP method lookup
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let key = unsafe { constants.get_unchecked(c) };
                    let rb;
                    unsafe {
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        rb = *sp.add(base + b);
                        // R[A+1] := R[B] (save object for method call)
                        *sp.add(base + a + 1) = rb;
                    }

                    // Fast path: rb is a table
                    if let Some(table_ref) = rb.as_table() {
                        // Try direct lookup (key is always a short string constant)
                        if let Some(val) = table_ref.impl_table.get_shortstr_fast(key) {
                            unsafe {
                                *lua_state.stack_mut().as_mut_ptr().add(base + a) = val;
                            }
                            continue;
                        }
                        // Key not found — check metatable __index (one level)
                        let meta = table_ref.meta_ptr();
                        if meta.is_null() {
                            unsafe {
                                *lua_state.stack_mut().as_mut_ptr().add(base + a) = LuaValue::nil();
                            }
                            continue;
                        }
                        let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
                        if mt.no_tm(TmKind::Index as u8) {
                            unsafe {
                                *lua_state.stack_mut().as_mut_ptr().add(base + a) = LuaValue::nil();
                            }
                            continue;
                        }
                        let event_key =
                            lua_state.vm_mut().const_strings.get_tm_value(TmKind::Index);
                        if let Some(tm) = mt.impl_table.get_shortstr_fast(&event_key) {
                            // __index is a table: try one-level lookup
                            if let Some(fallback_table) = tm.as_table() {
                                if let Some(val) = fallback_table.impl_table.get_shortstr_fast(key)
                                {
                                    unsafe {
                                        *lua_state.stack_mut().as_mut_ptr().add(base + a) = val;
                                    }
                                    continue;
                                }
                                // Deep chain: continue from tm (cold path,
                                // avoids redundant lookups in exec_self)
                                save_pc!();
                                table_ops::self_deep_chain(lua_state, tm, key, a, frame_idx)?;
                                restore_state!();
                                continue;
                            }
                            // __index is a function or unusual — fall through
                        } else {
                            mt.set_tm_absent(TmKind::Index as u8);
                            unsafe {
                                *lua_state.stack_mut().as_mut_ptr().add(base + a) = LuaValue::nil();
                            }
                            continue;
                        }
                    }

                    // Slow path: non-table receiver or __index function
                    save_pc!();
                    table_ops::exec_self(lua_state, instr, constants, base, frame_idx, &mut pc)?;
                    restore_state!();
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
                        let new_chunk = lua_func.chunk();

                        save_pc!();
                        lua_state.push_lua_frame(
                            &func,
                            func_idx + 1,
                            nargs,
                            nresults,
                            new_chunk.param_count,
                            new_chunk.max_stack_size,
                            new_chunk as *const _,
                        )?;

                        // Inline callee entry: restore context without startfunc
                        frame_idx = lua_state.call_depth() - 1;
                        let ci = lua_state.get_call_info(frame_idx);
                        pc = ci.pc as usize;
                        base = ci.base;
                        chunk_ptr = ci.chunk_ptr;
                        chunk = unsafe { &*chunk_ptr };
                        upvalue_ptrs = unsafe {
                            let lf: *const _ = ci.func.as_lua_function_unchecked();
                            (&*lf).upvalues()
                        };
                        constants = &chunk.constants;
                        code = &chunk.code;

                        // Call hook for inline Lua call (cold path)
                        if trap
                            && lua_state.hook_mask & crate::lua_vm::LUA_MASKCALL != 0
                            && lua_state.allow_hook
                        {
                            hook_on_call(lua_state, lua_state.hook_mask, 0, chunk)?;
                        }
                        // Init oldpc for new function
                        lua_state.oldpc = if chunk.is_vararg { 0 } else { u32::MAX };
                        continue;
                    }

                    // Semi-cold path: __call metamethod on table
                    // Extracted to cold function to reduce lua_execute code size
                    // and register pressure in the hot loop.
                    if func.ttistable() {
                        save_pc!();
                        if handle_call_metamethod(lua_state, func, func_idx, b, c)? {
                            continue 'startfunc;
                        }
                    }

                    // Cold path: C function or non-table __call metamethod
                    save_pc!();
                    match call::handle_call(lua_state, base, a, b, c, 0) {
                        Ok(FrameAction::Continue) => {
                            restore_state!();
                            // rethook: update oldpc for caller
                            lua_state.oldpc = (pc - 1) as u32;
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

                    // NOTE: No return hook here. Lua 5.5's luaD_pretailcall does NOT
                    // call rethook for Lua-to-Lua tail calls. Only the "tail call" event
                    // fires via the call hook at the new function's entry (pc==0).
                    // For C function tail calls, hooks are handled inside call_c_function.

                    // Delegate to tailcall handler
                    // (call hook fires via 'startfunc when pc==0, with LUA_HOOKTAILCALL event)
                    match call::handle_tailcall(lua_state, base, a, b) {
                        Ok(FrameAction::Continue) => {
                            // C tail call returned
                            restore_state!();
                            lua_state.oldpc = (pc - 1) as u32;
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
                    let rb = unsafe { stack.get_unchecked(base + b) };

                    // l_isfalse: nil or false
                    let is_false = rb.tt() == LUA_VFALSE || rb.is_nil();
                    if is_false {
                        setbtvalue(unsafe { stack.get_unchecked_mut(base + a) });
                    } else {
                        setbfvalue(unsafe { stack.get_unchecked_mut(base + a) });
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
                                    return Err(cold::error_forloop_invalid_jump(
                                        lua_state, frame_idx, pc,
                                    ));
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

                    // Convert string values to numbers (Lua 5.5 allows string for-loop params)
                    for offset in 0..3 {
                        let val = unsafe { *stack.get_unchecked(ra + offset) };
                        if val.is_string() {
                            let num = crate::stdlib::basic::parse_number::parse_lua_number(
                                val.as_str().unwrap_or(""),
                            );
                            if !num.is_nil() {
                                unsafe { *stack.get_unchecked_mut(ra + offset) = num };
                            }
                            // If conversion fails, leave the string — error will be reported below
                        }
                    }

                    if ttisinteger(unsafe { stack.get_unchecked(ra) })
                        && ttisinteger(unsafe { stack.get_unchecked(ra + 2) })
                    {
                        // Integer loop (init and step are integers)
                        let init = ivalue(unsafe { stack.get_unchecked(ra) });
                        let step = ivalue(unsafe { stack.get_unchecked(ra + 2) });

                        if step == 0 {
                            save_pc!();
                            return Err(cold::error_for_step_zero(lua_state));
                        }

                        // forlimit: convert limit to integer per C Lua 5.5 logic
                        let (limit, should_skip) = 'forlimit: {
                            // Try integer limit directly
                            if ttisinteger(unsafe { stack.get_unchecked(ra + 1) }) {
                                let lim = ivalue(unsafe { stack.get_unchecked(ra + 1) });
                                let skip = if step > 0 { init > lim } else { init < lim };
                                break 'forlimit (lim, skip);
                            }
                            // Try converting to float (handles float and string)
                            let mut flimit = 0.0;
                            let limit_val = unsafe { *stack.get_unchecked(ra + 1) }; // Copy to avoid borrow conflict
                            if !tonumberns(&limit_val, &mut flimit) {
                                save_pc!();
                                return Err(cold::error_for_bad_limit(
                                    lua_state, frame_idx, pc, &limit_val,
                                ));
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

                            setivalue(unsafe { stack.get_unchecked_mut(ra) }, count as i64);
                            setivalue(unsafe { stack.get_unchecked_mut(ra + 1) }, step);
                            setivalue(unsafe { stack.get_unchecked_mut(ra + 2) }, init);
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
                        // rethook: update oldpc after C function returns
                        lua_state.oldpc = (pc - 1) as u32;
                    } else {
                        // Slow path: Lua function or __call metamethod
                        match call::handle_call(lua_state, base, a + 3, 3, c + 1, 0) {
                            Ok(FrameAction::Continue) => {
                                restore_state!();
                                lua_state.oldpc = (pc - 1) as u32;
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

                    // Fast path: v1 is a table with a Lua metamethod.
                    // Non-recursive: push the metamethod frame and continue
                    // the main loop instead of call_tm_res → lua_execute recursion.
                    // Result is delivered via handle_pending_ops on RETURN.
                    let base_mm = lua_state.get_frame_base(frame_idx);
                    let v1 = unsafe { *lua_state.stack().get_unchecked(base_mm + a) };
                    if v1.ttistable() {
                        let table = unsafe { &mut *(v1.value.ptr as *mut GcTable) };
                        let meta = table.data.meta_ptr();
                        if !meta.is_null() {
                            let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
                            let tm_idx = c as u8;
                            if !mt.no_tm(tm_idx) {
                                let tm_kind = unsafe { TmKind::from_u8_unchecked(tm_idx) };
                                let event_key =
                                    lua_state.vm_mut().const_strings.get_tm_value(tm_kind);
                                if let Some(mm) = mt.impl_table.get_shortstr_fast(&event_key) {
                                    let v2 =
                                        unsafe { *lua_state.stack().get_unchecked(base_mm + b) };

                                    if mm.is_lua_function() {
                                        // Non-recursive Lua metamethod call
                                        save_pc!();
                                        cold::push_lua_mm_frame(lua_state, mm, v1, v2, frame_idx)?;
                                        continue 'startfunc;
                                    }

                                    // C function metamethod — use cold helper
                                    let pi = unsafe { *code.get_unchecked(pc - 2) };
                                    let result_reg = pi.get_a() as usize;
                                    save_pc!();
                                    cold::call_c_mm_bin(
                                        lua_state, mm, v1, v2, result_reg, frame_idx,
                                    )?;
                                    restore_state!();
                                    continue;
                                } else {
                                    mt.set_tm_absent(tm_idx);
                                }
                            }
                        }
                    }

                    // Slow path: string coercion, userdata, v2 metamethods
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
                    let key = unsafe { constants.get_unchecked(c) };
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
                        unsafe { *stack.get_unchecked_mut(base + a) = val };
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
                                unsafe {
                                    *stack.get_unchecked_mut(base + a) =
                                        result.unwrap_or(LuaValue::nil())
                                };
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

                    let key = unsafe { *constants.get_unchecked(b) };
                    let value = if k {
                        unsafe { *constants.get_unchecked(c) }
                    } else {
                        unsafe { *lua_state.stack_mut().get_unchecked(base + c) }
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
                    let rb = unsafe { *lua_state.stack_mut().get_unchecked(base + b) };
                    if rb.ttistable() {
                        let table_gc = unsafe { &mut *(rb.value.ptr as *mut GcTable) };
                        let table = &mut table_gc.data;
                        let meta = table.meta_ptr();
                        if meta.is_null() {
                            // No metatable — raw len
                            setivalue(
                                unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                                table.len() as i64,
                            );
                            continue;
                        }
                        // Has metatable — inline fasttm check for __len
                        let mt = unsafe { &mut (*meta.as_mut_ptr()).data };
                        const TM_LEN_BIT: u8 = TmKind::Len as u8;
                        if mt.no_tm(TM_LEN_BIT) {
                            // __len cached absent — raw len
                            setivalue(
                                unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                                table.len() as i64,
                            );
                            continue;
                        }
                        let event_key = lua_state.vm_mut().const_strings.get_tm_value(TmKind::Len);
                        if let Some(mm) = mt.impl_table.get_shortstr_fast(&event_key) {
                            if mm.is_lua_function() {
                                // Non-recursive __len: push metamethod frame
                                save_pc!();
                                cold::push_lua_mm_frame(lua_state, mm, rb, rb, frame_idx)?;
                                continue 'startfunc;
                            }
                            // __len is not Lua function — fall to handle_len
                        } else {
                            // __len absent — cache and use raw len
                            mt.set_tm_absent(TM_LEN_BIT);
                            setivalue(
                                unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                                table.len() as i64,
                            );
                            continue;
                        }
                    } else if let Some(s) = rb.as_str() {
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            s.len() as i64,
                        );
                        continue;
                    }
                    handle_len(lua_state, instr, &mut base, frame_idx, pc)?;
                }

                OpCode::Concat => {
                    // Match C Lua: set L->top.p = ra + n before concat/checkGC.
                    // This ensures the GC marks all registers up to the concat
                    // range, preventing atomic-phase clearing from nil'ing out
                    // temp registers (like a function copy) below the operands.
                    let a = instr.get_a() as usize;
                    let n = instr.get_b() as usize;
                    let concat_top = base + a + n;
                    if concat_top > lua_state.get_top() {
                        lua_state.set_top_raw(concat_top);
                    }
                    handle_concat(lua_state, instr, &mut base, frame_idx, pc)?;
                }

                // ============================================================
                // COMPARISON OPERATIONS (register-register)
                // ============================================================
                OpCode::Eq => {
                    comparison_ops::exec_eq(lua_state, instr, base, frame_idx, &mut pc)?;
                }

                OpCode::Lt => {
                    // LT fast path: inline integer/float comparison
                    // Avoids exec_lt function call overhead for the common case
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();

                    let stack = lua_state.stack();
                    let ra = unsafe { *stack.get_unchecked(base + a) };
                    let rb = unsafe { *stack.get_unchecked(base + b) };

                    if ra.ttisinteger() && rb.ttisinteger() {
                        let cond = ra.ivalue() < rb.ivalue();
                        if cond != k {
                            pc += 1;
                        }
                    } else if ra.ttisfloat() && rb.ttisfloat() {
                        let cond = ra.fltvalue() < rb.fltvalue();
                        if cond != k {
                            pc += 1;
                        }
                    } else {
                        comparison_ops::exec_lt(lua_state, instr, base, frame_idx, &mut pc)?;
                    }
                }

                OpCode::Le => {
                    // LE fast path: inline integer/float comparison
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();

                    let stack = lua_state.stack();
                    let ra = unsafe { *stack.get_unchecked(base + a) };
                    let rb = unsafe { *stack.get_unchecked(base + b) };

                    if ra.ttisinteger() && rb.ttisinteger() {
                        let cond = ra.ivalue() <= rb.ivalue();
                        if cond != k {
                            pc += 1;
                        }
                    } else if ra.ttisfloat() && rb.ttisfloat() {
                        let cond = ra.fltvalue() <= rb.fltvalue();
                        if cond != k {
                            pc += 1;
                        }
                    } else {
                        comparison_ops::exec_le(lua_state, instr, base, frame_idx, &mut pc)?;
                    }
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
                    let rb = unsafe { stack.get_unchecked(base + b) };
                    let is_false = rb.is_nil() || (rb.is_boolean() && rb.tt() == LUA_VFALSE);

                    if is_false == k {
                        pc += 1; // Condition failed - skip next instruction (JMP)
                    } else {
                        // Condition succeeded - copy value and EXECUTE next instruction (must be JMP)
                        unsafe { *stack.get_unchecked_mut(base + a) = *rb };
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
                    // After VARARGPREP, reset oldpc to sentinel so the first
                    // real instruction always fires a line event.
                    // Our absolute line_info makes changedline(0, 1) return
                    // false when VARARGPREP and the next instruction share a
                    // line. The sentinel forces unconditional fire.
                    if trap {
                        lua_state.oldpc = u32::MAX;
                    }
                }

                OpCode::ExtraArg => {
                    // Extra argument for previous opcode
                    // This instruction should never be executed directly
                    // It's always consumed by the previous instruction (NEWTABLE, SETLIST, etc.)
                    // If we reach here, it's a compiler error
                    save_pc!();
                    return Err(cold::error_unexpected_extraarg(lua_state));
                }
            } // end match
        } // end 'mainloop
    } // end 'startfunc
}
