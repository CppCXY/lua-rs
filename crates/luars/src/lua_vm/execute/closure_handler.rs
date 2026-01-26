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
    Chunk, UpvaluePtr,
    lua_vm::{LuaError, LuaResult, LuaState},
};
use std::rc::Rc;

/// Handle OP_CLOSURE instruction
/// Create a closure from prototype Bx and store in R[A]
///
/// Based on lvm.c:1929-1934 and pushclosure (lvm.c:834-849)
pub fn handle_closure(
    lua_state: &mut LuaState,
    base: usize,
    a: usize,
    bx: usize,
    current_chunk: &Rc<Chunk>,
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

    // Create upvalue array for the new closure
    let mut new_upvalues = Vec::with_capacity(num_upvalues);

    // Fill in upvalues according to descriptors
    for desc in upvalue_descs {
        let upval_ptr = if desc.is_local {
            // Capture local variable from current stack frame
            // desc.index is relative to base
            let stack_index = base + desc.index as usize;

            // Find or create open upvalue for this stack position
            lua_state.find_or_create_upvalue(stack_index)?
        } else {
            // Inherit upvalue from parent closure
            let parent_idx = desc.index as usize;
            if parent_idx >= parent_upvalues.len() {
                return Err(LuaError::RuntimeError);
            }
            parent_upvalues[parent_idx]
        };

        new_upvalues.push(upval_ptr);
    }

    // Create the function with the proto and upvalues
    let closure_value = lua_state.vm_mut().create_function(proto, new_upvalues);

    // Store in R[A]
    lua_state.stack_mut()[base + a] = closure_value;
    Ok(())
}
