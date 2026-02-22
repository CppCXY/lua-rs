/*----------------------------------------------------------------------
  Closure Creation Handler - Lua 5.5 Style

  Based on Lua 5.5.0 lvm.c:834-849, 1929-1934

  Implements:
  - OP_CLOSURE: Create closure from prototype

  Key operations:
  1. Get child Proto from current function's child_protos[Bx]
  2. Create new LuaFunction/Closure
  3. Set up upvalues:
     - instack=true: capture parent's local variable (open upvalue)
     - instack=false: inherit parent's upvalue
  4. Store closure in R[A]
----------------------------------------------------------------------*/

use crate::{
    Chunk, Instruction, UpvaluePtr,
    lua_value::UpvalueStore,
    lua_vm::{LuaError, LuaResult, LuaState},
};

#[cold]
#[inline(never)]
pub fn handle_closure(
    lua_state: &mut LuaState,
    instr: Instruction,
    base: usize,
    frame_idx: usize,
    chunk: &Chunk,
    upvalue_ptrs: &[UpvaluePtr],
    pc: usize,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let bx = instr.get_bx() as usize;

    // Create closure from child prototype
    handle_closure_internal(lua_state, base, a, bx, chunk, upvalue_ptrs)?;

    // Save PC for GC, then set top to ra+1 for GC scan scope (like C Lua's checkGC).
    // Use set_top_raw: stack is already grown by push_lua_frame, so no bounds check needed.
    let new_top = base + a + 1;
    lua_state.set_frame_pc(frame_idx, pc as u32);
    lua_state.set_top_raw(new_top);
    lua_state.check_gc()?;

    let frame_top = lua_state.get_call_info(frame_idx).top;
    lua_state.set_top_raw(frame_top);
    Ok(())
}

/// Handle OP_CLOSURE instruction
/// Create a closure from prototype Bx and store in R[A]
///
/// Based on lvm.c:1929-1934 and pushclosure (lvm.c:834-849)
#[inline]
fn handle_closure_internal(
    lua_state: &mut LuaState,
    base: usize,
    a: usize,
    bx: usize,
    current_chunk: &Chunk,
    parent_upvalues: &[UpvaluePtr],
) -> LuaResult<()> {
    // Get child prototype
    if bx >= current_chunk.child_protos.len() {
        return Err(LuaError::RuntimeError);
    }
    let proto = current_chunk.child_protos[bx].clone();

    // Get upvalue descriptors
    let upvalue_descs = &proto.upvalue_descs;
    let num_upvalues = upvalue_descs.len();

    // Build UpvalueStore directly — avoid heap allocation for 0-2 upvalues
    let upvalue_store = match num_upvalues {
        0 => UpvalueStore::Empty,
        1 => {
            let desc = &upvalue_descs[0];
            let ptr = resolve_upvalue(lua_state, base, desc, parent_upvalues)?;
            UpvalueStore::One(ptr)
        }
        2 => {
            let p0 = resolve_upvalue(lua_state, base, &upvalue_descs[0], parent_upvalues)?;
            let p1 = resolve_upvalue(lua_state, base, &upvalue_descs[1], parent_upvalues)?;
            UpvalueStore::Two([p0, p1])
        }
        _ => {
            let mut v = Vec::with_capacity(num_upvalues);
            for desc in upvalue_descs {
                v.push(resolve_upvalue(lua_state, base, desc, parent_upvalues)?);
            }
            UpvalueStore::Many(v.into_boxed_slice())
        }
    };

    // Create the function with the proto and upvalues (no intermediate Vec)
    let closure_value = lua_state.create_function(proto, upvalue_store)?;

    // Store in R[A]
    lua_state.stack_mut()[base + a] = closure_value;
    Ok(())
}

/// Resolve a single upvalue from its descriptor
#[inline(always)]
fn resolve_upvalue(
    lua_state: &mut LuaState,
    base: usize,
    desc: &crate::lua_value::UpvalueDesc,
    parent_upvalues: &[UpvaluePtr],
) -> LuaResult<UpvaluePtr> {
    if desc.is_local {
        let stack_index = base + desc.index as usize;
        lua_state.find_or_create_upvalue(stack_index)
    } else {
        let parent_idx = desc.index as usize;
        if parent_idx >= parent_upvalues.len() {
            return Err(LuaError::RuntimeError);
        }
        Ok(parent_upvalues[parent_idx])
    }
}
