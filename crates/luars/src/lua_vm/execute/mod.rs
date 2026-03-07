/*----------------------------------------------------------------------
  Lua 5.5 VM Execution Engine - Slice-Based High-Performance Implementation

  Design Philosophy:
  1. **Slice-Based**: Code and constants accessed via `&[T]` slices with
     `noalias` guarantees — LLVM keeps slice base pointers in registers
     across function calls (raw pointers must be reloaded after `&mut` calls)
  2. **Minimal Indirection**: Use get_unchecked for stack access (no bounds checks)
  3. **No Allocation in Loop**: All errors via lua_state.error(), no String construction
  4. **CPU Register Optimization**: code, constants, pc, base, trap in CPU registers
  5. **Unsafe but Sound**: Use raw pointers with invariant guarantees for stack

  Key Invariants (maintained by caller):
  - Stack pointer valid throughout execution (no reallocation)
  - CallInfo valid and matches current frame
  - Chunk lifetime extends through execution
  - base + register < stack.len() (validated at call time)

  This leverages Rust's type system for LLVM optimization opportunities
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
mod noinline;
mod table_ops;

use call::FrameAction;

use crate::{
    GcTable,
    lua_value::{LUA_VFALSE, LUA_VTABLE, LuaValue},
    lua_vm::{
        Instruction, LuaError, LuaResult, LuaState, OpCode,
        call_info::call_status::{CIST_C, CIST_PENDING_FINISH, CIST_TAIL},
        execute::{
            closure_handler::handle_closure,
            cold::{
                handle_call_metamethod, handle_close, handle_errnil, handle_getvarg, handle_len,
                handle_loadkx,
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
        // Register pressure optimization: only 5 `let mut` loop variables
        // (base, pc, code, constants, frame_idx) to fit in callee-saved GPRs.
        // chunk, upvalue_ptrs, and trap are derived on-demand from lua_state
        // to reduce live ranges and help LLVM's register allocator.
        let ci = lua_state.get_call_info(frame_idx);
        let mut base = ci.base;
        let pc_init = ci.pc as usize;
        let chunk_raw = ci.chunk_ptr;

        // Derive code/constants directly without keeping chunk as loop variable.
        // chunk is only needed on cold paths (hooks, closures, varargs).
        let chunk_init = unsafe { &*chunk_raw };
        debug_assert!(lua_state.stack_len() >= base + chunk_init.max_stack_size + EXTRA_STACK);

        let mut code: &[Instruction] = &chunk_init.code;
        let mut constants: &[LuaValue] = &chunk_init.constants;
        let mut pc: usize = pc_init;

        lua_state.oldpc = if pc_init > 0 {
            (pc_init - 1) as u32
        } else if chunk_init.is_vararg {
            0
        } else {
            u32::MAX
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

        // Macro to derive chunk from current frame (cold path only).
        macro_rules! current_chunk {
            () => {
                unsafe { &*lua_state.get_call_info(frame_idx).chunk_ptr }
            };
        }

        // Macro to get upvalue pointer from current frame's cached field.
        // The pointer is pre-computed in push_lua_frame, avoiding the
        // func → GcPtr → GcRClosure → LuaFunction → UpvalueStore enum match
        // chain on every access (saves 2-3 loads + 1 branch per GetUpval/SetUpval).
        macro_rules! current_upvalue_ptrs {
            () => {
                lua_state.get_call_info(frame_idx).upvalue_ptrs
            };
        }

        // CALL HOOK: fire when entering a new Lua function (pc == 0)
        // trap: local hook flag matching C Lua's `int trap` pattern.
        // Kept as local bool so LLVM can place it in a register / consistent
        // stack slot without perturbing jump-table register allocation.
        let mut trap = lua_state.hook_mask != 0;
        if pc == 0 && trap {
            let hook_mask = lua_state.hook_mask;
            if hook_mask & crate::lua_vm::LUA_MASKCALL != 0 && lua_state.allow_hook {
                hook_on_call(lua_state, hook_mask, call_status, chunk_init)?;
            }
            if hook_mask & crate::lua_vm::LUA_MASKCOUNT != 0 {
                lua_state.hook_count = lua_state.base_hook_count;
            }
        }

        // Macro to re-sync trap from lua_state after hook-related calls.
        macro_rules! updatetrap {
            () => {
                trap = lua_state.hook_mask != 0;
            };
        }

        // MAINLOOP: Main instruction dispatch loop
        loop {
            let instr = unsafe { *code.get_unchecked(pc) };
            pc += 1;

            if trap {
                let chunk_ref = current_chunk!();
                hook_check_instruction(lua_state, pc, chunk_ref, frame_idx)?;
                updatetrap!();
            }

            // Dispatch instruction (continues in next replacement...)
            match instr.get_opcode() {
                OpCode::Move => {
                    // R[A] := R[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    unsafe {
                        let stack = lua_state.stack_mut();
                        *stack.get_unchecked_mut(base + a) = *stack.get_unchecked(base + b);
                    }
                }
                OpCode::LoadI => {
                    // R[A] := sBx
                    let a = instr.get_a() as usize;
                    let sbx = instr.get_sbx();
                    unsafe {
                        *lua_state.stack_mut().get_unchecked_mut(base + a) =
                            LuaValue::integer(sbx as i64);
                    }
                }
                OpCode::LoadF => {
                    // R[A] := (float)sBx
                    let a = instr.get_a() as usize;
                    let sbx = instr.get_sbx();
                    unsafe {
                        *lua_state.stack_mut().get_unchecked_mut(base + a) =
                            LuaValue::float(sbx as f64);
                    }
                }
                OpCode::LoadK => {
                    // R[A] := K[Bx]
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;
                    unsafe {
                        *lua_state.stack_mut().get_unchecked_mut(base + a) =
                            *constants.get_unchecked(bx);
                    }
                }
                OpCode::LoadKX => {
                    let mut pc_idx = pc;
                    handle_loadkx(
                        lua_state,
                        instr,
                        base,
                        frame_idx,
                        code,
                        constants,
                        &mut pc_idx,
                    )?;
                    pc = pc_idx;
                }
                OpCode::LoadFalse => {
                    // R[A] := false
                    let a = instr.get_a() as usize;
                    setbfvalue(unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) });
                }
                OpCode::LFalseSkip => {
                    // R[A] := false; pc++
                    let a = instr.get_a() as usize;
                    setbfvalue(unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) });
                    pc += 1; // Skip next instruction
                }
                OpCode::LoadTrue => {
                    // R[A] := true
                    let a = instr.get_a() as usize;
                    setbtvalue(unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) });
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

                        if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                            psetivalue(ra_ptr, pivalue(v1_ptr).wrapping_add(pivalue(v2_ptr)));
                            pc += 1;
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) + pfltvalue(v2_ptr));
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
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) - pfltvalue(v2_ptr));
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
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) * pfltvalue(v2_ptr));
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

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) / pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 / n2);
                                pc += 1;
                            }
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
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, (pfltvalue(v1_ptr) / pfltvalue(v2_ptr)).floor());
                            pc += 1;
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
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, lua_fmod(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
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

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, luai_numpow(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, luai_numpow(n1, n2));
                                pc += 1;
                            }
                        }
                    }
                }
                OpCode::Unm => {
                    // 取负: -value
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let rb = unsafe { *lua_state.stack().get_unchecked(base + b) };

                    if ttisinteger(&rb) {
                        let ib = ivalue(&rb);
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            ib.wrapping_neg(),
                        );
                    } else {
                        let mut nb = 0.0;
                        if tonumberns(&rb, &mut nb) {
                            setfltvalue(
                                unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                                -nb,
                            );
                        } else {
                            // Try non-recursive __unm for tables/userdata
                            save_pc!();
                            if cold::try_push_unary_mm_frame(
                                lua_state,
                                rb,
                                metamethod::TmKind::Unm,
                                frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                            // Fall through to recursive path (C function mm or error)
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
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) + pfltvalue(v2_ptr));
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
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) - pfltvalue(v2_ptr));
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
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) * pfltvalue(v2_ptr));
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
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, lua_fmod(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
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

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, luai_numpow(pfltvalue(v1_ptr), pfltvalue(v2_ptr)));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, luai_numpow(n1, n2));
                                pc += 1;
                            }
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

                        if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, pfltvalue(v1_ptr) / pfltvalue(v2_ptr));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1_ptr, &mut n1) && ptonumberns(v2_ptr, &mut n2) {
                                psetfltvalue(ra_ptr, n1 / n2);
                                pc += 1;
                            }
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
                        } else if pttisfloat(v1_ptr) && pttisfloat(v2_ptr) {
                            psetfltvalue(ra_ptr, (pfltvalue(v1_ptr) / pfltvalue(v2_ptr)).floor());
                            pc += 1;
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

                    let v1 = unsafe { lua_state.stack().get_unchecked(base + b) };
                    let v2 = unsafe { constants.get_unchecked(c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            i1 & i2,
                        );
                    }
                }
                OpCode::BOrK => {
                    // R[A] := R[B] | K[C]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = unsafe { lua_state.stack().get_unchecked(base + b) };
                    let v2 = unsafe { constants.get_unchecked(c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            i1 | i2,
                        );
                    }
                }
                OpCode::BXorK => {
                    // R[A] := R[B] ^ K[C] (bitwise xor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = unsafe { lua_state.stack().get_unchecked(base + b) };
                    let v2 = unsafe { constants.get_unchecked(c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            i1 ^ i2,
                        );
                    }
                }
                OpCode::BAnd => {
                    // op_bitwise(L, l_band)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = unsafe { lua_state.stack().get_unchecked(base + b) };
                    let v2 = unsafe { lua_state.stack().get_unchecked(base + c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            i1 & i2,
                        );
                    }
                }
                OpCode::BOr => {
                    // op_bitwise(L, l_bor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = unsafe { lua_state.stack().get_unchecked(base + b) };
                    let v2 = unsafe { lua_state.stack().get_unchecked(base + c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            i1 | i2,
                        );
                    }
                }
                OpCode::BXor => {
                    // op_bitwise(L, l_bxor)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = unsafe { lua_state.stack().get_unchecked(base + b) };
                    let v2 = unsafe { lua_state.stack().get_unchecked(base + c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            i1 ^ i2,
                        );
                    }
                }
                OpCode::Shl => {
                    // op_bitwise(L, luaV_shiftl)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = unsafe { lua_state.stack().get_unchecked(base + b) };
                    let v2 = unsafe { lua_state.stack().get_unchecked(base + c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            lua_shiftl(i1, i2),
                        );
                    }
                }
                OpCode::Shr => {
                    // op_bitwise(L, luaV_shiftr)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let v1 = unsafe { lua_state.stack().get_unchecked(base + b) };
                    let v2 = unsafe { lua_state.stack().get_unchecked(base + c) };

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            lua_shiftr(i1, i2),
                        );
                    }
                }
                OpCode::BNot => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let v1 = unsafe { *lua_state.stack().get_unchecked(base + b) };

                    let mut ib = 0i64;
                    if tointegerns(&v1, &mut ib) {
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            !ib,
                        );
                    } else {
                        // Try non-recursive __bnot for tables/userdata
                        save_pc!();
                        if cold::try_push_unary_mm_frame(
                            lua_state,
                            v1,
                            metamethod::TmKind::Bnot,
                            frame_idx,
                        )? {
                            continue 'startfunc;
                        }
                        // Fall through to recursive path (C function mm or error)
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

                    let rb = unsafe { lua_state.stack().get_unchecked(base + b) };

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftl(ic, ib): shift ic left by ib
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
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

                    let rb = unsafe { lua_state.stack().get_unchecked(base + b) };

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftr(ib, ic) = luaV_shiftl(ib, -ic)
                        setivalue(
                            unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                            lua_shiftr(ib, ic as i64),
                        );
                    }
                    // else: metamethod
                }
                OpCode::Jmp => {
                    // pc += sJ (pointer arithmetic, like C Lua)
                    let sj = instr.get_sj();
                    pc = (pc as isize + sj as isize) as usize;
                }
                OpCode::Return => {
                    // return R[A], ..., R[A+B-2]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    // Update PC before returning
                    save_pc!();

                    // Return hook (cold path — re-read hook_mask directly,
                    // no cross-dispatch `trap` variable needed)
                    if lua_state.hook_mask & crate::lua_vm::LUA_MASKRET != 0 && lua_state.allow_hook
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
                    if lua_state.hook_mask & crate::lua_vm::LUA_MASKRET != 0 && lua_state.allow_hook
                    {
                        hook_on_return(lua_state, frame_idx, pc as u32, 0)?;
                    }
                    return_handler::handle_return0(lua_state, frame_idx);

                    // Inline context restore (like Return1) to avoid full 'startfunc
                    // reload overhead. Critical for closures like counter.increment()
                    // that return no values.
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
                    base = ci.base;
                    let ci_pc = ci.pc as usize;
                    let caller_chunk = unsafe { &*ci.chunk_ptr };
                    code = &caller_chunk.code;
                    constants = &caller_chunk.constants;
                    pc = ci_pc;
                    if lua_state.hook_mask != 0 {
                        lua_state.oldpc = (ci_pc - 1) as u32;
                    }
                }
                OpCode::Return1 => {
                    // return R[A] — hottest return path
                    let a = instr.get_a() as usize;

                    // Return hook (cold path)
                    if lua_state.hook_mask & crate::lua_vm::LUA_MASKRET != 0 && lua_state.allow_hook
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
                    base = ci.base;
                    let ci_pc = ci.pc as usize;
                    let caller_chunk = unsafe { &*ci.chunk_ptr };
                    code = &caller_chunk.code;
                    constants = &caller_chunk.constants;
                    pc = ci_pc;
                    if lua_state.hook_mask != 0 {
                        lua_state.oldpc = (ci_pc - 1) as u32;
                    }
                }
                OpCode::GetUpval => {
                    // R[A] := UpValue[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    unsafe {
                        let upvalue_ptrs = current_upvalue_ptrs!();
                        let uv = &(*upvalue_ptrs.add(b)).as_ref().data;
                        let sp = lua_state.stack.as_mut_ptr();
                        *sp.add(base + a) = *uv.get_v_ptr();
                    }
                }
                OpCode::SetUpval => {
                    // UpValue[B] := R[A]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    unsafe {
                        let upvalue_ptrs = current_upvalue_ptrs!();
                        let sp = lua_state.stack.as_ptr();
                        let value = *sp.add(base + a);
                        let upval_ptr = *upvalue_ptrs.add(b);
                        upval_ptr.as_mut_ref().data.set_value(value);
                        // GC barrier (only for collectable values)
                        if value.is_collectable()
                            && let Some(gc_ptr) = value.as_gc_ptr()
                        {
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

                    if k && (pc) < current_chunk!().code.len() {
                        let extra_instr = unsafe { *code.get_unchecked(pc) };
                        if extra_instr.get_opcode() == OpCode::ExtraArg {
                            vc += extra_instr.get_ax() as usize * 1024;
                        }
                    }

                    pc += 1; // skip EXTRAARG

                    let value = lua_state.create_table(vc, hash_size)?;
                    unsafe { *lua_state.stack_mut().get_unchecked_mut(base + a) = value };

                    // Lua 5.5's OP_NEWTABLE: lower top to ra+1 then checkGC,
                    // so the GC only scans up to the table (excludes stale
                    // registers above). Then restore top to ci->top.
                    // Use set_top_raw: stack was already grown by push_lua_frame.
                    let new_top = base + a + 1;
                    save_pc!();
                    lua_state.set_top_raw(new_top);
                    lua_state.check_gc()?;
                    let frame_top = lua_state.get_call_info(frame_idx).top as usize;
                    lua_state.set_top_raw(frame_top);
                }
                OpCode::GetTable => {
                    // GETTABLE: R[A] := R[B][R[C]]
                    // Pointer-based access (like C Lua's vRB/vRC — no 16-byte copy)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let sp = lua_state.stack.as_mut_ptr();

                    unsafe {
                        let rb_ptr = sp.add(base + b);
                        let rc_ptr = sp.add(base + c) as *const LuaValue;

                        if (*rb_ptr).tt == LUA_VTABLE {
                            let table_gc = &*((*rb_ptr).value.ptr as *const GcTable);
                            let native = &table_gc.data.impl_table;

                            if pttisinteger(rc_ptr)
                                && native.fast_geti_into(pivalue(rc_ptr), sp.add(base + a))
                            {
                                continue;
                            }
                            // Non-integer key OR integer key missed array
                            let rc = *rc_ptr;
                            if let Some(val) = native.raw_get(&rc) {
                                *sp.add(base + a) = val;
                                continue;
                            }
                            // Miss: check metatable
                            let meta = table_gc.data.meta_ptr();
                            if meta.is_null() {
                                *sp.add(base + a) = LuaValue::nil();
                                continue;
                            }
                            let rb = *rb_ptr;
                            save_pc!();
                            match noinline::try_index_meta_generic(
                                lua_state, meta, rb, rc, frame_idx,
                            )? {
                                noinline::IndexResult::Found(val) => {
                                    *lua_state.stack_mut().get_unchecked_mut(base + a) = val;
                                    continue;
                                }
                                noinline::IndexResult::CallMm => continue 'startfunc,
                                noinline::IndexResult::FallThrough => {}
                            }
                        }
                    }

                    // Cold: metatable __index chain or non-table
                    save_pc!();
                    let mut pc_idx = pc;
                    table_ops::exec_gettable(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                    pc = pc_idx;
                    restore_state!();
                }
                OpCode::GetI => {
                    // GETI: R[A] := R[B][C] (integer key)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as i64;

                    let sp = lua_state.stack.as_mut_ptr();

                    unsafe {
                        let rb_ptr = sp.add(base + b);

                        if (*rb_ptr).tt == LUA_VTABLE {
                            let table_gc = &*((*rb_ptr).value.ptr as *const GcTable);
                            let native = &table_gc.data.impl_table;

                            if native.fast_geti_into(c, sp.add(base + a)) {
                                continue;
                            }
                            // Array miss: check hash part for sparse integer keys
                            if let Some(val) = native.fast_geti(c) {
                                *sp.add(base + a) = val;
                                continue;
                            }
                            let meta = table_gc.data.meta_ptr();
                            if meta.is_null() {
                                *sp.add(base + a) = LuaValue::nil();
                                continue;
                            }
                            let rb = *rb_ptr;
                            save_pc!();
                            match noinline::try_index_meta_int(lua_state, meta, rb, c, frame_idx)? {
                                noinline::IndexResult::Found(val) => {
                                    *lua_state.stack_mut().get_unchecked_mut(base + a) = val;
                                    continue;
                                }
                                noinline::IndexResult::CallMm => continue 'startfunc,
                                noinline::IndexResult::FallThrough => {}
                            }
                        }
                    }

                    // Slow path: metamethod lookup
                    save_pc!();
                    let mut pc_idx = pc;
                    table_ops::exec_geti(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                    pc = pc_idx;
                    restore_state!();
                }
                OpCode::GetField => {
                    // GETFIELD: R[A] := R[B][K[C]:string]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let key = unsafe { constants.get_unchecked(c) };
                    let sp = lua_state.stack.as_mut_ptr();

                    unsafe {
                        let rb_ptr = sp.add(base + b);

                        if (*rb_ptr).tt == LUA_VTABLE {
                            let table_gc = &*((*rb_ptr).value.ptr as *const GcTable);
                            let table_ref = &table_gc.data;
                            if let Some(val) = table_ref.impl_table.get_shortstr_fast(key) {
                                *sp.add(base + a) = val;
                                continue;
                            }
                            let meta = table_ref.meta_ptr();
                            if meta.is_null() {
                                *sp.add(base + a) = LuaValue::nil();
                                continue;
                            }
                            let rb = *rb_ptr;
                            save_pc!();
                            match noinline::try_index_meta_str(lua_state, meta, rb, key, frame_idx)?
                            {
                                noinline::IndexResult::Found(val) => {
                                    *lua_state.stack_mut().get_unchecked_mut(base + a) = val;
                                    continue;
                                }
                                noinline::IndexResult::CallMm => continue 'startfunc,
                                noinline::IndexResult::FallThrough => {}
                            }
                        }
                    }

                    // Cold: metatable __index chain, __index function, non-table
                    save_pc!();
                    let mut pc_idx = pc;
                    table_ops::exec_getfield(
                        lua_state,
                        instr,
                        constants,
                        base,
                        frame_idx,
                        &mut pc_idx,
                    )?;
                    pc = pc_idx;
                    restore_state!();
                }
                OpCode::SetTable => {
                    // SETTABLE: R[A][R[B]] := RK(C)
                    // Pointer-based: avoid 16B LuaValue copies on fast path
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let sp = lua_state.stack().as_ptr();
                    let ra_ptr = unsafe { sp.add(base + a) };
                    let rb_ptr = unsafe { sp.add(base + b) };
                    let val_ptr = if k {
                        unsafe { constants.as_ptr().add(c) }
                    } else {
                        unsafe { sp.add(base + c) }
                    };

                    if unsafe { (*ra_ptr).tt } == LUA_VTABLE {
                        let table_gc = unsafe { &mut *((*ra_ptr).value.ptr as *mut GcTable) };
                        let table_ref = &mut table_gc.data;
                        if !table_ref.has_metatable() {
                            // No metatable: try integer fast path first (t[i] = v)
                            if unsafe { pttisinteger(rb_ptr) } {
                                let ikey = unsafe { (*rb_ptr).value.i };
                                if unsafe { table_ref.impl_table.fast_seti_ptr(ikey, val_ptr) } {
                                    if unsafe { (*val_ptr).tt } & 0x40 != 0 {
                                        lua_state.gc_barrier_back(unsafe {
                                            (*ra_ptr).as_gc_ptr_table_unchecked()
                                        });
                                    }
                                    continue;
                                }
                                let val = unsafe { *val_ptr };
                                let delta = table_ref.impl_table.set_int_slow(ikey, val);
                                if delta != 0 {
                                    lua_state.gc_track_table_resize(
                                        unsafe { (*ra_ptr).as_table_ptr_unchecked() },
                                        delta,
                                    );
                                }
                                if val.is_collectable() {
                                    lua_state.gc_barrier_back(unsafe {
                                        (*ra_ptr).as_gc_ptr_table_unchecked()
                                    });
                                }
                                continue;
                            }
                            // Non-integer key: validate then raw_set
                            let rb = unsafe { *rb_ptr };
                            if rb.is_nil() {
                                return Err(cold::error_table_index_nil(lua_state));
                            }
                            if rb.ttisfloat() && rb.fltvalue().is_nan() {
                                return Err(cold::error_table_index_nan(lua_state));
                            }
                            let ra = unsafe { *ra_ptr };
                            let val = unsafe { *val_ptr };
                            lua_state.raw_set(&ra, rb, val);
                            continue;
                        }
                        // Has metatable: if integer key with existing non-nil value
                        // in array, __newindex is NOT consulted
                        if unsafe { pttisinteger(rb_ptr) } {
                            let val = unsafe { *val_ptr };
                            if table_ref
                                .impl_table
                                .fast_seti_existing(unsafe { (*rb_ptr).value.i }, val)
                            {
                                if val.is_collectable() {
                                    lua_state.gc_barrier_back(unsafe {
                                        (*ra_ptr).as_gc_ptr_table_unchecked()
                                    });
                                }
                                continue;
                            }
                        }
                        // Generic non-integer existing key check
                        let rb = unsafe { *rb_ptr };
                        if let Some(existing) = table_ref.impl_table.raw_get(&rb)
                            && !existing.is_nil()
                        {
                            let ra = unsafe { *ra_ptr };
                            let val = unsafe { *val_ptr };
                            lua_state.raw_set(&ra, rb, val);
                            continue;
                        }
                        // Noinline __newindex fast path
                        let meta = table_ref.meta_ptr();
                        if !meta.is_null() {
                            save_pc!();
                            let ra = unsafe { *ra_ptr };
                            let val = unsafe { *val_ptr };
                            if noinline::try_newindex_meta(lua_state, meta, ra, rb, val, frame_idx)?
                            {
                                continue 'startfunc;
                            }
                        }
                    }

                    // Cold: metatable __newindex chain or non-table
                    save_pc!();
                    let mut pc_idx = pc;
                    table_ops::exec_settable(
                        lua_state,
                        instr,
                        constants,
                        base,
                        frame_idx,
                        &mut pc_idx,
                    )?;
                    pc = pc_idx;
                }
                OpCode::SetI => {
                    // SETI: R[A][B] := RK(C) (integer key)
                    // Pointer-based: avoid 16B LuaValue copies on fast path
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let sp = lua_state.stack().as_ptr();
                    let ra_ptr = unsafe { sp.add(base + a) };
                    let val_ptr = if k {
                        unsafe { constants.as_ptr().add(c) }
                    } else {
                        unsafe { sp.add(base + c) }
                    };

                    if unsafe { (*ra_ptr).tt } == LUA_VTABLE {
                        let table_gc = unsafe { &mut *((*ra_ptr).value.ptr as *mut GcTable) };
                        let table_ref = &mut table_gc.data;
                        if !table_ref.has_metatable() {
                            if unsafe { table_ref.impl_table.fast_seti_ptr(b as i64, val_ptr) } {
                                if unsafe { (*val_ptr).tt } & 0x40 != 0 {
                                    lua_state.gc_barrier_back(unsafe {
                                        (*ra_ptr).as_gc_ptr_table_unchecked()
                                    });
                                }
                                continue;
                            }
                            // No metatable: use set_int_slow
                            let value = unsafe { *val_ptr };
                            let delta = table_ref.impl_table.set_int_slow(b as i64, value);
                            if delta != 0 {
                                lua_state.gc_track_table_resize(
                                    unsafe { (*ra_ptr).as_table_ptr_unchecked() },
                                    delta,
                                );
                            }
                            if value.is_collectable() {
                                lua_state.gc_barrier_back(unsafe {
                                    (*ra_ptr).as_gc_ptr_table_unchecked()
                                });
                            }
                            continue;
                        }
                        if unsafe {
                            table_ref
                                .impl_table
                                .fast_seti_existing_ptr(b as i64, val_ptr)
                        } {
                            if unsafe { (*val_ptr).tt } & 0x40 != 0 {
                                lua_state.gc_barrier_back(unsafe {
                                    (*ra_ptr).as_gc_ptr_table_unchecked()
                                });
                            }
                            continue;
                        }
                        // Noinline __newindex fast path: Lua function → non-recursive
                        let meta = table_ref.meta_ptr();
                        if !meta.is_null() {
                            save_pc!();
                            let ra = unsafe { *ra_ptr };
                            let value = unsafe { *val_ptr };
                            if noinline::try_newindex_meta(
                                lua_state,
                                meta,
                                ra,
                                LuaValue::integer(b as i64),
                                value,
                                frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                    }

                    // Slow path: metamethod or non-table
                    save_pc!();
                    let mut pc_idx = pc;
                    table_ops::exec_seti(
                        lua_state,
                        instr,
                        constants,
                        base,
                        frame_idx,
                        &mut pc_idx,
                    )?;
                    pc = pc_idx;
                    restore_state!();
                }
                OpCode::SetField => {
                    // SETFIELD: R[A][K[B]:string] := RK(C)
                    // HOT PATH: Uses fast_setfield() for zero-cost abstraction
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let ra = unsafe { *lua_state.stack().get_unchecked(base + a) };
                    let key = unsafe { constants.get_unchecked(b) };
                    let value = if k {
                        unsafe { *constants.as_ptr().add(c) }
                    } else {
                        unsafe { *lua_state.stack().get_unchecked(base + c) }
                    };

                    // Try fast path: fast_setfield only succeeds when the key
                    // already exists with a non-nil value. Per Lua semantics,
                    // __newindex is NEVER consulted when the key already exists
                    // in the table's own hash part. So this is safe regardless
                    // of whether the table has a metatable.
                    if ra.tt == LUA_VTABLE {
                        let table_gc = unsafe { &mut *(ra.value.ptr as *mut GcTable) };
                        let table_ref = &mut table_gc.data;
                        if table_ref.impl_table.fast_setfield(key, value) {
                            // Existing key updated — GC write barrier
                            if value.is_collectable() {
                                lua_state
                                    .gc_barrier_back(unsafe { ra.as_gc_ptr_table_unchecked() });
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
                                if value.is_collectable() {
                                    lua_state
                                        .gc_barrier_back(unsafe { ra.as_gc_ptr_table_unchecked() });
                                }
                                continue;
                            }
                            // Needs rehash — use raw_set directly
                            let (_, delta) = table_ref.impl_table.raw_set(key, value);
                            table_ref.invalidate_tm_cache();
                            if delta != 0 {
                                lua_state.gc_track_table_resize(
                                    unsafe { ra.as_table_ptr_unchecked() },
                                    delta,
                                );
                            }
                            if value.is_collectable() {
                                lua_state
                                    .gc_barrier_back(unsafe { ra.as_gc_ptr_table_unchecked() });
                            }
                            continue;
                        }
                        // Has metatable with new key: noinline __newindex fast path
                        let meta = table_ref.meta_ptr();
                        if !meta.is_null() {
                            save_pc!();
                            if noinline::try_newindex_meta(
                                lua_state, meta, ra, *key, value, frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                    }

                    // Slow path: metamethod, non-table, or has metatable with new key
                    save_pc!();
                    let mut pc_idx = pc;
                    table_ops::exec_setfield(
                        lua_state,
                        instr,
                        constants,
                        base,
                        frame_idx,
                        &mut pc_idx,
                    )?;
                    pc = pc_idx;
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
                    if rb.tt == LUA_VTABLE {
                        let table_gc = unsafe { &*(rb.value.ptr as *const GcTable) };
                        let table_ref = &table_gc.data;
                        if let Some(val) = table_ref.impl_table.get_shortstr_fast(key) {
                            unsafe {
                                *lua_state.stack_mut().get_unchecked_mut(base + a) = val;
                            }
                            continue;
                        }
                        let meta = table_ref.meta_ptr();
                        if meta.is_null() {
                            unsafe {
                                *lua_state.stack_mut().get_unchecked_mut(base + a) =
                                    LuaValue::nil();
                            }
                            continue;
                        }
                        save_pc!();
                        match noinline::try_index_meta_str(lua_state, meta, rb, key, frame_idx)? {
                            noinline::IndexResult::Found(val) => {
                                unsafe {
                                    *lua_state.stack_mut().get_unchecked_mut(base + a) = val;
                                }
                                continue;
                            }
                            noinline::IndexResult::CallMm => continue 'startfunc,
                            noinline::IndexResult::FallThrough => {}
                        }
                    }

                    // Cold: metatable __index chain, __index function, non-table
                    save_pc!();
                    let mut pc_idx = pc;
                    table_ops::exec_self(
                        lua_state,
                        instr,
                        constants,
                        base,
                        frame_idx,
                        &mut pc_idx,
                    )?;
                    pc = pc_idx;
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
                        let new_chunk_ptr = new_chunk as *const crate::lua_value::Chunk;

                        let new_base = func_idx + 1;
                        save_pc!();
                        lua_state.push_lua_frame(
                            &func,
                            new_base,
                            nargs,
                            nresults,
                            new_chunk.param_count,
                            new_chunk.max_stack_size,
                            new_chunk_ptr,
                        )?;

                        // Inline callee entry: use known values directly instead of
                        // reading back from CallInfo (avoids redundant call_stack reload
                        // + frame_idx*72 address computation + 3 loads from memory we
                        // just wrote to — saves ~7 instructions on the hot path).
                        frame_idx = lua_state.call_depth() - 1;
                        base = new_base;
                        // Use raw pointer to derive code/constants (func is local,
                        // can't borrow new_chunk across the loop boundary).
                        let callee_chunk = unsafe { &*new_chunk_ptr };
                        code = &callee_chunk.code;
                        constants = &callee_chunk.constants;
                        pc = 0;

                        // Call hook for inline Lua call (cold path)
                        if lua_state.hook_mask & crate::lua_vm::LUA_MASKCALL != 0
                            && lua_state.allow_hook
                        {
                            hook_on_call(lua_state, lua_state.hook_mask, 0, callee_chunk)?;
                        }
                        if lua_state.hook_mask != 0 {
                            lua_state.oldpc = if callee_chunk.is_vararg { 0 } else { u32::MAX };
                        }
                        updatetrap!();
                        continue;
                    }

                    // Semi-cold path: __call metamethod on table
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
                            lua_state.oldpc = (pc - 1) as u32;
                            updatetrap!();
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
                    let func_idx = base + a;

                    save_pc!();

                    // Hot path: Lua-to-Lua tail call
                    let func = unsafe { *lua_state.stack().get_unchecked(func_idx) };
                    if func.is_lua_function() {
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

                        // Close upvalues only when k bit is set (like C Lua's TESTARG_k)
                        if instr.get_k() {
                            lua_state.close_upvalues(base);
                        }

                        let lua_func = unsafe { func.as_lua_function_unchecked() };
                        let new_chunk_ptr = call::pretailcall_lua(
                            lua_state, func, lua_func, func_idx, base, nargs, frame_idx,
                        )?;

                        // Set local vars directly from known values
                        let ci = lua_state.get_call_info(frame_idx);
                        base = ci.base;
                        let new_chunk = unsafe { &*new_chunk_ptr };
                        code = &new_chunk.code;
                        constants = &new_chunk.constants;
                        pc = 0;

                        lua_state.oldpc = if new_chunk.is_vararg { 0 } else { u32::MAX };

                        // Call hook at function entry (cold path)
                        if lua_state.hook_mask != 0 {
                            let hook_mask = lua_state.hook_mask;
                            if hook_mask & crate::lua_vm::LUA_MASKCALL != 0 && lua_state.allow_hook
                            {
                                hook_on_call(lua_state, hook_mask, CIST_TAIL, new_chunk)?;
                            }
                            if hook_mask & crate::lua_vm::LUA_MASKCOUNT != 0 {
                                lua_state.hook_count = lua_state.base_hook_count;
                            }
                        }
                        continue;
                    }

                    // Cold path: C function, __call metamethod, etc.
                    match call::handle_tailcall(lua_state, base, a, b) {
                        Ok(FrameAction::Continue) => {
                            restore_state!();
                            lua_state.oldpc = (pc - 1) as u32;
                            updatetrap!();
                        }
                        Ok(FrameAction::TailCall) => {
                            continue 'startfunc;
                        }
                        Ok(FrameAction::Call) => {
                            continue 'startfunc;
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::Not => {
                    // R[A] := not R[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let rb = unsafe { lua_state.stack().get_unchecked(base + b) };

                    // l_isfalse: nil or false
                    let is_false = rb.tt() == LUA_VFALSE || rb.is_nil();
                    if is_false {
                        setbtvalue(unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) });
                    } else {
                        setbfvalue(unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) });
                    }
                }
                OpCode::ForLoop => {
                    // Numeric for loop
                    // If integer: check counter, decrement, add step, jump back
                    // If float: add step, check limit, jump back
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    unsafe {
                        // Compute stack-relative pointer for ForLoop fast path.
                        // SAFETY: ForLoop fast paths never grow the stack.
                        let sp = lua_state.stack_mut().as_mut_ptr();
                        let ra_ptr = sp.add(base + a);

                        // Check if integer loop (tag of step at ra+1)
                        if pttisinteger(ra_ptr.add(1) as *const LuaValue) {
                            // Integer loop (most common for numeric loops)
                            // ra: counter (count of iterations left)
                            // ra+1: step
                            // ra+2: control variable (idx)
                            let count = pivalue(ra_ptr as *const LuaValue) as u64;
                            if count > 0 {
                                // More iterations
                                let step = pivalue(ra_ptr.add(1) as *const LuaValue);
                                let idx = pivalue(ra_ptr.add(2) as *const LuaValue);

                                // Update counter (decrement) - only write value, tag unchanged
                                (*ra_ptr).value.i = (count - 1) as i64;

                                // Update control variable: idx += step - only write value
                                (*ra_ptr.add(2)).value.i = idx.wrapping_add(step);

                                // Jump back (no error check - validated at compile time)
                                pc -= bx;
                            }
                            // else: counter expired, exit loop
                        } else {
                            // Float loop
                            // ra: limit
                            // ra+1: step
                            // ra+2: idx (control variable)
                            let step = pfltvalue(ra_ptr.add(1) as *const LuaValue);
                            let limit = pfltvalue(ra_ptr as *const LuaValue);
                            let idx = pfltvalue(ra_ptr.add(2) as *const LuaValue);

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
                                (*ra_ptr.add(2)).value.n = new_idx;

                                // Jump back (bytecode compiler guarantees valid targets)
                                pc -= bx;
                            }
                            // else: exit loop
                        }
                    }
                }
                OpCode::ForPrep => {
                    // Cold path — only runs once per loop, extracted to reduce
                    // main loop code size and prevent r12/r15 clobbering
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;
                    let mut pc_idx = pc;
                    cold::handle_forprep_int(lua_state, base + a, bx, frame_idx, &mut pc_idx)?;
                    pc = pc_idx;
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
                    } else if iterator.is_lua_function() {
                        // FAST PATH: Lua closure iterator (closure-based for loops)
                        // Inline call setup — avoids handle_call overhead, FrameAction
                        // enum, set_top_raw, and 'startfunc full context reload.
                        let lua_func = unsafe { iterator.as_lua_function_unchecked() };
                        let new_chunk = lua_func.chunk();
                        let new_chunk_ptr = new_chunk as *const crate::lua_value::Chunk;
                        let new_base = ra_base + 4; // func at ra+3, args start at ra+4
                        let nresults = c as i32 + 1;

                        lua_state.push_lua_frame(
                            &iterator,
                            new_base,
                            2, // always 2 args (state, control)
                            nresults,
                            new_chunk.param_count,
                            new_chunk.max_stack_size,
                            new_chunk_ptr,
                        )?;

                        // Inline callee entry (same as Call opcode fast path)
                        frame_idx = lua_state.call_depth() - 1;
                        base = new_base;
                        let callee_chunk = unsafe { &*new_chunk_ptr };
                        code = &callee_chunk.code;
                        constants = &callee_chunk.constants;
                        pc = 0;

                        if lua_state.hook_mask & crate::lua_vm::LUA_MASKCALL != 0
                            && lua_state.allow_hook
                        {
                            hook_on_call(lua_state, lua_state.hook_mask, 0, callee_chunk)?;
                        }
                        if lua_state.hook_mask != 0 {
                            lua_state.oldpc = if callee_chunk.is_vararg { 0 } else { u32::MAX };
                        }
                        updatetrap!();
                        continue;
                    } else {
                        // Cold path: __call metamethod
                        match call::handle_call(lua_state, base, a + 3, 3, c + 1, 0) {
                            Ok(FrameAction::Continue) => {
                                restore_state!();
                                lua_state.oldpc = (pc - 1) as u32;
                                updatetrap!();
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

                    let base_mm = lua_state.get_frame_base(frame_idx);
                    let v1 = unsafe { *lua_state.stack().get_unchecked(base_mm + a) };
                    if v1.ttistable() {
                        let v2 = unsafe { *lua_state.stack().get_unchecked(base_mm + b) };
                        let result_reg = unsafe { code.get_unchecked(pc - 2) }.get_a() as usize;
                        save_pc!();
                        match noinline::try_mmbin_table_fast(
                            lua_state, v1, v1, v2, c as u8, result_reg, frame_idx,
                        )? {
                            noinline::MmBinResult::CallMm => continue 'startfunc,
                            noinline::MmBinResult::Handled => {
                                restore_state!();
                                continue;
                            }
                            noinline::MmBinResult::FallThrough => {}
                        }
                    }

                    // Slow path: string coercion, userdata, v2 metamethods
                    save_pc!();
                    metamethod::handle_mmbin(lua_state, a, b, c, pc, code, frame_idx)?;
                    restore_state!();
                }
                OpCode::MmBinI => {
                    // Call metamethod over R[A] and immediate sB
                    let a = instr.get_a() as usize;
                    let sb = instr.get_sb();
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let base_mm = lua_state.get_frame_base(frame_idx);
                    let v1 = unsafe { *lua_state.stack().get_unchecked(base_mm + a) };
                    if v1.ttistable() {
                        let imm = LuaValue::integer(sb as i64);
                        let (p1, p2) = if k { (imm, v1) } else { (v1, imm) };
                        let result_reg = unsafe { code.get_unchecked(pc - 2) }.get_a() as usize;
                        save_pc!();
                        match noinline::try_mmbin_table_fast(
                            lua_state, v1, p1, p2, c as u8, result_reg, frame_idx,
                        )? {
                            noinline::MmBinResult::CallMm => continue 'startfunc,
                            noinline::MmBinResult::Handled => {
                                restore_state!();
                                continue;
                            }
                            noinline::MmBinResult::FallThrough => {}
                        }
                    }

                    // Slow path
                    save_pc!();
                    metamethod::handle_mmbini(lua_state, a, sb, c, k, pc, code, frame_idx)?;
                    restore_state!();
                }
                OpCode::MmBinK => {
                    // Call metamethod over R[A] and K[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let base_mm = lua_state.get_frame_base(frame_idx);
                    let v1 = unsafe { *lua_state.stack().get_unchecked(base_mm + a) };
                    if v1.ttistable() {
                        let kb = unsafe { *constants.as_ptr().add(b) };
                        let (p1, p2) = if k { (kb, v1) } else { (v1, kb) };
                        let result_reg = unsafe { code.get_unchecked(pc - 2) }.get_a() as usize;
                        save_pc!();
                        match noinline::try_mmbin_table_fast(
                            lua_state, v1, p1, p2, c as u8, result_reg, frame_idx,
                        )? {
                            noinline::MmBinResult::CallMm => continue 'startfunc,
                            noinline::MmBinResult::Handled => {
                                restore_state!();
                                continue;
                            }
                            noinline::MmBinResult::FallThrough => {}
                        }
                    }

                    // Slow path
                    save_pc!();
                    metamethod::handle_mmbink(
                        lua_state, a, b, c, k, pc, code, constants, frame_idx,
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

                    let upval_ptr = unsafe { *current_upvalue_ptrs!().add(b) };
                    let upval = &upval_ptr.as_ref().data;
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
                        unsafe { *lua_state.stack_mut().get_unchecked_mut(base + a) = val };
                    } else {
                        // Noinline __index metatable check
                        let table_value = *upval.get_value_ref();
                        if table_value.tt == LUA_VTABLE {
                            let table_gc = unsafe { &*(table_value.value.ptr as *const GcTable) };
                            let meta = table_gc.data.meta_ptr();
                            if !meta.is_null() {
                                save_pc!();
                                match noinline::try_index_meta_str(
                                    lua_state,
                                    meta,
                                    table_value,
                                    key,
                                    frame_idx,
                                )? {
                                    noinline::IndexResult::Found(val) => {
                                        unsafe {
                                            *lua_state.stack_mut().get_unchecked_mut(base + a) = val
                                        };
                                        continue;
                                    }
                                    noinline::IndexResult::CallMm => continue 'startfunc,
                                    noinline::IndexResult::FallThrough => {}
                                }
                            }
                        }
                        // Slow path: metamethod lookup (non-Lua __index, table chain, non-table)
                        let table_value = *unsafe { *current_upvalue_ptrs!().add(b) }
                            .as_ref()
                            .data
                            .get_value_ref();
                        let write_pos = base + a;
                        let call_info = lua_state.get_call_info_mut(frame_idx);
                        if write_pos + 1 > call_info.top as usize {
                            call_info.top = (write_pos + 1) as u32;
                            lua_state.set_top(write_pos + 1)?;
                        }
                        save_pc!();
                        match helper::lookup_from_metatable(lua_state, &table_value, key) {
                            Ok(result) => {
                                restore_state!();
                                unsafe {
                                    *lua_state.stack_mut().get_unchecked_mut(base + a) =
                                        result.unwrap_or(LuaValue::nil())
                                };
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
                }

                OpCode::SetTabUp => {
                    // UpValue[A][K[B]:shortstring] := RK(C)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let key = unsafe { *constants.as_ptr().add(b) };
                    let value = if k {
                        unsafe { *constants.as_ptr().add(c) }
                    } else {
                        unsafe { *lua_state.stack().get_unchecked(base + c) }
                    };

                    // Fast path: direct set for existing short string key
                    // Use raw pointer to avoid borrow conflicts with lua_state
                    let (table_val_copy, table_raw_ptr) = unsafe {
                        let upval = &(*current_upvalue_ptrs!().add(a)).as_ref().data;
                        let tv = upval.get_value_ref();
                        (*tv, tv as *const LuaValue)
                    };
                    if table_val_copy.tt == LUA_VTABLE {
                        let table = unsafe { &mut *(table_val_copy.value.ptr as *mut GcTable) };
                        let native = &mut table.data.impl_table;
                        if native.has_hash() && native.set_shortstr_unchecked(&key, value) {
                            if value.is_collectable()
                                && let Some(gc_ptr) = table_val_copy.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                        // Noinline __newindex fast path: Lua function → non-recursive
                        let meta = table.data.meta_ptr();
                        if !meta.is_null() {
                            save_pc!();
                            if noinline::try_newindex_meta(
                                lua_state,
                                meta,
                                table_val_copy,
                                key,
                                value,
                                frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                    }

                    // Slow path: handle metamethods (__newindex)
                    let table_value = unsafe { *table_raw_ptr };
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
                    let rb = unsafe { *lua_state.stack().get_unchecked(base + b) };
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
                        // Has metatable — use noinline helper
                        save_pc!();
                        match noinline::try_len_meta(lua_state, meta, rb, frame_idx)? {
                            noinline::LenResult::RawLen => {
                                let tbl = unsafe { &*(rb.value.ptr as *const GcTable) };
                                setivalue(
                                    unsafe { lua_state.stack_mut().get_unchecked_mut(base + a) },
                                    tbl.data.len() as i64,
                                );
                                continue;
                            }
                            noinline::LenResult::CallMm => continue 'startfunc,
                            noinline::LenResult::FallThrough => {}
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

                OpCode::Eq => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();
                    let ra = unsafe { *lua_state.stack().get_unchecked(base + a) };
                    let rb = unsafe { *lua_state.stack().get_unchecked(base + b) };
                    // Fast path: same identity → equal
                    // EQ: if ((R[A] == R[B]) ~= k) then pc++
                    if ra == rb {
                        // cond=true: skip when k=false (true ~= false → true)
                        if !k {
                            pc += 1;
                        }
                    } else if ra.tt() != rb.tt() {
                        // Different types → never equal (cond=false)
                        // skip when k=true (false ~= true → true)
                        if k {
                            pc += 1;
                        }
                    } else if ra.ttistable() || ra.ttisfulluserdata() {
                        // Inline metatable lookup for __eq from first operand (table fast path)
                        if ra.tt == LUA_VTABLE {
                            save_pc!();
                            if noinline::try_comp_meta_table(
                                lua_state,
                                ra,
                                ra,
                                rb,
                                TmKind::Eq,
                                frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        // Fall through: try rb's metatable or recursive path
                        save_pc!();
                        if cold::try_push_eq_mm_frame(lua_state, ra, rb, frame_idx)? {
                            continue 'startfunc;
                        }
                        let mut pc_idx = pc;
                        comparison_ops::exec_eq(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                        pc = pc_idx;
                    } else {
                        let mut pc_idx = pc;
                        comparison_ops::exec_eq(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                        pc = pc_idx;
                    }
                }

                OpCode::Lt => {
                    // LT fast path: inline integer/float comparison
                    // Avoids exec_lt function call overhead for the common case
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();

                    let ra = unsafe { *lua_state.stack().get_unchecked(base + a) };
                    let rb = unsafe { *lua_state.stack().get_unchecked(base + b) };

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
                    } else if ra.tt == LUA_VTABLE {
                        // Inline metatable lookup for __lt from first operand
                        save_pc!();
                        if noinline::try_comp_meta_table(
                            lua_state,
                            ra,
                            ra,
                            rb,
                            TmKind::Lt,
                            frame_idx,
                        )? {
                            continue 'startfunc;
                        }
                        // Fall through: try rb's metatable or other types
                        let mut pc_idx = pc;
                        comparison_ops::exec_lt(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                        pc = pc_idx;
                    } else {
                        // Try non-recursive metamethod fast path for rb table
                        if rb.tt == LUA_VTABLE {
                            save_pc!();
                            if cold::try_push_comp_mm_frame(
                                lua_state,
                                ra,
                                rb,
                                TmKind::Lt,
                                frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = pc;
                        comparison_ops::exec_lt(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                        pc = pc_idx;
                    }
                }

                OpCode::Le => {
                    // LE fast path: inline integer/float comparison
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();

                    let ra = unsafe { *lua_state.stack().get_unchecked(base + a) };
                    let rb = unsafe { *lua_state.stack().get_unchecked(base + b) };

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
                    } else if ra.tt == LUA_VTABLE {
                        // Inline metatable lookup for __le from first operand
                        save_pc!();
                        if noinline::try_comp_meta_table(
                            lua_state,
                            ra,
                            ra,
                            rb,
                            TmKind::Le,
                            frame_idx,
                        )? {
                            continue 'startfunc;
                        }
                        // Fall through: try rb's metatable or other types
                        let mut pc_idx = pc;
                        comparison_ops::exec_le(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                        pc = pc_idx;
                    } else {
                        // Try non-recursive metamethod fast path for rb table
                        if rb.tt == LUA_VTABLE {
                            save_pc!();
                            if cold::try_push_comp_mm_frame(
                                lua_state,
                                ra,
                                rb,
                                TmKind::Le,
                                frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = pc;
                        comparison_ops::exec_le(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                        pc = pc_idx;
                    }
                }
                OpCode::EqK => {
                    let mut pc_idx = pc;
                    comparison_ops::exec_eqk(lua_state, instr, constants, base, &mut pc_idx)?;
                    pc = pc_idx;
                }

                OpCode::EqI => {
                    let mut pc_idx = pc;
                    comparison_ops::exec_eqi(lua_state, instr, base, &mut pc_idx)?;
                    pc = pc_idx;
                }

                OpCode::LtI => {
                    // LTI fast path: if (R[A] < sB) ~= k then pc++
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();

                    let ra = unsafe { lua_state.stack().get_unchecked(base + a) };
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
                        // ra is not number, try inline metatable lookup for tables
                        let ra_val = *ra;
                        if ra_val.tt == LUA_VTABLE {
                            let isf = instr.get_c() != 0;
                            let imm_val = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            save_pc!();
                            if noinline::try_comp_meta_table(
                                lua_state,
                                ra_val,
                                ra_val,
                                imm_val,
                                TmKind::Lt,
                                frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = pc;
                        comparison_ops::exec_lti(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                        pc = pc_idx;
                    }
                }

                OpCode::LeI => {
                    // LEI fast path: if (R[A] <= sB) ~= k then pc++
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();

                    let ra = unsafe { lua_state.stack().get_unchecked(base + a) };
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
                        // Inline metatable lookup for __le
                        let ra_val = *ra;
                        if ra_val.tt == LUA_VTABLE {
                            let isf = instr.get_c() != 0;
                            let imm_val = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            save_pc!();
                            if noinline::try_comp_meta_table(
                                lua_state,
                                ra_val,
                                ra_val,
                                imm_val,
                                TmKind::Le,
                                frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = pc;
                        comparison_ops::exec_lei(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                        pc = pc_idx;
                    }
                }

                OpCode::GtI => {
                    // GTI fast path
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();

                    let ra = unsafe { lua_state.stack().get_unchecked(base + a) };
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
                        // GTI: R[A] > im is equivalent to im < R[A]
                        let ra_val = *ra;
                        if ra_val.tt == LUA_VTABLE {
                            let isf = instr.get_c() != 0;
                            let imm_val = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            save_pc!();
                            // swap: __lt(imm, ra)
                            if noinline::try_comp_meta_table(
                                lua_state,
                                ra_val,
                                imm_val,
                                ra_val,
                                TmKind::Lt,
                                frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = pc;
                        comparison_ops::exec_gti(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                        pc = pc_idx;
                    }
                }

                OpCode::GeI => {
                    // GEI fast path
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();

                    let ra = unsafe { lua_state.stack().get_unchecked(base + a) };
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
                        // GEI: R[A] >= im is equivalent to im <= R[A]
                        let ra_val = *ra;
                        if ra_val.tt == LUA_VTABLE {
                            let isf = instr.get_c() != 0;
                            let imm_val = if isf {
                                LuaValue::float(im as f64)
                            } else {
                                LuaValue::integer(im as i64)
                            };
                            save_pc!();
                            // swap: __le(imm, ra)
                            if noinline::try_comp_meta_table(
                                lua_state,
                                ra_val,
                                imm_val,
                                ra_val,
                                TmKind::Le,
                                frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = pc;
                        comparison_ops::exec_gei(lua_state, instr, base, frame_idx, &mut pc_idx)?;
                        pc = pc_idx;
                    }
                }

                OpCode::Test => {
                    // docondjump(): if (cond != k) then pc++ else donextjump
                    let a = instr.get_a() as usize;
                    let k = instr.get_k();

                    let ra = unsafe { lua_state.stack().get_unchecked(base + a) };

                    // l_isfalse: nil or false
                    let is_false = ra.is_nil() || ra.tt() == LUA_VFALSE;
                    let cond = !is_false;

                    if cond != k {
                        pc += 1; // Skip next instruction (JMP)
                    } else {
                        // Execute next instruction (must be JMP)
                        let next_instr = unsafe { *code.get_unchecked(pc) };
                        pc += 1;
                        let sj = next_instr.get_sj();
                        pc = (pc as isize + sj as isize) as usize;
                    }
                }

                OpCode::TestSet => {
                    // if (l_isfalse(R[B]) == k) then pc++ else R[A] := R[B]; donextjump
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();

                    let rb = unsafe { lua_state.stack().get_unchecked(base + b) };
                    let is_false = rb.is_nil() || (rb.is_boolean() && rb.tt() == LUA_VFALSE);

                    if is_false == k {
                        pc += 1; // Condition failed - skip next instruction (JMP)
                    } else {
                        // Condition succeeded - copy value and EXECUTE next instruction (must be JMP)
                        unsafe { *lua_state.stack_mut().get_unchecked_mut(base + a) = *rb };
                        // donextjump: fetch and execute next JMP instruction
                        let next_instr = unsafe { *code.get_unchecked(pc) };
                        debug_assert!(next_instr.get_opcode() == OpCode::Jmp);
                        pc += 1; // Move past the JMP instruction
                        let sj = next_instr.get_sj();
                        pc = (pc as isize + sj as isize) as usize; // Execute the jump
                    }
                }
                OpCode::SetList => {
                    let mut pc_idx = pc;
                    closure_vararg_ops::exec_setlist(lua_state, instr, code, base, &mut pc_idx)?;
                    pc = pc_idx;
                }
                OpCode::Closure => {
                    let chunk_ref = current_chunk!();
                    handle_closure(lua_state, instr, base, frame_idx, chunk_ref, pc)?;
                }

                OpCode::Vararg => {
                    let chunk_ref = current_chunk!();
                    closure_vararg_ops::exec_vararg(lua_state, instr, base, frame_idx, chunk_ref)?;
                }

                OpCode::GetVarg => {
                    handle_getvarg(lua_state, instr, base, frame_idx)?;
                }

                OpCode::ErrNNil => {
                    handle_errnil(lua_state, instr, base, constants, frame_idx, pc)?;
                }

                OpCode::VarargPrep => {
                    let chunk_ref = current_chunk!();
                    closure_vararg_ops::exec_varargprep(
                        lua_state, frame_idx, chunk_ref, &mut base,
                    )?;
                    // real instruction always fires a line event.
                    // Our absolute line_info makes changedline(0, 1) return
                    // false when VARARGPREP and the next instruction share a
                    // line. The sentinel forces unconditional fire.
                    if lua_state.hook_mask != 0 {
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
