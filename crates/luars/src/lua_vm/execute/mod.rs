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
mod concat;
mod helper;
mod metamethod;
mod return_handler;

// Extracted opcode modules to reduce main loop size
mod closure_vararg_ops;
mod comparison_ops;
mod table_ops;

use call::FrameAction;

use crate::{
    lua_value::{LUA_VFALSE, LuaValue},
    lua_vm::{
        LuaError, LuaResult, LuaState, OpCode,
        execute::helper::{
            fltvalue, ivalue, setbfvalue, setbtvalue, setfltvalue, setivalue, setnilvalue,
            tointeger, tointegerns, tonumber, tonumberns, ttisfloat, ttisinteger, ttisstring,
        },
    },
};
pub use helper::{get_metamethod_event, get_metatable};
pub use metamethod::TmKind;

/// Main VM execution entry point
///
/// Executes bytecode starting from current PC in the active call frame
/// Returns when all frames are popped (depth reaches 0)
///
/// Architecture: Lua-style single loop, NOT recursive calls
/// - CALL (Lua): push frame, reload chunk/upvalues, continue loop
/// - CALL (C): execute directly, return immediately (no frame push)
/// - RETURN: pop frame, reload chunk/upvalues, continue loop
/// - TAILCALL: replace frame, reload chunk/upvalues, continue loop
pub fn lua_execute(lua_state: &mut LuaState) -> LuaResult<()> {
    lua_execute_until(lua_state, 0)
}

/// Execute until call depth reaches target_depth
/// Used for protected calls (pcall) to execute only the called function
/// without affecting caller frames
///
/// ARCHITECTURE: Single-loop execution like Lua C's luaV_execute
/// - Uses labeled loops instead of goto for context switching
/// - Function calls/returns just update pointers and continue
/// - Zero Rust function call overhead
pub fn lua_execute_until(lua_state: &mut LuaState, target_depth: usize) -> LuaResult<()> {
    // STARTFUNC: Function context switching point (like Lua C's startfunc label)
    'startfunc: loop {
        // Check if we've reached target depth
        let current_depth = lua_state.call_depth();
        if current_depth <= target_depth {
            return Ok(());
        }

        let frame_idx = current_depth - 1;

        // ===== LOAD FRAME CONTEXT =====
        let func_value = lua_state
            .get_frame_func(frame_idx)
            .ok_or(LuaError::RuntimeError)?;

        let Some(func_body) = func_value.as_lua_function() else {
            return Err(lua_state.error("Current frame is not a function".to_string()));
        };

        let Some(chunk) = func_body.chunk() else {
            return Err(lua_state.error("Lua function has no chunk".to_string()));
        };

        let upvalue_ptrs = func_body.upvalues();

        // Load frame state
        let mut pc = lua_state.get_frame_pc(frame_idx) as usize;
        let mut base = lua_state.get_frame_base(frame_idx);

        // Pre-grow stack
        let needed_size = base + chunk.max_stack_size;
        lua_state.grow_stack(needed_size)?;

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
                if frame_idx < lua_state.call_depth() {
                    base = lua_state.get_frame_base(frame_idx);
                } else {
                    panic!(
                        "restore_state: frame_idx {} >= call_depth {}",
                        frame_idx,
                        lua_state.call_depth()
                    );
                }
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
                    // R[A] := R[B] - OPTIMIZED with raw pointers
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let stack = lua_state.stack_mut();
                    stack[base + a] = stack[base + b];
                }
                OpCode::LoadI => {
                    // R[A] := sBx - OPTIMIZED with raw pointers
                    let a = instr.get_a() as usize;
                    let sbx = instr.get_sbx();
                    let stack = lua_state.stack_mut();
                    stack[base + a] = LuaValue::integer(sbx as i64);
                }
                OpCode::LoadF => {
                    // R[A] := (float)sBx
                    let a = instr.get_a() as usize;
                    let sbx = instr.get_sbx();
                    let stack = lua_state.stack_mut();
                    stack[base + a] = LuaValue::float(sbx as f64);
                }
                OpCode::LoadK => {
                    // R[A] := K[Bx] - OPTIMIZED with raw pointers
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;
                    let stack = lua_state.stack_mut();
                    stack[base + a] = constants[bx];
                }
                OpCode::LoadKX => {
                    // R[A] := K[extra_arg]; pc++
                    let a = instr.get_a() as usize;

                    if pc >= code.len() {
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(lua_state.error("LOADKX: missing EXTRAARG".to_string()));
                    }

                    let extra = code[pc];
                    pc += 1; // Consume EXTRAARG

                    if extra.get_opcode() != OpCode::ExtraArg {
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(lua_state.error("LOADKX: expected EXTRAARG".to_string()));
                    }

                    let ax = extra.get_ax() as usize;
                    if ax >= constants.len() {
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(
                            lua_state.error(format!("LOADKX: invalid constant index {}", ax))
                        );
                    }

                    let value = constants[ax];
                    let stack = lua_state.stack_mut();
                    stack[base + a] = value; // setobj2s
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

                    // OPTIMIZATION: Use raw slice access to reduce bounds checking
                    let stack = lua_state.stack_mut();
                    unsafe {
                        let v1 = stack.get_unchecked(base + b);
                        let v2 = stack.get_unchecked(base + c);

                        // Fast path: both integers (most common case)
                        if ttisinteger(v1) && ttisinteger(v2) {
                            let i1 = ivalue(v1);
                            let i2 = ivalue(v2);
                            let ra = stack.get_unchecked_mut(base + a);
                            setivalue(ra, i1.wrapping_add(i2));
                            pc += 1; // Skip metamethod on success
                        }
                        // Slow path: try float conversion
                        else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                                let ra = stack.get_unchecked_mut(base + a);
                                setfltvalue(ra, n1 + n2);
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

                    // OPTIMIZATION: Use get_unchecked to avoid bounds checking
                    let stack = lua_state.stack_mut();
                    unsafe {
                        let v1 = stack.get_unchecked(base + b);

                        // Fast path: integer (most common)
                        if ttisinteger(v1) {
                            let iv1 = ivalue(v1);
                            let ra = stack.get_unchecked_mut(base + a);
                            setivalue(ra, iv1.wrapping_add(sc as i64));
                            pc += 1; // Skip metamethod on success
                        }
                        // Slow path: float
                        else if ttisfloat(v1) {
                            let nb = fltvalue(v1);
                            let ra = stack.get_unchecked_mut(base + a);
                            setfltvalue(ra, nb + (sc as f64));
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
                        let v1 = stack.get_unchecked(base + b);
                        let v2 = stack.get_unchecked(base + c);

                        if ttisinteger(v1) && ttisinteger(v2) {
                            let i1 = ivalue(v1);
                            let i2 = ivalue(v2);
                            let ra = stack.get_unchecked_mut(base + a);
                            setivalue(ra, i1.wrapping_sub(i2));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                                let ra = stack.get_unchecked_mut(base + a);
                                setfltvalue(ra, n1 - n2);
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
                        let v1 = stack.get_unchecked(base + b);
                        let v2 = stack.get_unchecked(base + c);

                        if ttisinteger(v1) && ttisinteger(v2) {
                            let i1 = ivalue(v1);
                            let i2 = ivalue(v2);
                            let ra = stack.get_unchecked_mut(base + a);
                            setivalue(ra, i1.wrapping_mul(i2));
                            pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                                let ra = stack.get_unchecked_mut(base + a);
                                setfltvalue(ra, n1 * n2);
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
                            setivalue(&mut stack[base + a], i1.div_euclid(i2));
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
                            setivalue(&mut stack[base + a], i1.rem_euclid(i2));
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                                pc += 1;
                                setfltvalue(&mut stack[base + a], n1 - (n1 / n2).floor() * n2);
                            }
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
                            metamethod::try_unary_tm(
                                lua_state,
                                rb,
                                base + a,
                                metamethod::TmKind::Unm,
                            )?;
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

                    let mut i2 = 0i64;
                    if ttisinteger(v1) && tointeger(v2, &mut i2) {
                        let i1 = ivalue(v1);
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

                    let mut i2 = 0i64;
                    if ttisinteger(v1) && tointeger(v2, &mut i2) {
                        let i1 = ivalue(v1);
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

                    let mut i2 = 0i64;
                    if ttisinteger(v1) && tointeger(v2, &mut i2) {
                        let i1 = ivalue(v1);
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

                    let mut i2 = 0i64;
                    if ttisinteger(v1) && tointeger(v2, &mut i2) {
                        let i1 = ivalue(v1);
                        if i2 != 0 {
                            pc += 1;
                            let result = i1 - (i1 / i2) * i2;
                            setivalue(&mut stack[base + a], result);
                        }
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            if n2 != 0.0 {
                                pc += 1;
                                setfltvalue(&mut stack[base + a], n1 - (n1 / n2).floor() * n2);
                            }
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

                    let mut i2 = 0i64;
                    if ttisinteger(v1) && tointeger(v2, &mut i2) {
                        let i1 = ivalue(v1);
                        if i2 != 0 {
                            pc += 1;
                            let result = if (i1 ^ i2) >= 0 {
                                i1 / i2
                            } else {
                                (i1 / i2) - if i1 % i2 != 0 { 1 } else { 0 }
                            };
                            setivalue(&mut stack[base + a], result);
                        }
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            if n2 != 0.0 {
                                pc += 1;
                                setfltvalue(&mut stack[base + a], (n1 / n2).floor());
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
                        // Lua wraps shift amount to 0-63
                        let shift = (i2 & 63) as u32;
                        setivalue(&mut stack[base + a], i1.wrapping_shl(shift));
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
                        // Lua wraps shift amount to 0-63
                        let shift = (i2 & 63) as u32;
                        setivalue(&mut stack[base + a], (i1 as u64).wrapping_shr(shift) as i64);
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
                        metamethod::try_unary_tm(
                            lua_state,
                            v1,
                            base + a,
                            metamethod::TmKind::Bnot,
                        )?;
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
                        let result = if ib >= 0 {
                            if ib >= 64 { 0 } else { (ic as i64) << ib }
                        } else {
                            // negative shift = right shift
                            let shift = -ib;
                            if shift >= 64 {
                                if ic < 0 { -1 } else { 0 }
                            } else {
                                (ic as i64) >> shift
                            }
                        };
                        setivalue(&mut stack[base + a], result);
                    }
                    // else: metamethod
                }
                OpCode::ShrI => {
                    // R[A] := R[B] >> sC
                    // Arithmetic right shift
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc(); // shift amount

                    let stack = lua_state.stack_mut();
                    let rb = &stack[base + b];

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        pc += 1;
                        // luaV_shiftl(ib, -ic): shift ib left by -ic (i.e., right by ic)
                        let shift_amount = -ic;
                        let result = if shift_amount >= 0 {
                            if shift_amount >= 64 {
                                0
                            } else {
                                ib << shift_amount
                            }
                        } else {
                            // right shift
                            let shift = -shift_amount;
                            if shift >= 64 {
                                if ib < 0 { -1 } else { 0 }
                            } else {
                                ib >> shift
                            }
                        };
                        setivalue(&mut stack[base + a], result);
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

                    lua_state.check_gc()?;
                    // Return pops frame, continue to 'startfunc to load caller's context
                    continue 'startfunc;
                }
                OpCode::Return0 => {
                    // return (no values)
                    save_pc!();
                    return_handler::handle_return0(lua_state, frame_idx)?;

                    lua_state.check_gc()?;
                    // Return pops frame, continue to 'startfunc
                    continue 'startfunc;
                }
                OpCode::Return1 => {
                    // return R[A]
                    let a = instr.get_a() as usize;
                    save_pc!();
                    return_handler::handle_return1(lua_state, base, frame_idx, a)?;

                    lua_state.check_gc()?;
                    // Return pops frame, continue to 'startfunc
                    continue 'startfunc;
                }
                OpCode::GetUpval => {
                    // R[A] := UpValue[B]
                    // ZERO-COST: Direct pointer dereference like Lua C's cl->upvals[B]->v.p
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    if b >= upvalue_ptrs.len() {
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(
                            lua_state.error(format!("GETUPVAL: invalid upvalue index {}", b))
                        );
                    }

                    // ULTRA-OPTIMIZED: Direct double-pointer dereference
                    // Matches Lua C: setobj2s(L, ra, cl->upvals[b]->v.p)
                    let value = upvalue_ptrs[b].as_ref().data.get_value();

                    let stack = lua_state.stack_mut();
                    stack[base + a] = value;
                }
                OpCode::SetUpval => {
                    // UpValue[B] := R[A]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let value = lua_state.stack()[base + a];
                    // Get value to set
                    // Set value in upvalue (OPTIMIZED: Uses direct pointer)
                    // ULTRA-OPTIMIZED: Direct double-pointer write
                    // Matches Lua C: setobj(L, uv->v.p, s2v(ra))
                    let upval_ptr = upvalue_ptrs[b];
                    if let Some(stack_index) = upval_ptr.as_mut_ref().data.get_stack_index() {
                        lua_state.stack_mut()[stack_index] = value;
                    } else {
                        // Closed upvalue: points to own storage
                        upval_ptr.as_mut_ref().data.close(value);
                    }

                    // GC barrier: luaC_barrier(L, uv, s2v(ra))
                    // If upvalue is black and value is white collectable, restore invariant
                    if value.is_collectable() {
                        if let Some(gc_ptr) = value.as_gc_ptr() {
                            lua_state.gc_barrier(upval_ptr, gc_ptr);
                        }
                    }
                }
                OpCode::Close => {
                    // Close all upvalues >= R[A]
                    let a = instr.get_a() as usize;
                    let close_from = base + a;

                    // Close upvalues at or above this level
                    lua_state.close_upvalues(close_from);
                }
                OpCode::Tbc => {
                    // Mark variable as to-be-closed
                    let _a = instr.get_a() as usize;

                    // TODO: Implement to-be-closed variables
                    // This is for the <close> attribute in Lua 5.4+
                    // Needs tracking in stack/upvalue system
                    // For now, this is a no-op
                }
                OpCode::NewTable => {
                    // R[A] := {} (new table)
                    // This instruction uses ivABC format:
                    // vB (6 bits) = log2(hash size) + 1
                    // vC (10 bits) = array size
                    let a = instr.get_a() as usize;
                    let vb = instr.get_vb() as usize; // Use get_vb() for ivABC format
                    let mut vc = instr.get_vc() as usize; // Use get_vc() for ivABC format
                    let k = instr.get_k();

                    // Calculate hash size: if vB > 0, hash_size = 2^(vB-1)
                    // vB is 6 bits, so max value is 63
                    let hash_size = if vb > 0 {
                        if vb > 31 {
                            // Safety check to prevent overflow on 32-bit systems
                            0
                        } else {
                            1usize << (vb - 1)
                        }
                    } else {
                        0
                    };

                    // Check for EXTRAARG instruction for larger array sizes
                    // If k is set, the EXTRAARG contains additional array size
                    if k {
                        if pc < code.len() {
                            let extra_instr = code[pc];
                            if extra_instr.get_opcode() == OpCode::ExtraArg {
                                let extra = extra_instr.get_ax() as usize;
                                // Add extra to array size: vc += extra * (MAXARG_vC + 1)
                                // MAXARG_vC is 10 bits = 1023, so + 1 = 1024
                                vc += extra * 1024;
                            }
                        }
                    }

                    // ALWAYS skip the next instruction (EXTRAARG), as per Lua 5.4+ spec
                    pc += 1;

                    // Create table with pre-allocated sizes
                    let value = lua_state.create_table(vc, hash_size)?;

                    let stack = lua_state.stack_mut();
                    stack[base + a] = value;

                    // like checkGC
                    let new_top = base + a + 1;
                    save_pc!();
                    lua_state.set_top(new_top)?;
                    lua_state.check_gc()?;
                }
                OpCode::GetTable => {
                    table_ops::exec_gettable(lua_state, instr, base, frame_idx, &mut pc)?;
                }
                OpCode::GetI => {
                    table_ops::exec_geti(lua_state, instr, base, frame_idx, &mut pc)?;
                }
                OpCode::GetField => {
                    table_ops::exec_getfield(
                        lua_state, instr, constants, base, frame_idx, &mut pc,
                    )?;
                }
                OpCode::SetTable => {
                    table_ops::exec_settable(
                        lua_state, instr, constants, base, frame_idx, &mut pc,
                    )?;
                }
                OpCode::SetI => {
                    table_ops::exec_seti(lua_state, instr, constants, base, frame_idx, &mut pc)?;
                }
                OpCode::SetField => {
                    table_ops::exec_setfield(
                        lua_state, instr, constants, base, frame_idx, &mut pc,
                    )?;
                }
                OpCode::Self_ => {
                    table_ops::exec_self(lua_state, instr, constants, base, frame_idx, &mut pc)?;
                }
                OpCode::Call => {
                    // R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    // Save PC before call
                    save_pc!();

                    // Delegate to call handler
                    match call::handle_call(lua_state, base, a, b, c) {
                        Ok(FrameAction::Continue) => {
                            // C function executed, continue in current frame
                            restore_state!();
                        }
                        Ok(FrameAction::Call) => {
                            // Lua function pushed new frame
                            // Continue to 'startfunc to load new function context
                            continue 'startfunc;
                        }
                        Ok(FrameAction::TailCall) => {
                            // Tail call replaced current frame
                            continue 'startfunc;
                        }
                        Ok(FrameAction::Return) => {
                            // Shouldn't happen from handle_call
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
                        Ok(FrameAction::Call) | Ok(FrameAction::Return) => {
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

                                // Update counter (decrement)
                                setivalue(stack.get_unchecked_mut(ra), (count - 1) as i64);

                                // Update control variable: idx += step
                                setivalue(stack.get_unchecked_mut(ra + 2), idx.wrapping_add(step));

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
                                // Update control variable
                                setfltvalue(stack.get_unchecked_mut(ra + 2), new_idx);

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
                    // Prepare numeric for loop
                    // Input: ra=init, ra+1=limit, ra+2=step
                    // Output (integer): ra=counter, ra+1=step, ra+2=idx
                    // Output (float): ra=limit, ra+1=step, ra+2=idx
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    let ra = base + a;
                    let pinit_idx = ra;
                    let plimit_idx = ra + 1;
                    let pstep_idx = ra + 2;

                    // Check if integer loop
                    if ttisinteger(&stack[pinit_idx]) && ttisinteger(&stack[pstep_idx]) {
                        // Integer loop
                        let init = ivalue(&stack[pinit_idx]);
                        let step = ivalue(&stack[pstep_idx]);

                        if step == 0 {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("'for' step is zero".to_string()));
                        }

                        // Get limit (may need conversion)
                        let limit = if ttisinteger(&stack[plimit_idx]) {
                            ivalue(&stack[plimit_idx])
                        } else if ttisfloat(&stack[plimit_idx]) {
                            let flimit = fltvalue(&stack[plimit_idx]);
                            if step < 0 {
                                flimit.ceil() as i64
                            } else {
                                flimit.floor() as i64
                            }
                        } else {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("'for' limit must be a number".to_string()));
                        };

                        // Check if loop should run
                        let should_skip = if step > 0 { init > limit } else { init < limit };

                        if should_skip {
                            // Skip loop
                            pc += bx + 1;
                        } else {
                            // Prepare loop counter
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

                            // Setup: ra=counter, ra+1=step, ra+2=idx
                            setivalue(&mut stack[ra], count as i64);
                            setivalue(&mut stack[ra + 1], step);
                            setivalue(&mut stack[ra + 2], init);
                        }
                    } else {
                        // Float loop
                        let mut init = 0.0;
                        let mut limit = 0.0;
                        let mut step = 0.0;

                        if !tonumberns(&stack[plimit_idx], &mut limit) {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("'for' limit must be a number".to_string()));
                        }
                        if !tonumberns(&stack[pstep_idx], &mut step) {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("'for' step must be a number".to_string()));
                        }
                        if !tonumberns(&stack[pinit_idx], &mut init) {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(
                                lua_state.error("'for' initial value must be a number".to_string())
                            );
                        }

                        if step == 0.0 {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("'for' step is zero".to_string()));
                        }

                        // Check if loop should run
                        let should_skip = if step > 0.0 {
                            limit < init
                        } else {
                            init < limit
                        };

                        if should_skip {
                            // Skip loop
                            pc += bx + 1;
                        } else {
                            // Setup: ra=limit, ra+1=step, ra+2=idx
                            setfltvalue(&mut stack[ra], limit);
                            setfltvalue(&mut stack[ra + 1], step);
                            setfltvalue(&mut stack[ra + 2], init);
                        }
                    }
                }
                OpCode::TForPrep => {
                    // Prepare generic for loop
                    // Before: ra=iterator, ra+1=state, ra+2=control, ra+3=closing
                    // After: ra=iterator, ra+1=state, ra+2=closing(tbc), ra+3=control
                    // Then jump to loop end (where TFORCALL is)
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    let ra = base + a;

                    // Swap control and closing variables
                    let temp = stack[ra + 3]; // closing
                    stack[ra + 3] = stack[ra + 2]; // control -> closing position
                    stack[ra + 2] = temp; // closing -> control position

                    // TODO: Mark ra+2 as to-be-closed if not nil
                    // For now, skip TBC handling

                    // Jump to loop end (+ Bx)
                    pc += bx;
                }
                OpCode::TForCall => {
                    // Generic for loop call
                    // Call: ra+3,ra+4,...,ra+2+C := ra(ra+1, ra+2)
                    // ra=iterator, ra+1=state, ra+2=closing, ra+3=control
                    let a = instr.get_a() as usize;
                    let c = instr.get_c() as usize;

                    // Get values before modifying stack
                    let ra_base = base + a;
                    let (iterator, state, control) = {
                        let stack = lua_state.stack_mut();
                        (stack[ra_base], stack[ra_base + 1], stack[ra_base + 3])
                    };

                    // Setup call stack using safe API:
                    // ra+3: function (copy from ra)
                    // ra+4: arg1 (copy from ra+1, state)
                    // ra+5: arg2 (copy from ra+3, control variable)
                    lua_state.stack_set(ra_base + 3, iterator)?;
                    lua_state.stack_set(ra_base + 4, state)?;
                    lua_state.stack_set(ra_base + 5, control)?;

                    // Save PC before call
                    lua_state.set_frame_pc(frame_idx, pc as u32);

                    // Call iterator function at base+a+3
                    // Arguments: 2 (state and control)
                    // Results: c (number of loop variables)
                    match call::handle_call(lua_state, base, a + 3, 3, c + 1) {
                        Ok(FrameAction::Continue) => {
                            // C function completed, results already in place
                            // Fall through to next instruction (TFORLOOP)
                            restore_state!();
                        }
                        Ok(FrameAction::Call) => {
                            // Lua function pushed new frame
                            continue 'startfunc;
                        }
                        Ok(FrameAction::TailCall) | Ok(FrameAction::Return) => {
                            continue 'startfunc;
                        }
                        Err(e) => return Err(e),
                    }
                }
                OpCode::TForLoop => {
                    // Generic for loop test
                    // If ra+3 != nil then ra+2 = ra+3 and jump back
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    let ra = base + a;

                    // Check if ra+3 (new control value) is not nil
                    if !stack[ra + 3].is_nil() {
                        // Continue loop: update control variable and jump back
                        stack[ra + 2] = stack[ra + 3];

                        if bx > pc {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("TFORLOOP: invalid jump".to_string()));
                        }
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

                    let table_value = upvalue_ptrs[b].as_ref().data.get_value();
                    let key = &constants[c];

                    // Fast path: direct table access without metamethod
                    let result = if let Some(table) = table_value.as_table() {
                        if let Some(val) = table.raw_get(key) {
                            val
                        } else {
                            // Key not found, need metamethod - update frame state
                            let write_pos = base + a;
                            let call_info = lua_state.get_call_info_mut(frame_idx);
                            if write_pos + 1 > call_info.top {
                                call_info.top = write_pos + 1;
                                lua_state.set_top(write_pos + 1)?;
                            }
                            save_pc!();
                            let result =
                                helper::lookup_from_metatable(lua_state, &table_value, key);
                            restore_state!();
                            result.unwrap_or(LuaValue::nil())
                        }
                    } else {
                        // Not a table, need metamethod
                        let write_pos = base + a;
                        let call_info = lua_state.get_call_info_mut(frame_idx);
                        if write_pos + 1 > call_info.top {
                            call_info.top = write_pos + 1;
                            lua_state.set_top(write_pos + 1)?;
                        }
                        save_pc!();
                        let result = helper::lookup_from_metatable(lua_state, &table_value, key);
                        restore_state!();
                        result.unwrap_or(LuaValue::nil())
                    };

                    let stack = lua_state.stack_mut();
                    stack[base + a] = result;
                }

                OpCode::SetTabUp => {
                    // UpValue[A][K[B]:shortstring] := RK(C)
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let table_value = upvalue_ptrs[a].as_ref().data.get_value();
                    let key = constants[b];

                    // Get value (RK: register or constant)
                    let value = if k {
                        constants[c]
                    } else {
                        lua_state.stack_mut()[base + c]
                    };

                    // Set table[key] = value
                    if table_value.is_table() {
                        lua_state.raw_set(&table_value, key, value);
                    }
                }

                // ============================================================
                // LENGTH AND CONCATENATION
                // ============================================================
                OpCode::Len => {
                    // R[A] := #R[B]
                    // Port of luaV_objlen from lvm.c:731-757
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    let rb = lua_state.stack_mut()[base + b];

                    // Try to get length based on type
                    if let Some(s) = rb.as_str() {
                        // String: get length directly
                        let len = s.len();
                        setivalue(&mut lua_state.stack_mut()[base + a], len as i64);
                    } else if let Some(table) = rb.as_table() {
                        // Table: check for __len metamethod first
                        let has_metatable = table.get_metatable().is_some();

                        if has_metatable {
                            // Try __len metamethod
                            if let Some(mm) =
                                helper::get_metamethod_event(lua_state, &rb, TmKind::Len)
                            {
                                // Call metamethod with Protect pattern
                                save_pc!();
                                let result = metamethod::call_tm_res(lua_state, mm, rb, rb)?;
                                restore_state!();
                                lua_state.stack_mut()[base + a] = result;
                            } else {
                                // No metamethod, use primitive length
                                let len = table.len();
                                setivalue(&mut lua_state.stack_mut()[base + a], len as i64);
                            }
                        } else {
                            // No metatable, use primitive length
                            let len = table.len();
                            setivalue(&mut lua_state.stack_mut()[base + a], len as i64);
                        }
                    } else {
                        // Other types: try __len metamethod
                        if let Some(mm) = helper::get_metamethod_event(lua_state, &rb, TmKind::Len)
                        {
                            save_pc!();
                            let result = metamethod::call_tm_res(lua_state, mm, rb, rb)?;
                            restore_state!();
                            lua_state.stack_mut()[base + a] = result;
                        } else {
                            // No metamethod: type error
                            return Err(lua_state.error(format!(
                                "attempt to get length of a {} value",
                                rb.type_name()
                            )));
                        }
                    }
                }

                OpCode::Concat => {
                    // R[A] := R[A].. ... ..R[A + B - 1]
                    // Port of OP_CONCAT from lvm.c:1626
                    let a = instr.get_a() as usize;
                    let n = instr.get_b() as usize;

                    // Try string concatenation first
                    // If that fails (non-string without metamethod), try metamethod
                    match concat::concat_strings(lua_state, base, a, n) {
                        Ok(result) => {
                            let stack = lua_state.stack_mut();
                            stack[base + a] = result;
                        }
                        Err(_) => {
                            // Not strings - try __concat metamethod on last two values
                            // Following Lua's luaT_tryconcatTM pattern
                            if n >= 2 {
                                let (v1, v2) = {
                                    let stack = lua_state.stack_mut();
                                    (stack[base + a + n - 2], stack[base + a + n - 1])
                                };

                                // Try to get __concat metamethod
                                if let Some(mm) = helper::get_binop_metamethod(
                                    lua_state,
                                    &v1,
                                    &v2,
                                    TmKind::Concat,
                                ) {
                                    save_pc!();
                                    let result = metamethod::call_tm_res(lua_state, mm, v1, v2)?;
                                    restore_state!();

                                    // Store result back and reduce count
                                    let stack = lua_state.stack_mut();
                                    stack[base + a + n - 2] = result;

                                    // If we still have more than 1 value left, continue concatenating
                                    // For simplicity, just store the result in R[A]
                                    if n == 2 {
                                        stack[base + a] = result;
                                    } else {
                                        // Multiple concat with metamethod - simplified approach
                                        // Store the result of last two, will need to handle rest
                                        return Err(lua_state.error(
                                            "complex concat with metamethod not fully supported"
                                                .to_string(),
                                        ));
                                    }
                                } else {
                                    return Err(lua_state.error(format!(
                                        "attempt to concatenate {} and {} values",
                                        v1.type_name(),
                                        v2.type_name()
                                    )));
                                }
                            } else {
                                return Err(lua_state
                                    .error("concat requires at least 2 values".to_string()));
                            }
                        }
                    }

                    let new_top = base + a + 1;
                    // GC check after concatenation (like Lua 5.5 OP_CONCAT)
                    save_pc!();
                    lua_state.set_top(new_top)?;
                    lua_state.check_gc()?;
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
                    // if ((R[A] <= R[B]) ~= k) then pc++; else donextjump
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();

                    let cond = {
                        let stack = lua_state.stack_mut();
                        let ra = &stack[base + a];
                        let rb = &stack[base + b];

                        if ttisinteger(ra) && ttisinteger(rb) {
                            ivalue(ra) <= ivalue(rb)
                        } else if (ttisinteger(ra) || ttisfloat(ra))
                            && (ttisinteger(rb) || ttisfloat(rb))
                        {
                            let mut na = 0.0;
                            let mut nb = 0.0;
                            tonumberns(ra, &mut na);
                            tonumberns(rb, &mut nb);
                            na <= nb
                        } else if ttisstring(ra) && ttisstring(rb) {
                            // String comparison - copy IDs first
                            // OPTIMIZATION: Use as_string_str() for direct access
                            if let (Some(sa), Some(sb)) = (ra.as_str(), rb.as_str()) {
                                sa <= sb
                            } else {
                                false
                            }
                        } else {
                            // Try metamethod - use Protect pattern
                            let va = *ra;
                            let vb = *rb;

                            save_pc!();
                            let result =
                                match metamethod::try_comp_tm(lua_state, va, vb, TmKind::Le)? {
                                    Some(result) => result,
                                    None => {
                                        return Err(lua_state.error(
                                            "attempt to compare non-comparable values".to_string(),
                                        ));
                                    }
                                };
                            restore_state!();
                            result
                        }
                    };

                    if cond != k {
                        pc += 1; // Condition failed - skip next instruction
                    }
                    // else: Condition succeeded - execute next instruction (must be JMP)
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
                    comparison_ops::exec_lti(lua_state, instr, base, &mut pc)?;
                }

                OpCode::LeI => {
                    comparison_ops::exec_lei(lua_state, instr, base, &mut pc)?;
                }

                OpCode::GtI => {
                    comparison_ops::exec_gti(lua_state, instr, base, &mut pc)?;
                }

                OpCode::GeI => {
                    comparison_ops::exec_gei(lua_state, instr, base, &mut pc)?;
                }

                // ============================================================
                // CONDITIONAL TESTS
                // ============================================================
                OpCode::Test => {
                    // docondjump(): if (cond != k) then pc++ else donextjump
                    let a = instr.get_a() as usize;
                    let k = instr.get_k();

                    let stack = lua_state.stack_mut();
                    let ra = &stack[base + a];

                    // l_isfalse: nil or false
                    let is_false = ra.is_nil() || (ra.is_boolean() && ra.tt() == LUA_VFALSE);
                    let cond = !is_false;

                    if cond != k {
                        pc += 1; // Skip next instruction (JMP)
                    } else {
                        // Execute next instruction (must be JMP)
                        let next_instr = unsafe { *chunk.code.get_unchecked(pc) };
                        debug_assert!(next_instr.get_opcode() == OpCode::Jmp);
                        pc += 1; // Move past the JMP
                        let sj = next_instr.get_sj();
                        pc = (pc as i32 + sj) as usize; // Execute the jump
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
                    closure_vararg_ops::exec_closure(
                        lua_state,
                        instr,
                        base,
                        &chunk,
                        &upvalue_ptrs,
                    )?;

                    let a = instr.get_a() as usize;
                    let new_top = base + a + 1;
                    save_pc!();
                    lua_state.set_top(new_top)?;
                    lua_state.check_gc()?;
                }

                OpCode::Vararg => {
                    closure_vararg_ops::exec_vararg(lua_state, instr, base, frame_idx, &chunk)?;
                }

                OpCode::GetVarg => {
                    // R[A] := varargs[R[C]]
                    // Based on lvm.c:1943 and ltm.c:292 luaT_getvararg
                    // This is for accessing individual vararg elements or the count
                    let a = instr.get_a() as usize;
                    let _b = instr.get_b() as usize; // unused in Lua 5.5
                    let c = instr.get_c() as usize;

                    // Get nextraargs from CallInfo
                    let call_info = lua_state.get_call_info(frame_idx);
                    let nextra = call_info.nextraargs as usize;
                    let func_obj = call_info
                        .func
                        .as_lua_function()
                        .ok_or(LuaError::RuntimeError)?;
                    // Get param_count from the function's chunk
                    let param_count = {
                        let chunk = func_obj.chunk().ok_or(LuaError::RuntimeError)?;
                        chunk.param_count
                    };

                    let stack = lua_state.stack_mut();
                    let ra_idx = base + a;
                    let rc = stack[base + c];

                    // Check if R[C] is string "n" (get vararg count)
                    if let Some(s) = rc.as_str() {
                        if s == "n" {
                            // Return vararg count
                            let stack = lua_state.stack_mut();
                            setivalue(&mut stack[ra_idx], nextra as i64);
                            pc += 1;
                            continue;
                        }
                    }

                    // Check if R[C] is an integer (vararg index, 1-based)
                    if ttisinteger(&rc) {
                        let index = ivalue(&rc);

                        // Check if index is valid (1 <= index <= nextraargs)
                        let stack = lua_state.stack_mut();
                        if nextra > 0 && index >= 1 && (index as usize) <= nextra {
                            // Get value from varargs
                            // varargs are stored after fixed parameters at base + param_count
                            let vararg_start = base + param_count;
                            let src_val = stack[vararg_start + (index as usize) - 1];
                            stack[ra_idx] = src_val;
                        } else {
                            // Out of bounds or no varargs: return nil
                            setnilvalue(&mut stack[ra_idx]);
                        }
                    } else {
                        // Not integer or "n": return nil
                        let stack = lua_state.stack_mut();
                        setnilvalue(&mut stack[ra_idx]);
                    }
                }

                OpCode::ErrNNil => {
                    // Raise error if R[A] is not nil (global already defined)
                    // Based on lvm.c:1949 and ldebug.c:817 luaG_errnnil
                    // This is used by the compiler to detect duplicate global definitions
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    let ra = &stack[base + a];

                    // If value is not nil, it means the global is already defined
                    if !ra.is_nil() {
                        // Get global name from constants if bx > 0
                        let global_name = if bx > 0 && bx - 1 < constants.len() {
                            if let Some(s) = constants[bx - 1].as_str() {
                                s.to_string()
                            } else {
                                "?".to_string()
                            }
                        } else {
                            "?".to_string()
                        };

                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(
                            lua_state.error(format!("global '{}' already defined", global_name))
                        );
                    }
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
