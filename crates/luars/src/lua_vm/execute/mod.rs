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
mod metamethod;
mod return_handler;

use call::FrameAction;

use std::rc::Rc;

use crate::{
    Chunk, UpvalueId,
    lua_value::{LUA_VFALSE, LUA_VNUMFLT, LUA_VNUMINT, LuaValue},
    lua_vm::{LuaError, LuaResult, LuaState, OpCode},
};
pub use metamethod::TmKind;

// ============ Type tag检查宏 (对应 Lua 的 ttis* 宏) ============

/// ttisinteger - 检查是否是整数 (最快的类型检查)
#[inline(always)]
unsafe fn ttisinteger(v: *const LuaValue) -> bool {
    unsafe { (*v).tt_ == LUA_VNUMINT }
}

/// ttisfloat - 检查是否是浮点数
#[inline(always)]
unsafe fn ttisfloat(v: *const LuaValue) -> bool {
    unsafe { (*v).tt_ == LUA_VNUMFLT }
}

/// ttisnumber - 检查是否是任意数字 (整数或浮点)
#[inline(always)]
unsafe fn ttisnumber(v: *const LuaValue) -> bool {
    unsafe { (*v).tt_ == LUA_VNUMINT || (*v).tt_ == LUA_VNUMFLT }
}

// ============ 值访问宏 (对应 Lua 的 ivalue/fltvalue) ============

/// ivalue - 直接获取整数值 (调用前必须用 ttisinteger 检查)
#[inline(always)]
unsafe fn ivalue(v: *const LuaValue) -> i64 {
    unsafe { (*v).value_.i }
}

/// fltvalue - 直接获取浮点值 (调用前必须用 ttisfloat 检查)
#[inline(always)]
unsafe fn fltvalue(v: *const LuaValue) -> f64 {
    unsafe { (*v).value_.n }
}

/// setivalue - 设置整数值
#[inline(always)]
unsafe fn setivalue(v: *mut LuaValue, i: i64) {
    unsafe {
        (*v).value_.i = i;
        (*v).tt_ = LUA_VNUMINT;
    }
}

/// chgivalue - 只修改整数值，不修改类型标签（Lua的chgivalue宏）
/// 调用前必须确认类型已经是整数！
#[inline(always)]
unsafe fn chgivalue(v: *mut LuaValue, i: i64) {
    unsafe {
        (*v).value_.i = i;
    }
}

/// setfltvalue - 设置浮点值
#[inline(always)]
unsafe fn setfltvalue(v: *mut LuaValue, n: f64) {
    unsafe {
        (*v).value_.n = n;
        (*v).tt_ = LUA_VNUMFLT;
    }
}

/// chgfltvalue - 只修改浮点值，不修改类型标签
/// 调用前必须确认类型已经是浮点！
#[inline(always)]
unsafe fn chgfltvalue(v: *mut LuaValue, n: f64) {
    unsafe {
        (*v).value_.n = n;
    }
}

/// setbfvalue - 设置false
#[inline(always)]
unsafe fn setbfvalue(v: *mut LuaValue) {
    unsafe {
        (*v) = LuaValue::boolean(false);
    }
}

/// setbtvalue - 设置true
#[inline(always)]
unsafe fn setbtvalue(v: *mut LuaValue) {
    unsafe {
        (*v) = LuaValue::boolean(true);
    }
}

/// setnilvalue - 设置nil
#[inline(always)]
unsafe fn setnilvalue(v: *mut LuaValue) {
    unsafe {
        *v = LuaValue::nil();
    }
}

// ============ 类型转换辅助函数 ============

/// tointegerns - 尝试转换为整数 (不抛出错误)
/// 对应 Lua 的 tointegerns 宏
#[inline(always)]
unsafe fn tointegerns(v: *const LuaValue, out: &mut i64) -> bool {
    unsafe {
        if ttisinteger(v) {
            *out = ivalue(v);
            true
        } else {
            false
        }
    }
}

/// tonumberns - 尝试转换为浮点数 (不抛出错误)
#[inline(always)]
unsafe fn tonumberns(v: *const LuaValue, out: &mut f64) -> bool {
    unsafe {
        if ttisfloat(v) {
            *out = fltvalue(v);
            true
        } else if ttisinteger(v) {
            *out = ivalue(v) as f64;
            true
        } else {
            false
        }
    }
}

/// tonumber - 从LuaValue引用转换为浮点数 (用于常量)
#[inline(always)]
fn tonumber(v: &LuaValue, out: &mut f64) -> bool {
    if v.tt_ == LUA_VNUMFLT {
        unsafe {
            *out = v.value_.n;
        }
        true
    } else if v.tt_ == LUA_VNUMINT {
        unsafe {
            *out = v.value_.i as f64;
        }
        true
    } else {
        false
    }
}

/// tointeger - 从LuaValue引用获取整数 (用于常量)
#[inline(always)]
fn tointeger(v: &LuaValue, out: &mut i64) -> bool {
    if v.tt_ == LUA_VNUMINT {
        unsafe {
            *out = v.value_.i;
        }
        true
    } else {
        false
    }
}

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
#[allow(unused)]
pub fn lua_execute(lua_state: &mut LuaState) -> LuaResult<()> {
    lua_execute_until(lua_state, 0)
}

/// Execute until call depth reaches target_depth
/// Used for protected calls (pcall) to execute only the called function
/// without affecting caller frames
pub fn lua_execute_until(lua_state: &mut LuaState, target_depth: usize) -> LuaResult<()> {
    // Main execution loop - continues until frames are popped to target depth
    'vm_loop: loop {
        // Check if we've reached target depth
        let current_depth = lua_state.call_depth();
        if current_depth <= target_depth {
            return Ok(()); // Reached target depth, stop execution
        }

        // Get current call frame index
        let frame_idx = current_depth - 1;

        // Load current function's chunk and upvalues
        let (chunk, upvalues_vec) = {
            let func_value = lua_state
                .get_frame_func(frame_idx)
                .ok_or(LuaError::RuntimeError)?;

            let Some(func_id) = func_value.as_function_id() else {
                return Err(lua_state.error("Current frame is not a function".to_string()));
            };

            let gc_function = lua_state
                .vm_mut()
                .object_pool
                .get_function_mut(func_id)
                .ok_or(LuaError::RuntimeError)?;

            // Check if this is a Lua function
            // C functions should not reach here because they execute synchronously
            // in handle_call and return FrameAction::Return immediately
            if !gc_function.is_lua_function() {
                return Err(lua_state.error("Unexpected C function in main VM loop".to_string()));
            }

            let Some(chunk_rc) = gc_function.chunk() else {
                return Err(lua_state.error("Lua function has no chunk".to_string()));
            };

            (chunk_rc.clone(), gc_function.upvalues.clone())
        };

        // Execute this frame until CALL/RETURN/TAILCALL
        match execute_frame(lua_state, frame_idx, chunk, upvalues_vec)? {
            FrameAction::Return => {
                // Frame was already popped by handle_return, just continue with caller
                // Loop continues with caller's frame (or exits if no caller)
            }
            FrameAction::Call => {
                // New frame was pushed, loop continues with callee's frame
                // Note: C functions don't push frames, they execute and return Continue
                continue 'vm_loop;
            }
            FrameAction::TailCall => {
                // Current frame was replaced, loop continues with new function
                continue 'vm_loop;
            }
            FrameAction::Continue => {
                // C function executed in current frame, continue with same frame
                // Just loop back to execute_frame with the same frame
                continue 'vm_loop;
            }
        }
    }
}

/// Execute a single frame until it calls another function or returns
fn execute_frame(
    lua_state: &mut LuaState,
    frame_idx: usize,
    chunk: Rc<Chunk>,
    upvalues_vec: Vec<UpvalueId>,
) -> LuaResult<FrameAction> {
    // Cache values in locals (will be in CPU registers)
    let mut pc = lua_state.get_frame_pc(frame_idx) as usize;
    let base = lua_state.get_frame_base(frame_idx);

    // PRE-GROW STACK: Ensure enough space for this frame
    // This prevents reallocation during normal instruction execution
    let needed_size = base + chunk.max_stack_size;
    lua_state.grow_stack(needed_size)?;

    // SAFETY: Get raw pointer after grow_stack
    // This pointer may become invalid after operations that:
    // - Create tables/strings/closures (may trigger GC)
    // - Call functions (may grow stack)
    // - Concatenate strings (may allocate)
    // After such operations, we must refresh stack_ptr
    let mut stack_ptr = lua_state.stack_ptr_mut();

    // Constants and code pointers (avoid repeated dereferencing)
    let constants = &chunk.constants;
    let code = &chunk.code;

    // Main interpreter loop
    loop {
        // Fetch instruction and advance PC
        // NOTE: No bounds check - compiler guarantees valid bytecode
        let instr = unsafe { *code.get_unchecked(pc) };
        pc += 1;

        // Dispatch instruction
        // The match compiles to a jump table in release mode
        match instr.get_opcode() {
            OpCode::Move => {
                // R[A] := R[B]
                // setobjs2s(L, ra, RB(i))
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let rb = stack_ptr.add(base + b);
                    *ra = *rb; // Direct copy (setobjs2s)
                }
            }
            OpCode::LoadI => {
                // R[A] := sBx (signed integer immediate)
                let a = instr.get_a() as usize;
                let sbx = instr.get_sbx();

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    setivalue(ra, sbx as i64);
                }
            }
            OpCode::LoadF => {
                // R[A] := (float)sBx
                let a = instr.get_a() as usize;
                let sbx = instr.get_sbx();

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    setfltvalue(ra, sbx as f64);
                }
            }
            OpCode::LoadK => {
                // R[A] := K[Bx]
                let a = instr.get_a() as usize;
                let bx = instr.get_bx() as usize;

                if bx >= constants.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error(format!("LOADK: invalid constant index {}", bx)));
                }

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let rb = &constants[bx];
                    *ra = *rb; // setobj2s
                }
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
                    return Err(lua_state.error(format!("LOADKX: invalid constant index {}", ax)));
                }

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let rb = &constants[ax];
                    *ra = *rb; // setobj2s
                }
            }
            OpCode::LoadFalse => {
                // R[A] := false
                let a = instr.get_a() as usize;
                unsafe {
                    let ra = stack_ptr.add(base + a);
                    setbfvalue(ra);
                }
            }
            OpCode::LFalseSkip => {
                // R[A] := false; pc++
                let a = instr.get_a() as usize;
                unsafe {
                    let ra = stack_ptr.add(base + a);
                    setbfvalue(ra);
                }
                pc += 1; // Skip next instruction
            }
            OpCode::LoadTrue => {
                // R[A] := true
                let a = instr.get_a() as usize;
                unsafe {
                    let ra = stack_ptr.add(base + a);
                    setbtvalue(ra);
                }
            }
            OpCode::LoadNil => {
                // R[A], R[A+1], ..., R[A+B] := nil
                let a = instr.get_a() as usize;
                let mut b = instr.get_b() as usize;

                unsafe {
                    let mut ra = stack_ptr.add(base + a);
                    loop {
                        setnilvalue(ra);
                        if b == 0 {
                            break;
                        }
                        b -= 1;
                        ra = ra.add(1);
                    }
                }
            }
            OpCode::Add => {
                // op_arith(L, l_addi, luai_numadd)
                // R[A] := R[B] + R[C]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    // Fast path: both integers
                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        pc += 1; // Skip metamethod on success
                        setivalue(ra, i1.wrapping_add(i2));
                    }
                    // Slow path: try float conversion
                    else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                            pc += 1; // Skip metamethod on success
                            setfltvalue(ra, n1 + n2);
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
                    let v1 = stack_ptr.add(base + b);
                    let ra = stack_ptr.add(base + a);

                    // Fast path: integer
                    if ttisinteger(v1) {
                        let iv1 = ivalue(v1);
                        pc += 1; // Skip metamethod on success
                        setivalue(ra, iv1.wrapping_add(sc as i64));
                    }
                    // Slow path: float
                    else if ttisfloat(v1) {
                        let nb = fltvalue(v1);
                        let fimm = sc as f64;
                        pc += 1; // Skip metamethod on success
                        setfltvalue(ra, nb + fimm);
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
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        pc += 1;
                        setivalue(ra, i1.wrapping_sub(i2));
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(ra, n1 - n2);
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
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        pc += 1;
                        setivalue(ra, i1.wrapping_mul(i2));
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(ra, n1 * n2);
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
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    let mut n1 = 0.0;
                    let mut n2 = 0.0;
                    if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                        pc += 1;
                        setfltvalue(ra, n1 / n2);
                    }
                }
            }
            OpCode::IDiv => {
                // op_arith(L, luaV_idiv, luai_numidiv) - 整数除法
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        if i2 != 0 {
                            pc += 1;
                            setivalue(ra, i1.div_euclid(i2));
                        }
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(ra, (n1 / n2).floor());
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
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        if i2 != 0 {
                            pc += 1;
                            setivalue(ra, i1.rem_euclid(i2));
                        }
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(ra, n1 - (n1 / n2).floor() * n2);
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
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    let mut n1 = 0.0;
                    let mut n2 = 0.0;
                    if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                        pc += 1;
                        setfltvalue(ra, n1.powf(n2));
                    }
                }
            }
            OpCode::Unm => {
                // 取负: -value
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;

                unsafe {
                    let rb = stack_ptr.add(base + b);
                    let ra = stack_ptr.add(base + a);

                    if ttisinteger(rb) {
                        let ib = ivalue(rb);
                        setivalue(ra, ib.wrapping_neg());
                    } else {
                        let mut nb = 0.0;
                        if tonumberns(rb, &mut nb) {
                            setfltvalue(ra, -nb);
                        }
                        // else: metamethod
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
                    let v1 = stack_ptr.add(base + b);
                    let v2 = constants.get_unchecked(c); // K[C]
                    let ra = stack_ptr.add(base + a);

                    let mut i2 = 0i64;
                    if ttisinteger(v1) && tointeger(v2, &mut i2) {
                        let i1 = ivalue(v1);
                        pc += 1;
                        setivalue(ra, i1.wrapping_add(i2));
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(ra, n1 + n2);
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
                    let v1 = stack_ptr.add(base + b);
                    let v2 = constants.get_unchecked(c);
                    let ra = stack_ptr.add(base + a);

                    let mut i2 = 0i64;
                    if ttisinteger(v1) && tointeger(v2, &mut i2) {
                        let i1 = ivalue(v1);
                        pc += 1;
                        setivalue(ra, i1.wrapping_sub(i2));
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(ra, n1 - n2);
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
                    let v1 = stack_ptr.add(base + b);
                    let v2 = constants.get_unchecked(c);
                    let ra = stack_ptr.add(base + a);

                    let mut i2 = 0i64;
                    if ttisinteger(v1) && tointeger(v2, &mut i2) {
                        let i1 = ivalue(v1);
                        pc += 1;
                        setivalue(ra, i1.wrapping_mul(i2));
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            pc += 1;
                            setfltvalue(ra, n1 * n2);
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
                    let v1 = stack_ptr.add(base + b);
                    let v2 = constants.get_unchecked(c);
                    let ra = stack_ptr.add(base + a);

                    let mut i2 = 0i64;
                    if ttisinteger(v1) && tointeger(v2, &mut i2) {
                        let i1 = ivalue(v1);
                        if i2 != 0 {
                            pc += 1;
                            let result = i1 - (i1 / i2) * i2;
                            setivalue(ra, result);
                        }
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            if n2 != 0.0 {
                                pc += 1;
                                setfltvalue(ra, n1 - (n1 / n2).floor() * n2);
                            }
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
                    let v1 = stack_ptr.add(base + b);
                    let v2 = constants.get_unchecked(c);
                    let ra = stack_ptr.add(base + a);

                    let mut n1 = 0.0;
                    let mut n2 = 0.0;
                    if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                        pc += 1;
                        setfltvalue(ra, n1.powf(n2));
                    }
                }
            }
            OpCode::DivK => {
                // R[A] := R[B] / K[C] (float division)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = constants.get_unchecked(c);
                    let ra = stack_ptr.add(base + a);

                    let mut n1 = 0.0;
                    let mut n2 = 0.0;
                    if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                        pc += 1;
                        setfltvalue(ra, n1 / n2);
                    }
                }
            }
            OpCode::IDivK => {
                // R[A] := R[B] // K[C] (floor division)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = constants.get_unchecked(c);
                    let ra = stack_ptr.add(base + a);

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
                            setivalue(ra, result);
                        }
                    } else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumber(v2, &mut n2) {
                            if n2 != 0.0 {
                                pc += 1;
                                setfltvalue(ra, (n1 / n2).floor());
                            }
                        }
                    }
                }
            }
            OpCode::BAndK => {
                // R[A] := R[B] & K[C]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = constants.get_unchecked(c);
                    let ra = stack_ptr.add(base + a);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(ra, i1 & i2);
                    }
                }
            }
            OpCode::BOrK => {
                // R[A] := R[B] | K[C]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = constants.get_unchecked(c);
                    let ra = stack_ptr.add(base + a);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(ra, i1 | i2);
                    }
                }
            }
            OpCode::BXorK => {
                // R[A] := R[B] ^ K[C] (bitwise xor)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = constants.get_unchecked(c);
                    let ra = stack_ptr.add(base + a);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointeger(v2, &mut i2) {
                        pc += 1;
                        setivalue(ra, i1 ^ i2);
                    }
                }
            }
            OpCode::BAnd => {
                // op_bitwise(L, l_band)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(ra, i1 & i2);
                    }
                }
            }
            OpCode::BOr => {
                // op_bitwise(L, l_bor)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(ra, i1 | i2);
                    }
                }
            }
            OpCode::BXor => {
                // op_bitwise(L, l_bxor)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        setivalue(ra, i1 ^ i2);
                    }
                }
            }
            OpCode::Shl => {
                // op_bitwise(L, luaV_shiftl)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        // Lua wraps shift amount to 0-63
                        let shift = (i2 & 63) as u32;
                        setivalue(ra, i1.wrapping_shl(shift));
                    }
                }
            }
            OpCode::Shr => {
                // op_bitwise(L, luaV_shiftr)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let v1 = stack_ptr.add(base + b);
                    let v2 = stack_ptr.add(base + c);
                    let ra = stack_ptr.add(base + a);

                    let mut i1 = 0i64;
                    let mut i2 = 0i64;
                    if tointegerns(v1, &mut i1) && tointegerns(v2, &mut i2) {
                        pc += 1;
                        // Lua wraps shift amount to 0-63
                        let shift = (i2 & 63) as u32;
                        setivalue(ra, (i1 as u64).wrapping_shr(shift) as i64);
                    }
                }
            }
            OpCode::BNot => {
                // 按位非: ~value
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;

                unsafe {
                    let rb = stack_ptr.add(base + b);
                    let ra = stack_ptr.add(base + a);

                    let mut ib = 0i64;
                    if tointegerns(rb, &mut ib) {
                        setivalue(ra, !ib);
                    }
                    // else: metamethod
                }
            }
            OpCode::ShlI => {
                // R[A] := sC << R[B]
                // Note: In Lua 5.5, SHLI is immediate << register (not register << immediate)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let ic = instr.get_sc(); // shift amount from immediate

                unsafe {
                    let rb = stack_ptr.add(base + b);
                    let ra = stack_ptr.add(base + a);

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
                        setivalue(ra, result);
                    }
                    // else: metamethod
                }
            }
            OpCode::ShrI => {
                // R[A] := R[B] >> sC
                // Arithmetic right shift
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let ic = instr.get_sc(); // shift amount

                unsafe {
                    let rb = stack_ptr.add(base + b);
                    let ra = stack_ptr.add(base + a);

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
                        setivalue(ra, result);
                    }
                    // else: metamethod
                }
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
                lua_state.set_frame_pc(frame_idx, pc as u32);

                return return_handler::handle_return(
                    lua_state, stack_ptr, base, frame_idx, a, b, c, k,
                );
            }
            OpCode::Return0 => {
                // return (no values)
                lua_state.set_frame_pc(frame_idx, pc as u32);
                return return_handler::handle_return0(lua_state, frame_idx);
            }
            OpCode::Return1 => {
                // return R[A]
                let a = instr.get_a() as usize;
                lua_state.set_frame_pc(frame_idx, pc as u32);
                return return_handler::handle_return1(lua_state, stack_ptr, base, frame_idx, a);
            }
            OpCode::GetUpval => {
                // R[A] := UpValue[B]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;

                // 优化：直接使用缓存的upvalues_vec，避免查找function
                if b >= upvalues_vec.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error(format!("GETUPVAL: invalid upvalue index {}", b)));
                }

                let upval_id = upvalues_vec[b];
                let upvalue = lua_state
                    .vm_mut()
                    .object_pool
                    .get_upvalue(upval_id)
                    .ok_or(LuaError::RuntimeError)?;

                // Get value from upvalue
                let value = if upvalue.is_open() {
                    // Open: read from stack
                    let stack_idx = upvalue.get_stack_index().unwrap();
                    lua_state.stack_get(stack_idx).unwrap_or(LuaValue::nil())
                } else {
                    // Closed: read from upvalue storage
                    upvalue.get_closed_value().unwrap()
                };

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    *ra = value;
                }
            }
            OpCode::SetUpval => {
                // UpValue[B] := R[A]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;

                // Get value to set
                let value = unsafe {
                    let ra = stack_ptr.add(base + a);
                    *ra
                };

                // 优化：直接使用缓存的upvalues_vec
                if b >= upvalues_vec.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error(format!("SETUPVAL: invalid upvalue index {}", b)));
                }

                let upval_id = upvalues_vec[b];

                // Set value in upvalue
                let upvalue = lua_state
                    .vm_mut()
                    .object_pool
                    .get_upvalue_mut(upval_id)
                    .ok_or(LuaError::RuntimeError)?;

                if upvalue.is_open() {
                    // Open: write to stack
                    let stack_idx = upvalue.get_stack_index().unwrap();
                    lua_state.stack_set(stack_idx, value)?;
                } else {
                    // Closed: write to upvalue storage
                    unsafe {
                        upvalue.set_closed_value_unchecked(value);
                    }
                }

                // TODO: GC barrier (luaC_barrier)
            }
            OpCode::Close => {
                // Close all upvalues >= R[A]
                let a = instr.get_a() as usize;
                let close_from = base + a;

                // Close upvalues at or above this level
                closure_handler::close_upvalues_at_level(lua_state, close_from)?;
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
                let value = lua_state.create_table(vc, hash_size);

                // IMPORTANT: create_table may trigger GC, refresh stack_ptr
                stack_ptr = lua_state.stack_ptr_mut();

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    *ra = value;
                }
            }
            OpCode::GetTable => {
                // R[A] := R[B][R[C]]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let rb = stack_ptr.add(base + b);
                    let rc = stack_ptr.add(base + c);

                    // Fast path for table with integer key
                    if let Some(table_id) = (*rb).as_table_id() {
                        if ttisinteger(rc) {
                            let key = ivalue(rc);
                            let table = lua_state
                                .vm_mut()
                                .object_pool
                                .get_table(table_id)
                                .ok_or(LuaError::RuntimeError)?;

                            let result = table.get_int(key).unwrap_or(LuaValue::nil());
                            let ra = stack_ptr.add(base + a);
                            *ra = result;
                        } else {
                            // General case: use key as LuaValue
                            let key = *rc;
                            let table = lua_state
                                .vm_mut()
                                .object_pool
                                .get_table(table_id)
                                .ok_or(LuaError::RuntimeError)?;

                            let result = table.raw_get(&key).unwrap_or(LuaValue::nil());
                            let ra = stack_ptr.add(base + a);
                            *ra = result;
                        }
                    } else {
                        // Not a table - should trigger metamethod
                        // For now, return nil
                        let ra = stack_ptr.add(base + a);
                        setnilvalue(ra);
                    }
                }
            }
            OpCode::GetI => {
                // R[A] := R[B][C] (integer key)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                unsafe {
                    let rb = stack_ptr.add(base + b);

                    if let Some(table_id) = (*rb).as_table_id() {
                        let table = lua_state
                            .vm_mut()
                            .object_pool
                            .get_table(table_id)
                            .ok_or(LuaError::RuntimeError)?;

                        let result = table.get_int(c as i64).unwrap_or(LuaValue::nil());
                        let ra = stack_ptr.add(base + a);
                        *ra = result;
                    } else {
                        // Not a table - should trigger metamethod
                        let ra = stack_ptr.add(base + a);
                        setnilvalue(ra);
                    }
                }
            }
            OpCode::GetField => {
                // R[A] := R[B][K[C]:string]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                if c >= constants.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error(format!("GETFIELD: invalid constant index {}", c)));
                }

                unsafe {
                    let rb = stack_ptr.add(base + b);
                    let key = &constants[c];

                    if let Some(table_id) = (*rb).as_table_id() {
                        let table = lua_state
                            .vm_mut()
                            .object_pool
                            .get_table(table_id)
                            .ok_or(LuaError::RuntimeError)?;

                        let result = table.raw_get(key).unwrap_or(LuaValue::nil());
                        let ra = stack_ptr.add(base + a);
                        *ra = result;
                    } else {
                        // Not a table
                        let ra = stack_ptr.add(base + a);
                        setnilvalue(ra);
                    }
                }
            }
            OpCode::SetTable => {
                // R[A][R[B]] := RK(C)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let rb = stack_ptr.add(base + b);

                    // Get value (RK: register or constant)
                    let value = if k {
                        if c >= constants.len() {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("SETTABLE: invalid constant".to_string()));
                        }
                        constants[c]
                    } else {
                        let rc = stack_ptr.add(base + c);
                        *rc
                    };

                    if let Some(table_id) = (*ra).as_table_id() {
                        let key = *rb;

                        // Fast path for integer key
                        if ttisinteger(rb) {
                            let int_key = ivalue(rb);
                            let table = lua_state
                                .vm_mut()
                                .object_pool
                                .get_table_mut(table_id)
                                .ok_or(LuaError::RuntimeError)?;
                            table.set_int(int_key, value);
                        } else {
                            let table = lua_state
                                .vm_mut()
                                .object_pool
                                .get_table_mut(table_id)
                                .ok_or(LuaError::RuntimeError)?;
                            table.raw_set(key, value);
                        }
                    }
                    // else: should trigger metamethod
                }
            }
            OpCode::SetI => {
                // R[A][B] := RK(C) (integer key)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    // Get value (RK: register or constant)
                    let value = if k {
                        if c >= constants.len() {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("SETI: invalid constant".to_string()));
                        }
                        constants[c]
                    } else {
                        let rc = stack_ptr.add(base + c);
                        *rc
                    };

                    if let Some(table_id) = (*ra).as_table_id() {
                        let table = lua_state
                            .vm_mut()
                            .object_pool
                            .get_table_mut(table_id)
                            .ok_or(LuaError::RuntimeError)?;
                        table.set_int(b as i64, value);
                    }
                    // else: should trigger metamethod
                }
            }
            OpCode::SetField => {
                // R[A][K[B]:string] := RK(C)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;
                let k = instr.get_k();

                if b >= constants.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error(format!("SETFIELD: invalid constant index {}", b)));
                }

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let key = &constants[b];

                    // Get value (RK: register or constant)
                    let value = if k {
                        if c >= constants.len() {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("SETFIELD: invalid constant".to_string()));
                        }
                        constants[c]
                    } else {
                        let rc = stack_ptr.add(base + c);
                        *rc
                    };

                    if let Some(table_id) = (*ra).as_table_id() {
                        let table = lua_state
                            .vm_mut()
                            .object_pool
                            .get_table_mut(table_id)
                            .ok_or(LuaError::RuntimeError)?;
                        table.raw_set(*key, value);
                    }
                    // else: should trigger metamethod
                }
            }
            OpCode::Self_ => {
                // R[A+1] := R[B]; R[A] := R[B][K[C]:string]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                if c >= constants.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error(format!("SELF: invalid constant index {}", c)));
                }

                unsafe {
                    let rb = stack_ptr.add(base + b);
                    let ra = stack_ptr.add(base + a);
                    let ra1 = ra.add(1);

                    // R[A+1] := R[B] (save object)
                    *ra1 = *rb;

                    // R[A] := R[B][K[C]] (get method)
                    let key = &constants[c];

                    if let Some(table_id) = (*rb).as_table_id() {
                        let table = lua_state
                            .vm_mut()
                            .object_pool
                            .get_table(table_id)
                            .ok_or(LuaError::RuntimeError)?;

                        let result = table.raw_get(key).unwrap_or(LuaValue::nil());
                        *ra = result;
                    } else {
                        // Not a table - should trigger metamethod
                        setnilvalue(ra);
                    }
                }
            }
            OpCode::Call => {
                // R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                // Save PC before call
                lua_state.set_frame_pc(frame_idx, pc as u32);

                // Delegate to call handler - returns FrameAction
                match call::handle_call(lua_state, base, a, b, c) {
                    Ok(FrameAction::Continue) => {
                        // IMPORTANT: function call may grow stack, refresh stack_ptr
                        stack_ptr = lua_state.stack_ptr_mut();
                    }
                    other => return other,
                }
            }
            OpCode::TailCall => {
                // Tail call optimization: return R[A](R[A+1], ... ,R[A+B-1])
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;

                // Save PC before call
                lua_state.set_frame_pc(frame_idx, pc as u32);

                // Delegate to tailcall handler (returns FrameAction)
                match call::handle_tailcall(lua_state, base, a, b) {
                    Ok(FrameAction::Continue) => {
                        // IMPORTANT: tail call may grow stack, refresh stack_ptr
                        stack_ptr = lua_state.stack_ptr_mut();
                    }
                    other => return other,
                }
            }
            OpCode::Not => {
                // R[A] := not R[B]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;

                unsafe {
                    let rb = stack_ptr.add(base + b);
                    let ra = stack_ptr.add(base + a);

                    // l_isfalse: nil or false
                    let is_false = (*rb).tt_ == LUA_VFALSE || (*rb).is_nil();
                    if is_false {
                        setbtvalue(ra);
                    } else {
                        setbfvalue(ra);
                    }
                }
            }
            OpCode::ForLoop => {
                // Numeric for loop
                // If integer: check counter, decrement, add step, jump back
                // If float: add step, check limit, jump back
                let a = instr.get_a() as usize;
                let bx = instr.get_bx() as usize;

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    // Check if integer loop
                    if ttisinteger(ra.add(1)) {
                        // Integer loop
                        // ra: counter (count of iterations left)
                        // ra+1: step
                        // ra+2: control variable (idx)
                        let count = ivalue(ra) as u64; // unsigned count
                        if count > 0 {
                            // More iterations
                            let step = ivalue(ra.add(1));
                            let idx = ivalue(ra.add(2));

                            // Update counter (decrement) - 用chgivalue避免重设类型标签
                            chgivalue(ra, (count - 1) as i64);

                            // Update control variable: idx += step
                            let new_idx = idx.wrapping_add(step);
                            chgivalue(ra.add(2), new_idx);

                            // Jump back
                            if bx > pc {
                                lua_state.set_frame_pc(frame_idx, pc as u32);
                                return Err(lua_state.error("FORLOOP: invalid jump".to_string()));
                            }
                            pc -= bx;
                        }
                        // else: counter expired, exit loop
                    } else {
                        // Float loop
                        // ra: limit
                        // ra+1: step
                        // ra+2: idx (control variable)
                        let step = fltvalue(ra.add(1));
                        let limit = fltvalue(ra);
                        let idx = fltvalue(ra.add(2));

                        // idx += step
                        let new_idx = idx + step;

                        // Check if should continue
                        let should_continue = if step > 0.0 {
                            new_idx <= limit
                        } else {
                            new_idx >= limit
                        };

                        if should_continue {
                            // Update control variable - 用chgfltvalue避免重设类型标签
                            chgfltvalue(ra.add(2), new_idx);

                            // Jump back
                            if bx > pc {
                                lua_state.set_frame_pc(frame_idx, pc as u32);
                                return Err(lua_state.error("FORLOOP: invalid jump".to_string()));
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

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let pinit = ra;
                    let plimit = ra.add(1);
                    let pstep = ra.add(2);

                    // Check if integer loop
                    if ttisinteger(pinit) && ttisinteger(pstep) {
                        // Integer loop
                        let init = ivalue(pinit);
                        let step = ivalue(pstep);

                        if step == 0 {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("'for' step is zero".to_string()));
                        }

                        // Get limit (may need conversion)
                        let mut limit = 0i64;
                        if ttisinteger(plimit) {
                            limit = ivalue(plimit);
                        } else if ttisfloat(plimit) {
                            let flimit = fltvalue(plimit);
                            if step < 0 {
                                limit = flimit.ceil() as i64;
                            } else {
                                limit = flimit.floor() as i64;
                            }
                        } else {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("'for' limit must be a number".to_string()));
                        }

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
                            setivalue(ra, count as i64);
                            setivalue(ra.add(1), step);
                            setivalue(ra.add(2), init);
                        }
                    } else {
                        // Float loop
                        let mut init = 0.0;
                        let mut limit = 0.0;
                        let mut step = 0.0;

                        if !tonumberns(plimit, &mut limit) {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("'for' limit must be a number".to_string()));
                        }
                        if !tonumberns(pstep, &mut step) {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("'for' step must be a number".to_string()));
                        }
                        if !tonumberns(pinit, &mut init) {
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
                            setfltvalue(ra, limit);
                            setfltvalue(ra.add(1), step);
                            setfltvalue(ra.add(2), init);
                        }
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

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    // Swap control and closing variables
                    let temp = *ra.add(3); // closing
                    *ra.add(3) = *ra.add(2); // control -> closing position
                    *ra.add(2) = temp; // closing -> control position

                    // TODO: Mark ra+2 as to-be-closed if not nil
                    // For now, skip TBC handling

                    // Jump to loop end (+ Bx)
                    pc += bx;
                }
            }
            OpCode::TForCall => {
                // Generic for loop call
                // Call: ra+3,ra+4,...,ra+2+C := ra(ra+1, ra+2)
                // ra=iterator, ra+1=state, ra+2=closing, ra+3=control
                let a = instr.get_a() as usize;
                let c = instr.get_c() as usize;

                // Get values before modifying stack
                let ra_base = base + a;
                let iterator = unsafe { *stack_ptr.add(ra_base) };
                let state = unsafe { *stack_ptr.add(ra_base + 1) };
                let control = unsafe { *stack_ptr.add(ra_base + 3) };

                // Setup call stack using safe API:
                // ra+3: function (copy from ra)
                // ra+4: arg1 (copy from ra+1, state)
                // ra+5: arg2 (copy from ra+3, control variable)
                lua_state.stack_set(ra_base + 3, iterator)?;
                lua_state.stack_set(ra_base + 4, state)?;
                lua_state.stack_set(ra_base + 5, control)?;

                // Refresh stack pointer after potential reallocation
                stack_ptr = lua_state.stack_ptr_mut();

                // Save PC before call
                lua_state.set_frame_pc(frame_idx, pc as u32);

                // Call iterator function at base+a+3
                // Arguments: 2 (state and control)
                // Results: c (number of loop variables)
                match call::handle_call(lua_state, base, a + 3, 3, c + 1) {
                    Ok(FrameAction::Continue) => {
                        // IMPORTANT: iterator call may grow stack, refresh stack_ptr
                        stack_ptr = lua_state.stack_ptr_mut();
                        // C function completed, results already in place
                        // Fall through to next instruction (TFORLOOP)
                    }
                    other => return other,
                }
            }
            OpCode::TForLoop => {
                // Generic for loop test
                // If ra+3 != nil then ra+2 = ra+3 and jump back
                let a = instr.get_a() as usize;
                let bx = instr.get_bx() as usize;

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    // Check if ra+3 (new control value) is not nil
                    if !(*ra.add(3)).is_nil() {
                        // Continue loop: update control variable and jump back
                        *ra.add(2) = *ra.add(3);

                        if bx > pc {
                            lua_state.set_frame_pc(frame_idx, pc as u32);
                            return Err(lua_state.error("TFORLOOP: invalid jump".to_string()));
                        }
                        pc -= bx;
                    }
                    // else: exit loop (control variable is nil)
                }
            }
            OpCode::MmBin => {
                // Call metamethod over R[A] and R[B]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                // Save PC before metamethod call
                lua_state.set_frame_pc(frame_idx, pc as u32);

                // Delegate to metamethod handler
                metamethod::handle_mmbin(lua_state, base, a, b, c, pc, code)?;
            }
            OpCode::MmBinI => {
                // Call metamethod over R[A] and immediate sB
                let a = instr.get_a() as usize;
                let sb = instr.get_sb();
                let c = instr.get_c() as usize;
                let k = instr.get_k();

                // Save PC before metamethod call
                lua_state.set_frame_pc(frame_idx, pc as u32);

                // Delegate to metamethod handler
                metamethod::handle_mmbini(lua_state, base, a, sb, c, k, pc, code)?;
            }
            OpCode::MmBinK => {
                // Call metamethod over R[A] and K[B]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;
                let k = instr.get_k();

                // Save PC before metamethod call
                lua_state.set_frame_pc(frame_idx, pc as u32);

                // Delegate to metamethod handler
                metamethod::handle_mmbink(lua_state, base, a, b, c, k, pc, code, constants)?;
            }

            // ============================================================
            // UPVALUE TABLE ACCESS
            // ============================================================
            OpCode::GetTabUp => {
                // R[A] := UpValue[B][K[C]:shortstring]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;

                // Get upvalue B (usually _ENV for global access)
                if b >= upvalues_vec.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error(format!("GETTABUP: invalid upvalue index {}", b)));
                }

                let upval_id = upvalues_vec[b];
                let upvalue = lua_state
                    .vm_mut()
                    .object_pool
                    .get_upvalue(upval_id)
                    .ok_or(LuaError::RuntimeError)?;

                // Get table value from upvalue
                let table_value = if upvalue.is_open() {
                    let stack_idx = upvalue.get_stack_index().unwrap();
                    lua_state.stack_get(stack_idx).unwrap_or(LuaValue::nil())
                } else {
                    upvalue.get_closed_value().unwrap()
                };

                // Get key from constants (K[C])
                if c >= constants.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error(format!("GETTABUP: invalid constant index {}", c)));
                }
                let key = &constants[c];

                // Get value from table[key]
                let result = if let Some(table_id) = table_value.as_table_id() {
                    let table = lua_state
                        .vm_mut()
                        .object_pool
                        .get_table(table_id)
                        .ok_or(LuaError::RuntimeError)?;
                    table.raw_get(key).unwrap_or(LuaValue::nil())
                } else {
                    // Not a table - return nil (or should trigger metamethod)
                    LuaValue::nil()
                };

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    *ra = result;
                }
            }

            OpCode::SetTabUp => {
                // UpValue[A][K[B]:shortstring] := RK(C)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;
                let k = instr.get_k();

                // Get upvalue A (usually _ENV for global access)
                if a >= upvalues_vec.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error(format!("SETTABUP: invalid upvalue index {}", a)));
                }

                let upval_id = upvalues_vec[a];
                let upvalue = lua_state
                    .vm_mut()
                    .object_pool
                    .get_upvalue(upval_id)
                    .ok_or(LuaError::RuntimeError)?;

                // Get table value from upvalue
                let table_value = if upvalue.is_open() {
                    let stack_idx = upvalue.get_stack_index().unwrap();
                    lua_state.stack_get(stack_idx).unwrap_or(LuaValue::nil())
                } else {
                    upvalue.get_closed_value().unwrap()
                };

                // Get key from constants (K[B])
                if b >= constants.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error(format!("SETTABUP: invalid constant index {}", b)));
                }
                let key = constants[b];

                // Get value (RK: register or constant)
                let value = if k {
                    if c >= constants.len() {
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(lua_state.error("SETTABUP: invalid constant".to_string()));
                    }
                    constants[c]
                } else {
                    unsafe {
                        let rc = stack_ptr.add(base + c);
                        *rc
                    }
                };

                // Set table[key] = value
                if let Some(table_id) = table_value.as_table_id() {
                    let table = lua_state
                        .vm_mut()
                        .object_pool
                        .get_table_mut(table_id)
                        .ok_or(LuaError::RuntimeError)?;
                    table.raw_set(key, value);
                }
                // else: should trigger metamethod, but we skip for now
            }

            // ============================================================
            // LENGTH AND CONCATENATION
            // ============================================================
            OpCode::Len => {
                // R[A] := #R[B]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;

                unsafe {
                    let rb = stack_ptr.add(base + b);
                    let ra = stack_ptr.add(base + a);

                    // Get length based on type
                    if let Some(string_id) = (*rb).as_string_id() {
                        // String: get length from object pool
                        if let Some(s) = lua_state.vm_mut().object_pool.get_string(string_id) {
                            let len = s.as_str().len();
                            setivalue(ra, len as i64);
                        } else {
                            setivalue(ra, 0);
                        }
                    } else if let Some(table_id) = (*rb).as_table_id() {
                        // Table: use raw length (array part length)
                        // Note: __len metamethod excluded per user request
                        if let Some(table) = lua_state.vm_mut().object_pool.get_table(table_id) {
                            let len = table.len();
                            setivalue(ra, len as i64);
                        } else {
                            setivalue(ra, 0);
                        }
                    } else {
                        // Other types: length is 0
                        // Note: __len metamethod excluded per user request
                        setivalue(ra, 0);
                    }
                }
            }

            OpCode::Concat => {
                // R[A] := R[A].. ... ..R[A + B - 1]
                // Concatenate B values starting from R[A]
                // Optimized implementation matching Lua 5.5
                let a = instr.get_a() as usize;
                let n = instr.get_b() as usize;

                match concat::concat_strings(lua_state, stack_ptr, base, a, n) {
                    Ok(result) => {
                        // IMPORTANT: concat may allocate strings and trigger GC, refresh stack_ptr
                        stack_ptr = lua_state.stack_ptr_mut();

                        unsafe {
                            let ra = stack_ptr.add(base + a);
                            *ra = result;
                        }
                    }
                    Err(err) => {
                        return Err(err);
                    }
                }
            }

            // ============================================================
            // COMPARISON OPERATIONS (register-register)
            // ============================================================
            OpCode::Eq => {
                // if ((R[A] == R[B]) ~= k) then pc++; else donextjump
                // Lua 5.5: docondjump() - if cond != k, skip next; else execute next (JMP)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let rb = stack_ptr.add(base + b);

                    // Simple equality check (TODO: metamethod)
                    let cond = (*ra).raw_equal(&*rb, &lua_state.vm_mut().object_pool);
                    if cond != k {
                        pc += 1; // Condition failed - skip next instruction
                    }
                    // else: Condition succeeded - execute next instruction (must be JMP)
                }
            }

            OpCode::Lt => {
                // if ((R[A] < R[B]) ~= k) then pc++; else donextjump
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let rb = stack_ptr.add(base + b);

                    let cond = if ttisinteger(ra) && ttisinteger(rb) {
                        ivalue(ra) < ivalue(rb)
                    } else if ttisnumber(ra) && ttisnumber(rb) {
                        let mut na = 0.0;
                        let mut nb = 0.0;
                        tonumberns(ra, &mut na);
                        tonumberns(rb, &mut nb);
                        na < nb
                    } else {
                        false // TODO: metamethod
                    };

                    if cond != k {
                        pc += 1; // Condition failed - skip next instruction
                    }
                    // else: Condition succeeded - execute next instruction (must be JMP)
                }
            }

            OpCode::Le => {
                // if ((R[A] <= R[B]) ~= k) then pc++; else donextjump
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let rb = stack_ptr.add(base + b);

                    let cond = if ttisinteger(ra) && ttisinteger(rb) {
                        ivalue(ra) <= ivalue(rb)
                    } else if ttisnumber(ra) && ttisnumber(rb) {
                        let mut na = 0.0;
                        let mut nb = 0.0;
                        tonumberns(ra, &mut na);
                        tonumberns(rb, &mut nb);
                        na <= nb
                    } else {
                        false // TODO: metamethod
                    };

                    if cond != k {
                        pc += 1; // Condition failed - skip next instruction
                    }
                    // else: Condition succeeded - execute next instruction (must be JMP)
                }
            }

            // ============================================================
            // COMPARISON WITH CONSTANT/IMMEDIATE
            // ============================================================
            OpCode::EqK => {
                // if ((R[A] == K[B]) ~= k) then pc++; else donextjump
                // Lua 5.5: docondjump() - if cond != k, skip next; else execute next (JMP)
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let kb = constants.get(b).unwrap();

                    // Raw equality (no metamethods for constants)
                    let cond = (*ra).raw_equal(kb, &lua_state.vm_mut().object_pool);
                    if cond != k {
                        pc += 1; // Condition failed - skip next instruction
                    }
                    // else: Condition succeeded - execute next instruction (must be JMP)
                }
            }

            OpCode::EqI => {
                // if ((R[A] == sB) ~= k) then pc++; else donextjump
                let a = instr.get_a() as usize;
                let sb = instr.get_sb();
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    let cond = if ttisinteger(ra) {
                        ivalue(ra) == (sb as i64)
                    } else if ttisfloat(ra) {
                        fltvalue(ra) == (sb as f64)
                    } else {
                        false
                    };

                    if cond != k {
                        pc += 1; // Condition failed - skip next instruction
                    }
                    // else: Condition succeeded - execute next instruction (must be JMP)
                }
            }

            OpCode::LtI => {
                // if ((R[A] < sB) ~= k) then pc++; else donextjump
                let a = instr.get_a() as usize;
                let im = instr.get_sb();
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    let cond = if ttisinteger(ra) {
                        ivalue(ra) < (im as i64)
                    } else if ttisfloat(ra) {
                        fltvalue(ra) < (im as f64)
                    } else {
                        false // TODO: metamethod
                    };

                    if cond != k {
                        pc += 1; // Condition failed - skip next instruction
                    }
                    // else: Condition succeeded - execute next instruction (must be JMP)
                }
            }

            OpCode::LeI => {
                // if ((R[A] <= sB) ~= k) then pc++; else donextjump
                let a = instr.get_a() as usize;
                let im = instr.get_sb();
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    let cond = if ttisinteger(ra) {
                        ivalue(ra) <= (im as i64)
                    } else if ttisfloat(ra) {
                        fltvalue(ra) <= (im as f64)
                    } else {
                        false // TODO: metamethod
                    };

                    if cond != k {
                        pc += 1; // Condition failed - skip next instruction
                    }
                    // else: Condition succeeded - execute next instruction (must be JMP)
                }
            }

            OpCode::GtI => {
                // if ((R[A] > sB) ~= k) then pc++ (implemented as !(A <= B))
                let a = instr.get_a() as usize;
                let im = instr.get_sb();
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    let cond = if ttisinteger(ra) {
                        ivalue(ra) > (im as i64)
                    } else if ttisfloat(ra) {
                        fltvalue(ra) > (im as f64)
                    } else {
                        false // TODO: metamethod
                    };

                    if cond != k {
                        pc += 1; // Condition failed - skip next instruction
                    }
                    // else: Condition succeeded - execute next instruction (must be JMP)
                }
            }

            OpCode::GeI => {
                // if ((R[A] >= sB) ~= k) then pc++; else donextjump
                let a = instr.get_a() as usize;
                let im = instr.get_sb();
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    let cond = if ttisinteger(ra) {
                        ivalue(ra) >= (im as i64)
                    } else if ttisfloat(ra) {
                        fltvalue(ra) >= (im as f64)
                    } else {
                        false // TODO: metamethod
                    };

                    if cond != k {
                        pc += 1; // Condition failed - skip next instruction
                    }
                    // else: Condition succeeded - execute next instruction (must be JMP)
                }
            }

            // ============================================================
            // CONDITIONAL TESTS
            // ============================================================
            OpCode::Test => {
                // docondjump(): if (cond != k) then pc++ else donextjump
                let a = instr.get_a() as usize;
                let k = instr.get_k();

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    // l_isfalse: nil or false
                    let is_false =
                        (*ra).is_nil() || ((*ra).is_boolean() && (*ra).tt_ == LUA_VFALSE);
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
            }

            OpCode::TestSet => {
                // if (l_isfalse(R[B]) == k) then pc++ else R[A] := R[B]; donextjump
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let k = instr.get_k();

                unsafe {
                    let rb = stack_ptr.add(base + b);
                    let is_false =
                        (*rb).is_nil() || ((*rb).is_boolean() && (*rb).tt_ == LUA_VFALSE);

                    if is_false == k {
                        pc += 1; // Condition failed - skip next instruction (JMP)
                    } else {
                        // Condition succeeded - copy value and EXECUTE next instruction (must be JMP)
                        let ra = stack_ptr.add(base + a);
                        *ra = *rb;
                        // donextjump: fetch and execute next JMP instruction
                        let next_instr = unsafe { *chunk.code.get_unchecked(pc) };
                        debug_assert!(next_instr.get_opcode() == OpCode::Jmp);
                        pc += 1; // Move past the JMP instruction
                        let sj = next_instr.get_sj();
                        pc = (pc as i32 + sj) as usize; // Execute the jump
                    }
                }
            }

            // ============================================================
            // TABLE OPERATIONS
            // ============================================================
            OpCode::SetList => {
                // R[A][vC+i] := R[A+i], 1 <= i <= vB
                // Batch set table elements (used in table constructors)
                let a = instr.get_a() as usize;
                let mut vb = instr.get_vb() as usize; // number of elements
                let mut vc = instr.get_vc() as usize; // starting index offset
                let k = instr.get_k();

                // Check for EXTRAARG for larger starting indices
                if k {
                    if pc < code.len() {
                        let extra_instr = code[pc];

                        if extra_instr.get_opcode() == OpCode::ExtraArg {
                            pc += 1; // Consume EXTRAARG
                            let extra = extra_instr.get_ax() as usize;
                            // Add extra to starting index: vc += extra * (MAXARG_vC + 1)
                            // MAXARG_vC is 10 bits = 1023, so + 1 = 1024
                            vc += extra * 1024;
                        }
                    }
                }

                // If vB == 0, use all values from ra+1 to top
                if vb == 0 {
                    let stack_top = lua_state.get_top();
                    let ra = base + a;
                    vb = if stack_top > ra + 1 {
                        stack_top - ra - 1
                    } else {
                        0
                    };
                }

                // Get table from R[A]
                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let table_val = *ra;

                    if let Some(table_id) = table_val.as_table_id() {
                        let table = lua_state
                            .vm_mut()
                            .object_pool
                            .get_table_mut(table_id)
                            .ok_or(LuaError::RuntimeError)?;

                        // Set elements: table[vc+i] = R[A+i] for i=1..vb
                        for i in 1..=vb {
                            let val_ptr = ra.add(i);
                            let val = *val_ptr;
                            let index = (vc + i) as i64;
                            table.set_int(index, val);
                        }
                    }
                    // else: not a table, should error but we skip for now
                }
            }

            // ============================================================
            // CLOSURE AND VARARG
            // ============================================================
            OpCode::Closure => {
                // R[A] := closure(KPROTO[Bx])
                let a = instr.get_a() as usize;
                let bx = instr.get_bx() as usize;

                // Create closure from child prototype
                closure_handler::handle_closure(
                    lua_state,
                    stack_ptr,
                    base,
                    a,
                    bx,
                    &chunk,
                    &upvalues_vec,
                )?;

                // IMPORTANT: closure creation may trigger GC, refresh stack_ptr
                stack_ptr = lua_state.stack_ptr_mut();
            }

            OpCode::Vararg => {
                // R[A], ..., R[A+C-2] = varargs
                // Based on lvm.c:1936 and ltm.c:338 luaT_getvarargs
                let a = instr.get_a() as usize;
                let _b = instr.get_b() as usize; // vatab register (if k flag set)
                let c = instr.get_c() as usize;
                let _k = instr.get_k(); // whether B specifies vararg table register

                // n = number of results wanted (C-1), -1 means all
                let wanted = if c == 0 {
                    -1 // Get all varargs
                } else {
                    (c - 1) as i32
                };

                // Get nextraargs from CallInfo
                let call_info = lua_state.get_call_info(frame_idx);
                let nextra = call_info.nextraargs as usize;

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    if nextra == 0 {
                        // No varargs - fill with nil
                        if wanted < 0 {
                            // Getting all but there are none - stack top doesn't change
                        } else {
                            // Fill wanted slots with nil
                            for i in 0..(wanted as usize) {
                                setnilvalue(ra.add(i));
                            }
                        }
                    } else if wanted < 0 {
                        // Get all varargs
                        // varargs are stored after fixed parameters in the current frame
                        // They start at base - nextra (before the actual base)
                        if nextra > 0 && base >= nextra {
                            let vararg_start = base - nextra;
                            for i in 0..nextra {
                                let src = stack_ptr.add(vararg_start + i);
                                let dst = ra.add(i);
                                *dst = *src;
                            }
                            // Adjust stack top to include all varargs
                            let new_top = base + a + nextra;
                            lua_state.set_top(new_top);
                        }
                    } else {
                        // Get exactly 'wanted' varargs
                        let to_copy = if wanted as usize > nextra {
                            nextra
                        } else {
                            wanted as usize
                        };

                        // Copy available varargs
                        if nextra > 0 && base >= nextra && to_copy > 0 {
                            let vararg_start = base - nextra;
                            for i in 0..to_copy {
                                let src = stack_ptr.add(vararg_start + i);
                                let dst = ra.add(i);
                                *dst = *src;
                            }
                        }

                        // Fill remaining with nil
                        for i in to_copy..(wanted as usize) {
                            setnilvalue(ra.add(i));
                        }
                    }
                }
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

                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let rc = stack_ptr.add(base + c);

                    // Check if R[C] is string "n" (get vararg count)
                    if let Some(string_id) = (*rc).as_string_id() {
                        if let Some(s) = lua_state.vm_mut().object_pool.get_string(string_id) {
                            if s.as_str() == "n" {
                                // Return vararg count
                                setivalue(ra, nextra as i64);
                                pc += 1;
                                continue;
                            }
                        }
                    }

                    // Check if R[C] is an integer (vararg index, 1-based)
                    if ttisinteger(rc) {
                        let index = ivalue(rc);

                        // Check if index is valid (1 <= index <= nextraargs)
                        if nextra > 0 && index >= 1 && (index as usize) <= nextra && base >= nextra
                        {
                            // Get value from varargs
                            // varargs are stored before base
                            let vararg_start = base - nextra;
                            let src = stack_ptr.add(vararg_start + (index as usize) - 1);
                            *ra = *src;
                        } else {
                            // Out of bounds or no varargs: return nil
                            setnilvalue(ra);
                        }
                    } else {
                        // Not integer or "n": return nil
                        setnilvalue(ra);
                    }
                }
            }

            OpCode::ErrNNil => {
                // Raise error if R[A] is not nil (global already defined)
                // Based on lvm.c:1949 and ldebug.c:817 luaG_errnnil
                // This is used by the compiler to detect duplicate global definitions
                let a = instr.get_a() as usize;
                let bx = instr.get_bx() as usize;

                unsafe {
                    let ra = stack_ptr.add(base + a);

                    // If value is not nil, it means the global is already defined
                    if !(*ra).is_nil() {
                        // Get global name from constants if bx > 0
                        let global_name = if bx > 0 && bx - 1 < constants.len() {
                            if let Some(string_id) = constants[bx - 1].as_string_id() {
                                lua_state
                                    .vm_mut()
                                    .object_pool
                                    .get_string(string_id)
                                    .map(|s| s.as_str().to_string())
                                    .unwrap_or_else(|| "?".to_string())
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
            }

            OpCode::VarargPrep => {
                // Adjust varargs (prepare vararg function)
                // Based on lvm.c:1955 and ltm.c:272 luaT_adjustvarargs
                let c = instr.get_c() as usize; // number of fixed parameters

                // Calculate total arguments and extra arguments
                let call_info = lua_state.get_call_info(frame_idx);
                let func_pos = call_info.base;
                let stack_top = lua_state.get_top();

                // Total arguments = stack_top - func_pos - 1 (exclude function itself)
                let totalargs = if stack_top > func_pos {
                    stack_top - func_pos - 1
                } else {
                    0
                };

                let nfixparams = c; // C field contains number of fixed parameters
                let nextra = if totalargs > nfixparams {
                    totalargs - nfixparams
                } else {
                    0
                };

                // Store nextra in CallInfo for later use by VARARG/GETVARG
                let call_info = lua_state.get_call_info_mut(frame_idx);
                call_info.nextraargs = nextra as i32;

                // Adjust base to account for varargs
                // In Lua 5.5, varargs are placed BEFORE the function on the stack
                // We need to ensure proper stack layout
            }

            OpCode::ExtraArg => {
                // Extra argument for previous opcode
                // This instruction should never be executed directly
                // It's always consumed by the previous instruction (NEWTABLE, SETLIST, etc.)
                // If we reach here, it's a compiler error
                lua_state.set_frame_pc(frame_idx, pc as u32);
                return Err(lua_state.error("unexpected EXTRAARG instruction".to_string()));
            }
        }
    }
}
