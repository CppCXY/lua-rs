use crate::LuaResult;
use crate::lua_value::Chunk;
use crate::lua_vm::LUA_HOOKCOUNT;
use crate::lua_vm::LUA_HOOKLINE;
use crate::lua_vm::LUA_MASKLINE;
use crate::lua_vm::LuaState;
use crate::lua_vm::LuaVM;
use crate::lua_vm::call_info::call_status::CIST_TAIL;
use crate::lua_vm::{LUA_HOOKCALL, LUA_HOOKTAILCALL, LUA_MASKCALL, LUA_MASKCOUNT};

/// Fire call hook at function entry (normal call or tail call).
/// Called when pc == 0 and LUA_MASKCALL is set.
/// Also initialises hook_count when LUA_MASKCOUNT is set.
#[cold]
#[inline(never)]
pub fn hook_on_call(
    lua_state: &mut LuaState,
    _vm_ptr: *mut LuaVM,
    hook_mask: u8,
    call_status: u32,
    chunk: &Chunk,
) -> LuaResult<()> {
    if hook_mask & LUA_MASKCALL != 0 {
        let event = if call_status & CIST_TAIL != 0 {
            LUA_HOOKTAILCALL
        } else {
            LUA_HOOKCALL
        };
        // ftransfer=1 (first param), ntransfer=numparams (like C Lua's luaD_hookcall)
        lua_state.run_hook(event, -1, 1, chunk.param_count as i32)?;
    }
    // Initialise per-thread hook_count from per-thread base_hook_count
    if hook_mask & LUA_MASKCOUNT != 0 {
        lua_state.hook_count = lua_state.base_hook_count;
    }
    Ok(())
}

/// Fire return hook before leaving the current frame.
/// nres: number of return values being returned.
#[cold]
#[inline(never)]
pub fn hook_on_return(
    lua_state: &mut LuaState,
    frame_idx: usize,
    pc: u32,
    nres: i32,
) -> LuaResult<()> {
    lua_state.set_frame_pc(frame_idx, pc);
    let ci = lua_state.get_call_info(frame_idx);
    let base = ci.base;
    let first_res = if nres > 0 {
        lua_state.get_top() - nres as usize
    } else {
        lua_state.get_top()
    };
    let ftransfer = (first_res - base + 1) as i32;
    lua_state.run_hook(crate::lua_vm::LUA_HOOKRET, -1, ftransfer, nres)
}

/// Check count / line hooks inside the main instruction loop.
/// Returns the (possibly updated) hook_mask.
#[cold]
#[inline(never)]
pub fn hook_check_instruction(
    lua_state: &mut LuaState,
    _vm_ptr: *mut LuaVM,
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
            lua_state.hook_count = lua_state.base_hook_count;
            lua_state.set_frame_pc(frame_idx, pc as u32);
            lua_state.run_hook(LUA_HOOKCOUNT, -1, 0, 0)?;
        }
    }
    // Line hook: fire when source line changes
    if hook_mask & LUA_MASKLINE != 0 {
        let line_info = &chunk.line_info;
        if !line_info.is_empty() {
            // Normal code with line info: fire when line changes
            if pc > 0 && (pc - 1) < line_info.len() {
                let current_line = line_info[pc - 1];
                if current_line != *last_line {
                    *last_line = current_line;
                    lua_state.set_frame_pc(frame_idx, pc as u32);
                    lua_state.run_hook(LUA_HOOKLINE, current_line as i32, 0, 0)?;
                }
            }
        } else {
            // Stripped code (no line info): fire at first instruction (npc==0)
            // and on backward jumps (pc <= oldpc), with line=-1.
            // Reuse last_line as oldpc tracker (0 = not yet seen).
            let old_pc = *last_line as usize;
            let npc = pc.saturating_sub(1); // 0-indexed like C Lua's pcRel
            if npc == 0 || pc <= old_pc {
                lua_state.set_frame_pc(frame_idx, pc as u32);
                lua_state.run_hook(LUA_HOOKLINE, -1, 0, 0)?;
            }
            *last_line = pc as u32;
        }
    }
    // Re-read hook_mask (hook callback may have changed it via debug.sethook)
    Ok(lua_state.hook_mask)
}
