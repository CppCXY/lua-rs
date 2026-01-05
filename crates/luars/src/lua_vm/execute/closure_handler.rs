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
    Chunk, gc,
    lua_value::LuaValue,
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
    parent_upvalues: &[gc::UpvalueId],
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
        let upval_id = if desc.is_local {
            // Capture local variable from current stack frame
            // desc.index is relative to base
            let stack_index = base + desc.index as usize;

            // Find or create open upvalue for this stack position
            find_or_create_upvalue(lua_state, stack_index)?
        } else {
            // Inherit upvalue from parent closure
            let parent_idx = desc.index as usize;
            if parent_idx >= parent_upvalues.len() {
                return Err(LuaError::RuntimeError);
            }
            parent_upvalues[parent_idx]
        };

        new_upvalues.push(upval_id);
    }

    // Create the function with the proto and upvalues
    let func_id = lua_state
        .vm_mut()
        .object_pool
        .create_function(proto, new_upvalues);

    // Store in R[A]
    let closure_value = LuaValue::function(func_id);
    lua_state.stack_mut()[base + a] = closure_value;

    Ok(())
}

/// Find an existing open upvalue for stack_index, or create a new one
/// Based on Lua's luaF_findupval (lfunc.c)
fn find_or_create_upvalue(
    lua_state: &mut LuaState,
    stack_index: usize,
) -> LuaResult<crate::gc::UpvalueId> {
    // Use LuaState's find_or_create_upvalue which uses the open_upvalues list
    // This is O(n) where n is the number of open upvalues for THIS thread,
    // not all upvalues in the system
    lua_state.find_or_create_upvalue(stack_index)
}

/// Close all open upvalues at or above the given level
/// This is used by CLOSE instruction and return handlers
/// Based on Lua's luaF_close (lfunc.c)
pub fn close_upvalues_at_level(lua_state: &mut LuaState, level: usize) -> LuaResult<()> {
    // Collect upvalues to close (can't borrow mutably while iterating)
    let upvalues_to_close: Vec<_> = lua_state
        .vm_mut()
        .object_pool
        .iter_upvalues()
        .filter_map(|(id, upval)| {
            if let Some(stack_idx) = upval.get_stack_index() {
                if stack_idx >= level {
                    Some((id, stack_idx))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Close each upvalue
    for (upval_id, stack_idx) in upvalues_to_close {
        // Get the value from the stack
        let value = lua_state.stack_get(stack_idx).unwrap_or(LuaValue::nil());

        // Close the upvalue (move value from stack to upvalue storage)
        if let Some(upval) = lua_state.vm_mut().object_pool.get_upvalue_mut(upval_id) {
            unsafe {
                upval.close_with_value(value);
            }
        }
    }

    Ok(())
}
