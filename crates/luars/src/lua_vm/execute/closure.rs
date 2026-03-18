use crate::{
    Chunk, UpvaluePtr,
    lua_value::{UpvalueDesc, UpvalueStore},
    lua_vm::{LuaError, LuaResult, LuaState},
};

/// Handle OP_CLOSURE instruction
/// Create a closure from prototype Bx and store in R[A]
///
/// Based on lvm.c:1929-1934 and pushclosure (lvm.c:834-849)
pub fn push_closure(
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

    // Build UpvalueStore — avoid heap allocation for 0-1 upvalues
    let upvalue_store = match num_upvalues {
        0 => UpvalueStore::Empty,
        1 => {
            let uv = resolve_upvalue(lua_state, base, &upvalue_descs[0], parent_upvalues)?;
            UpvalueStore::One(uv)
        }
        _ => {
            let mut upvalue_vec = Vec::with_capacity(num_upvalues);
            for desc in upvalue_descs {
                upvalue_vec.push(resolve_upvalue(lua_state, base, desc, parent_upvalues)?);
            }
            UpvalueStore::Many(upvalue_vec.into_boxed_slice())
        }
    };

    // Create the function with the proto and upvalues
    let closure_value = lua_state.create_function(proto, upvalue_store)?;

    // Store in R[A]
    lua_state.stack_mut()[base + a] = closure_value;
    Ok(())
}

/// Resolve a single upvalue from its descriptor
#[inline]
fn resolve_upvalue(
    lua_state: &mut LuaState,
    base: usize,
    desc: &UpvalueDesc,
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
