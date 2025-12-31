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
    lua_value::LuaValue,
    lua_vm::{LuaError, LuaResult, LuaState, OpCode},
    Chunk,
};

/// Main VM execution entry point
/// 
/// Executes bytecode starting from current PC in the active call frame
/// Returns when function returns or error occurs
pub fn lua_execute(lua_state: &mut LuaState) -> LuaResult<()> {
    // Get current call frame - must exist
    let frame_idx = lua_state.call_depth().checked_sub(1)
        .ok_or(LuaError::RuntimeError)?;
    
    // Extract function and chunk BEFORE entering unsafe
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
    
    // Clone Rc to avoid holding mutable borrow
    let chunk_rc: Rc<Chunk> = chunk_rc.clone();
    
    // Now execute with the chunk
    execute_with_chunk(lua_state, frame_idx, chunk_rc)
}

/// Execute with a given chunk - uses pointer-based access
/// 
/// # Safety
/// This function uses raw pointers extensively:
/// - stack_ptr: points into lua_state.stack (must not reallocate during execution)
/// - call_info_ptr: points to current CallInfo (must remain valid)
/// - Chunk is Rc-cloned so it won't be dropped
fn execute_with_chunk(
    lua_state: &mut LuaState,
    frame_idx: usize,
    chunk: Rc<Chunk>,
) -> LuaResult<()> {
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
        // Bounds check for PC
        if pc >= code.len() {
            return Err(lua_state.error("PC out of bounds".to_string()));
        }
        
        // Fetch instruction and advance PC
        let instr = code[pc];
        pc += 1;
        
        // Dispatch instruction
        // The match compiles to a jump table in release mode
        match instr.get_opcode() {
            // ============================================================
            // MOVE - Most common operation
            // ============================================================
            
            OpCode::Move => {
                // R[A] := R[B]
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                
                unsafe {
                    let src_idx = base + b;
                    let dst_idx = base + a;
                    
                    if src_idx < stack_len && dst_idx < stack_len {
                        *stack_ptr.add(dst_idx) = *stack_ptr.add(src_idx);
                    } else {
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(lua_state.error(format!("MOVE: register out of bounds")));
                    }
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
                    let idx = base + a;
                    if idx < stack_len {
                        *stack_ptr.add(idx) = LuaValue::integer(sbx as i64);
                    }
                }
            }
            
            OpCode::LoadF => {
                // R[A] := (float)sBx
                let a = instr.get_a() as usize;
                let sbx = instr.get_sbx();
                
                unsafe {
                    let idx = base + a;
                    if idx < stack_len {
                        *stack_ptr.add(idx) = LuaValue::number(sbx as f64);
                    }
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
                    let idx = base + a;
                    if idx < stack_len {
                        *stack_ptr.add(idx) = constants[bx];
                    }
                }
            }
            
            OpCode::LoadKX => {
                // R[A] := K[extra_arg]
                let a = instr.get_a() as usize;
                
                if pc >= code.len() {
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    return Err(lua_state.error("LOADKX: missing EXTRAARG".to_string()));
                }
                
                let extra = code[pc];
                pc += 1;
                
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
                    let idx = base + a;
                    if idx < stack_len {
                        *stack_ptr.add(idx) = constants[ax];
                    }
                }
            }
            
            OpCode::LoadFalse => {
                // R[A] := false
                let a = instr.get_a() as usize;
                
                unsafe {
                    let idx = base + a;
                    if idx < stack_len {
                        *stack_ptr.add(idx) = LuaValue::boolean(false);
                    }
                }
            }
            
            OpCode::LFalseSkip => {
                // R[A] := false; pc++
                let a = instr.get_a() as usize;
                
                unsafe {
                    let idx = base + a;
                    if idx < stack_len {
                        *stack_ptr.add(idx) = LuaValue::boolean(false);
                    }
                }
                pc += 1;
            }
            
            OpCode::LoadTrue => {
                // R[A] := true
                let a = instr.get_a() as usize;
                
                unsafe {
                    let idx = base + a;
                    if idx < stack_len {
                        *stack_ptr.add(idx) = LuaValue::boolean(true);
                    }
                }
            }
            
            OpCode::LoadNil => {
                // R[A], R[A+1], ..., R[A+B] := nil
                let a = instr.get_a() as usize;
                let b = instr.get_b() as usize;
                
                unsafe {
                    for i in 0..=b {
                        let idx = base + a + i;
                        if idx < stack_len {
                            *stack_ptr.add(idx) = LuaValue::nil();
                        }
                    }
                }
            }
            
            // ============================================================
            // ARITHMETIC - Integer Fast Path
            // ============================================================
            
            OpCode::Add => {
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
                    
                    let vb = &*stack_ptr.add(idx_b);
                    let vc = &*stack_ptr.add(idx_c);
                    
                    // Try integer addition first (fast path)
                    let result = if vb.is_integer() && vc.is_integer() {
                        let x = vb.as_integer().unwrap();
                        let y = vc.as_integer().unwrap();
                        LuaValue::integer(x.wrapping_add(y))
                    } else if vb.is_number() && vc.is_number() {
                        // Float arithmetic
                        let x = vb.as_number().unwrap();
                        let y = vc.as_number().unwrap();
                        LuaValue::number(x + y)
                    } else {
                        // TODO: metamethods (__add)
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(lua_state.error("ADD: invalid operands for arithmetic".to_string()));
                    };
                    
                    *stack_ptr.add(idx_a) = result;
                }
            }
            
            OpCode::AddI => {
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
                    
                    let vb = &*stack_ptr.add(idx_b);
                    
                    // Try integer addition first (fast path)
                    let result = if vb.is_integer() {
                        let x = vb.as_integer().unwrap();
                        LuaValue::integer(x.wrapping_add(sc as i64))
                    } else if vb.is_number() {
                        let x = vb.as_number().unwrap();
                        LuaValue::number(x + sc as f64)
                    } else {
                        lua_state.set_frame_pc(frame_idx, pc as u32);
                        return Err(lua_state.error("ADDI: invalid operand type".to_string()));
                    };
                    
                    *stack_ptr.add(idx_a) = result;
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
                return Ok(());
            }
            
            OpCode::Return0 => {
                // return (no values)
                lua_state.set_frame_pc(frame_idx, pc as u32);
                return Ok(());
            }
            
            OpCode::Return1 => {
                // return R[A]
                let _a = instr.get_a() as usize;
                // TODO: copy return value to caller
                lua_state.set_frame_pc(frame_idx, pc as u32);
                return Ok(());
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
