use crate::LuaResult;
use crate::lua_value::Chunk;
use crate::lua_vm::LUA_HOOKCOUNT;
use crate::lua_vm::LUA_HOOKLINE;
use crate::lua_vm::LUA_MASKLINE;
use crate::lua_vm::LuaState;
use crate::lua_vm::LuaVM;
use crate::lua_vm::call_info::call_status::CIST_TAIL;
use crate::lua_vm::{LUA_HOOKCALL, LUA_HOOKTAILCALL, LUA_MASKCALL, LUA_MASKCOUNT};

// ===== Cold hook helpers — kept out of the hot execute loop =====
// All are #[cold] #[inline(never)] so the compiler won't bloat the main loop.

/// Fire call hook at function entry (normal call or tail call).
/// Called when pc == 0 and LUA_MASKCALL is set.
/// Also initialises hook_count when LUA_MASKCOUNT is set.
#[cold]
#[inline(never)]
pub fn hook_on_call(
    lua_state: &mut LuaState,
    vm_ptr: *mut LuaVM,
    hook_mask: u8,
    call_status: u32,
) -> LuaResult<()> {
    if hook_mask & LUA_MASKCALL != 0 {
        let event = if call_status & CIST_TAIL != 0 {
            LUA_HOOKTAILCALL
        } else {
            LUA_HOOKCALL
        };
        lua_state.run_hook(event, -1)?;
    }
    // Initialise per-thread hook_count from global base_hook_count
    if hook_mask & LUA_MASKCOUNT != 0 {
        lua_state.hook_count = unsafe { (*vm_ptr).base_hook_count };
    }
    Ok(())
}

/// Fire return hook before leaving the current frame.
#[cold]
#[inline(never)]
pub fn hook_on_return(lua_state: &mut LuaState, frame_idx: usize, pc: u32) -> LuaResult<()> {
    lua_state.set_frame_pc(frame_idx, pc);
    lua_state.run_hook(crate::lua_vm::LUA_HOOKRET, -1)
}

/// Check count / line hooks inside the main instruction loop.
/// Returns the (possibly updated) hook_mask.
#[cold]
#[inline(never)]
pub fn hook_check_instruction(
    lua_state: &mut LuaState,
    vm_ptr: *mut LuaVM,
    hook_mask: u8,
    pc: usize,
    chunk: &Chunk,
    last_line: &mut u32,
    frame_idx: usize,
) -> LuaResult<u8> {
    // Count hook: decrement per-instruction counter
    if hook_mask & LUA_MASKCOUNT != 0 {
        lua_state.hook_count -= 1;
        if lua_state.hook_count == 0 {
            lua_state.hook_count = unsafe { (*vm_ptr).base_hook_count };
            lua_state.set_frame_pc(frame_idx, pc as u32);
            lua_state.run_hook(LUA_HOOKCOUNT, -1)?;
        }
    }
    // Line hook: fire when source line changes
    if hook_mask & LUA_MASKLINE != 0 {
        let line_info = &chunk.line_info;
        if pc > 0 && (pc - 1) < line_info.len() {
            let current_line = line_info[pc - 1];
            if current_line != *last_line {
                *last_line = current_line;
                lua_state.set_frame_pc(frame_idx, pc as u32);
                lua_state.run_hook(LUA_HOOKLINE, current_line as i32)?;
            }
        }
    }
    // Re-read hook_mask (hook callback may have changed it via debug.sethook)
    Ok(unsafe { (*vm_ptr).hook_mask })
}
