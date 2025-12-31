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

use std::rc::Rc;

use crate::{
    lua_value::{LuaValue, LUA_VNUMINT, LUA_VNUMFLT, LUA_VFALSE},
    lua_vm::{LuaError, LuaResult, LuaState, OpCode},
    Chunk,
};

// ============ Submodules ============
mod call;
use call::FrameAction;

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

/// setfltvalue - 设置浮点值
#[inline(always)]
unsafe fn setfltvalue(v: *mut LuaValue, n: f64) {
    unsafe {
        (*v).value_.n = n;
        (*v).tt_ = LUA_VNUMFLT;
    }
}

/// setbfvalue - 设置false
#[inline(always)]
unsafe fn setbfvalue(v: *mut LuaValue) {
    unsafe { (*v) = LuaValue::boolean(false); }
}

/// setbtvalue - 设置true
#[inline(always)]
unsafe fn setbtvalue(v: *mut LuaValue) {
    unsafe { (*v) = LuaValue::boolean(true); }
}

/// setnilvalue - 设置nil
#[inline(always)]
unsafe fn setnilvalue(v: *mut LuaValue) {
    unsafe { *v = LuaValue::nil(); }
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

/// Main VM execution entry point
/// 
/// Executes bytecode starting from current PC in the active call frame
/// Returns when function returns or error occurs
/// 
/// Architecture: Lua-style single loop, NOT recursive calls
/// - CALL: push frame, reload chunk/upvalues, continue loop
/// - RETURN: pop frame, reload chunk/upvalues, continue loop
/// - TAILCALL: replace frame, reload chunk/upvalues, continue loop
#[allow(unused)]
pub fn lua_execute(lua_state: &mut LuaState) -> LuaResult<()> {
    // Main execution loop - continues until all frames are popped
    'vm_loop: loop {
        // Get current call frame
        let frame_idx = match lua_state.call_depth().checked_sub(1) {
            Some(idx) => idx,
            None => return Ok(()), // No more frames, VM done
        };
        
        // Load current function's chunk and upvalues
        let (chunk, upvalues_vec) = {
            let func_value = lua_state.get_frame_func(frame_idx)
                .ok_or(LuaError::RuntimeError)?;
            
            let Some(func_id) = func_value.as_function_id() else {
                return Err(lua_state.error("Current frame is not a Lua function".to_string()));
            };

            let gc_function = lua_state.vm_mut().object_pool.get_function_mut(func_id)
                .ok_or(LuaError::RuntimeError)?;

            let Some(chunk_rc) = gc_function.chunk() else {
                return Err(lua_state.error("Function has no chunk".to_string()));
            };
            
            (chunk_rc.clone(), Rc::new(gc_function.upvalues.clone()))
        };
        
        // Execute this frame until CALL/RETURN/TAILCALL
        match execute_frame(lua_state, frame_idx, chunk, upvalues_vec)? {
            FrameAction::Return => {
                // Pop frame and continue with caller (or exit if no caller)
                lua_state.pop_frame();
                // Loop continues with caller's frame
            }
            FrameAction::Call => {
                // New frame was pushed, loop continues with callee's frame
                continue 'vm_loop;
            }
            FrameAction::TailCall => {
                // Current frame was replaced, loop continues with new function
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
    upvalues_vec: Rc<Vec<crate::UpvalueId>>,
) -> LuaResult<FrameAction> {
    // SAFETY: Get raw pointers to avoid borrow checker
    // These pointers remain valid because:
    // 1. Stack won't reallocate during execution (we pre-grow it)
    // 2. CallInfo won't move (we access by index, not direct pointer)
    // 3. Chunk is Rc-cloned (won't be dropped)
    
    let stack_ptr = lua_state.stack_ptr_mut();
    let stack_len = lua_state.stack_len();
    
    // Cache values in locals (will be in CPU registers)
    let mut pc = lua_state.get_frame_pc(frame_idx) as usize;
    let base = lua_state.get_frame_base(frame_idx);
    
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
            // ============================================================
            // MOVE - Most common operation
            // ============================================================
            
            OpCode::Move => {
                // R[A] := R[B]
                // setobjs2s(L, ra, RB(i))
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                
                unsafe {
                    let ra = stack_ptr.add(base + a);
                    let rb = stack_ptr.add(base + b);
                    *ra = *rb;  // Direct copy (setobjs2s)
                }
            }
            
            // ============================================================
            // LOAD OPERATIONS
            // ============================================================
            
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
                    *ra = *rb;  // setobj2s
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
                pc += 1;  // Consume EXTRAARG
                
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
                    *ra = *rb;  // setobj2s
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
                pc += 1;  // Skip next instruction
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
                        if b == 0 { break; }
                        b -= 1;
                        ra = ra.add(1);
                    }
                }
            }
            
            // ============================================================
            // ARITHMETIC - Lua 5.5 Style (pc++ on success)
            // ============================================================
            
            OpCode::Add => {
                // op_arith(L, l_addi, luai_numadd)
                // R[A] := R[B] + R[C]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;
                
                unsafe {
                    let idx_b = base + b;
                    let idx_c = base + c;
                    let idx_a = base + a;
                    
                    if idx_b >= stack_len || idx_c >= stack_len || idx_a >= stack_len {
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(lua_state.error("ADD: register out of bounds".to_string()));
                    }
                    
                    let v1 = stack_ptr.add(idx_b);
                    let v2 = stack_ptr.add(idx_c);
                    let ra = stack_ptr.add(idx_a);
                    
                    // Fast path: both integers
                    if ttisinteger(v1) && ttisinteger(v2) {
                        let i1 = ivalue(v1);
                        let i2 = ivalue(v2);
                        pc += 1;  // Skip metamethod on success
                        setivalue(ra, i1.wrapping_add(i2));
                    }
                    // Slow path: try float conversion
                    else {
                        let mut n1 = 0.0;
                        let mut n2 = 0.0;
                        if tonumberns(v1, &mut n1) && tonumberns(v2, &mut n2) {
                            pc += 1;  // Skip metamethod on success
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
                    let idx_b = base + b;
                    let idx_a = base + a;
                    
                    if idx_b >= stack_len || idx_a >= stack_len {
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(lua_state.error("ADDI: register out of bounds".to_string()));
                    }
                    
                    let v1 = stack_ptr.add(idx_b);
                    let ra = stack_ptr.add(idx_a);
                    
                    // Fast path: integer
                    if ttisinteger(v1) {
                        let iv1 = ivalue(v1);
                        pc += 1;  // Skip metamethod on success
                        setivalue(ra, iv1.wrapping_add(sc as i64));
                    }
                    // Slow path: float
                    else if ttisfloat(v1) {
                        let nb = fltvalue(v1);
                        let fimm = sc as f64;
                        pc += 1;  // Skip metamethod on success
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
            
            // ============================================================
            // BITWISE OPERATIONS
            // ============================================================
            
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
            
            // ============================================================
            // CONTROL FLOW
            // ============================================================
            
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
            
            // ============================================================
            // RETURN OPERATIONS
            // ============================================================
            
            OpCode::Return => {
                // return R[A], ..., R[A+B-2]
                let _a = instr.get_a() as usize;
                let _b = instr.get_b() as usize;
                let _c = instr.get_c() as usize;
                let _k = instr.get_k();
                
                // TODO: Handle variable returns, close upvalues
                // Update PC before returning
                lua_state.set_frame_pc(frame_idx, pc as u32);
                return Ok(FrameAction::Return);
            }
            
            OpCode::Return0 => {
                // return (no values)
                lua_state.set_frame_pc(frame_idx, pc as u32);
                return Ok(FrameAction::Return);
            }
            
            OpCode::Return1 => {
                // return R[A]
                let _a = instr.get_a() as usize;
                // TODO: copy return value to caller
                lua_state.set_frame_pc(frame_idx, pc as u32);
                return Ok(FrameAction::Return);
            }
            
            // ============================================================
            // UPVALUE OPERATIONS
            // ============================================================
            
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
                let upvalue = lua_state.vm_mut().object_pool.get_upvalue(upval_id)
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
                let upvalue = lua_state.vm_mut().object_pool.get_upvalue_mut(upval_id)
                    .ok_or(LuaError::RuntimeError)?;
                
                if upvalue.is_open() {
                    // Open: write to stack
                    let stack_idx = upvalue.get_stack_index().unwrap();
                    lua_state.stack_set(stack_idx, value);
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
                
                // TODO: Implement upvalue closing
                // This requires tracking open upvalues in LuaState
                // For now, this is a no-op
                // Proper implementation needs:
                // 1. List of open upvalues in LuaState
                // 2. Close all upvalues with stack_index >= close_from
                // 3. Copy stack value to upvalue storage
                // 4. Mark upvalue as closed
                
                let _ = close_from; // Suppress unused warning
            }
            
            OpCode::Tbc => {
                // Mark variable as to-be-closed
                let _a = instr.get_a() as usize;
                
                // TODO: Implement to-be-closed variables
                // This is for the <close> attribute in Lua 5.4+
                // Needs tracking in stack/upvalue system
                // For now, this is a no-op
            }
            
            // ============================================================
            // TABLE OPERATIONS
            // ============================================================
            
            OpCode::NewTable => {
                // R[A] := {} (new table)
                let a = instr.get_a() as usize;
                let _vb = instr.get_b() as usize; // log2(hash size) + 1
                let _vc = instr.get_c() as usize; // array size
                
                // Create new table
                let value = lua_state.create_table(0, 0);
                
                unsafe {
                    let ra = stack_ptr.add(base + a);
                    *ra = value;
                }
                
                // TODO: Pre-allocate table size based on vB and vC
                // TODO: Check for EXTRAARG instruction for larger sizes
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
                            let table = lua_state.vm_mut().object_pool.get_table(table_id)
                                .ok_or(LuaError::RuntimeError)?;
                            
                            let result = table.get_int(key).unwrap_or(LuaValue::nil());
                            let ra = stack_ptr.add(base + a);
                            *ra = result;
                        } else {
                            // General case: use key as LuaValue
                            let key = *rc;
                            let table = lua_state.vm_mut().object_pool.get_table(table_id)
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
                        let table = lua_state.vm_mut().object_pool.get_table(table_id)
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
                        let table = lua_state.vm_mut().object_pool.get_table(table_id)
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
                            let table = lua_state.vm_mut().object_pool.get_table_mut(table_id)
                                .ok_or(LuaError::RuntimeError)?;
                            table.set_int(int_key, value);
                        } else {
                            let table = lua_state.vm_mut().object_pool.get_table_mut(table_id)
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
                        let table = lua_state.vm_mut().object_pool.get_table_mut(table_id)
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
                        let table = lua_state.vm_mut().object_pool.get_table_mut(table_id)
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
                        let table = lua_state.vm_mut().object_pool.get_table(table_id)
                            .ok_or(LuaError::RuntimeError)?;
                        
                        let result = table.raw_get(key).unwrap_or(LuaValue::nil());
                        *ra = result;
                    } else {
                        // Not a table - should trigger metamethod
                        setnilvalue(ra);
                    }
                }
            }
            
            // ============================================================
            // FUNCTION CALL OPERATIONS
            // ============================================================
            
            OpCode::Call => {
                // R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                let c = instr.get_c() as usize;
                
                // Save PC before call
                lua_state.set_frame_pc(frame_idx, pc as u32);
                
                // Delegate to call handler - returns FrameAction
                return call::handle_call(lua_state, frame_idx, base, a, b, c);
            }
            
            OpCode::TailCall => {
                // Tail call optimization: return R[A](R[A+1], ... ,R[A+B-1])
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                
                // Save PC before call
                lua_state.set_frame_pc(frame_idx, pc as u32);
                
                // Delegate to tailcall handler (returns FrameAction)
                return call::handle_tailcall(lua_state, frame_idx, base, a, b);
            }
            
            // ============================================================
            // LOGICAL OPERATIONS
            // ============================================================
            
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
            
            // ============================================================
            // FOR LOOP OPERATIONS
            // ============================================================
            
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
                        let count = ivalue(ra) as u64;  // unsigned count
                        if count > 0 {
                            // More iterations
                            let step = ivalue(ra.add(1));
                            let mut idx = ivalue(ra.add(2));
                            
                            // Update counter (decrement)
                            setivalue(ra, (count - 1) as i64);
                            
                            // Update control variable: idx += step
                            idx = idx.wrapping_add(step);
                            setivalue(ra.add(2), idx);
                            
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
                        let mut idx = fltvalue(ra.add(2));
                        
                        // idx += step
                        idx += step;
                        
                        // Check if should continue
                        let should_continue = if step > 0.0 {
                            idx <= limit
                        } else {
                            idx >= limit
                        };
                        
                        if should_continue {
                            // Update control variable
                            setfltvalue(ra.add(2), idx);
                            
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
                        let should_skip = if step > 0 {
                            init > limit
                        } else {
                            init < limit
                        };
                        
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
                            return Err(lua_state.error("'for' initial value must be a number".to_string()));
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
                    let temp = *ra.add(3);  // closing
                    *ra.add(3) = *ra.add(2);  // control -> closing position
                    *ra.add(2) = temp;  // closing -> control position
                    
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
                let _c = instr.get_c() as usize;
                
                unsafe {
                    let ra = stack_ptr.add(base + a);
                    
                    // Setup call stack:
                    // ra+3: function (copy from ra)
                    // ra+4: arg1 (copy from ra+1, state)
                    // ra+5: arg2 (copy from ra+2, closing/control)
                    *ra.add(5) = *ra.add(3);  // copy control variable
                    *ra.add(4) = *ra.add(1);  // copy state
                    *ra.add(3) = *ra;         // copy iterator function
                    
                    // TODO: Call function at ra+3
                    // For now, just fall through to TFORLOOP
                    // The next instruction should be TFORLOOP
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
            
            // ============================================================
            // UNIMPLEMENTED OPCODES
            // ============================================================
            
            _ => {
                lua_state.set_frame_pc(frame_idx, pc as u32);
                return Err(lua_state.error(format!(
                    "Unimplemented opcode: {:?} at pc={}",
                    instr.get_opcode(),
                    pc - 1
                )));
            }
        }
    }
}
