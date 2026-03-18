use crate::CallInfo;
use crate::LuaResult;
use crate::lua_value::Chunk;
use crate::lua_vm::LUA_HOOKCOUNT;
use crate::lua_vm::LUA_HOOKLINE;
use crate::lua_vm::LUA_HOOKRET;
use crate::lua_vm::LUA_MASKLINE;
use crate::lua_vm::LuaState;
use crate::lua_vm::call_info::call_status::CIST_TAIL;
use crate::lua_vm::{LUA_HOOKCALL, LUA_HOOKTAILCALL, LUA_MASKCALL, LUA_MASKCOUNT};

/// Fire call hook at function entry (normal call or tail call).
/// Called when pc == 0 and LUA_MASKCALL is set.
/// Also initialises hook_count when LUA_MASKCOUNT is set.
#[cold]
#[inline(never)]
pub fn hook_on_call(
    lua_state: &mut LuaState,
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
    ci: &mut CallInfo,
    pc: usize,
    nres: i32,
) -> LuaResult<()> {
    ci.save_pc(pc);
    let base = ci.base;
    let first_res = if nres > 0 {
        lua_state.get_top() - nres as usize
    } else {
        lua_state.get_top()
    };
    let ftransfer = (first_res - base + 1) as i32;
    lua_state.run_hook(LUA_HOOKRET, -1, ftransfer, nres)
}

/// Check count / line hooks inside the main instruction loop.
/// Returns true if trap should remain active.
///
/// Unlike C Lua's traceexec which uses npci = pcRel(++pc) (one-ahead),
/// we use npci = pc - 1 (current instruction). This avoids spurious
/// line events for skipped branches (e.g., JMP past else reporting
/// the else branch's line). The backward jump condition uses strict
/// less-than (npci < oldpc) to avoid firing on same-instruction
/// comparisons after function returns.
#[cold]
#[inline(never)]
pub fn hook_check_instruction(
    lua_state: &mut LuaState,
    pc: usize,
    chunk: &Chunk,
    ci: &mut CallInfo,
) -> LuaResult<bool> {
    let hook_mask = lua_state.hook_mask;
    if hook_mask == 0 {
        return Ok(false);
    }
    if !lua_state.allow_hook {
        return Ok(true);
    }
    // Count hook
    if hook_mask & LUA_MASKCOUNT != 0 {
        lua_state.hook_count -= 1;
        if lua_state.hook_count == 0 {
            lua_state.hook_count = lua_state.base_hook_count;
            ci.save_pc(pc);
            lua_state.run_hook(LUA_HOOKCOUNT, -1, 0, 0)?;
        }
    }
    // Line hook: fire when source line changes (changedline logic)
    if hook_mask & LUA_MASKLINE != 0 {
        let line_info = &chunk.line_info;
        if !line_info.is_empty() {
            // npci = current instruction's 0-based index
            let npci = pc.saturating_sub(1);
            let oldpc = lua_state.oldpc as usize;

            // Fire conditions:
            // 1. oldpc out of range (u32::MAX sentinel) → first instruction → always fire
            // 2. npci < oldpc (strict) → backward jump (loop) → fire
            // 3. changedline(oldpc, npci) → source line changed → fire
            //
            // We use strict < (not <=) because after function returns,
            // rethook sets oldpc = ci.pc - 1 = npci. With <=, this would
            // trigger the backward-jump condition spuriously.
            let should_fire = if oldpc >= line_info.len() {
                // Function entry sentinel (u32::MAX) → always fire
                true
            } else {
                npci < oldpc || {
                    let old_line = line_info[oldpc];
                    let new_line = if npci < line_info.len() {
                        line_info[npci]
                    } else {
                        // Past last instruction (RETURN) → use last line
                        line_info[line_info.len() - 1]
                    };
                    old_line != new_line
                }
            };

            if should_fire {
                let new_line = if npci < line_info.len() {
                    line_info[npci]
                } else {
                    line_info[line_info.len() - 1]
                };
                ci.save_pc(pc);
                lua_state.run_hook(LUA_HOOKLINE, new_line as i32, 0, 0)?;
            }
            // Store current instruction index (like C Lua's L->oldpc = npci)
            lua_state.oldpc = npci as u32;
        } else {
            // Stripped code (no line info): fire on backward jumps
            let npci = pc.saturating_sub(1);
            let oldpc = lua_state.oldpc as usize;
            if oldpc == usize::MAX || npci < oldpc {
                ci.save_pc(pc);
                lua_state.run_hook(LUA_HOOKLINE, -1, 0, 0)?;
            }
            lua_state.oldpc = npci as u32;
        }
    }
    Ok(lua_state.hook_mask != 0)
}
