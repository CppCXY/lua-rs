/*----------------------------------------------------------------------
  Lua 5.5 VM Execution Engine - Slice-Based High-Performance Implementation

  Design Philosophy:
  1. **Slice-Based**: Code and constants accessed via `&[T]` slices with
     `noalias` guarantees — LLVM keeps slice base pointers in registers
     across function calls (raw pointers must be reloaded after `&mut` calls)
  2. **Minimal Indirection**: Use get_unchecked for stack access (no bounds checks)
  3. **No Allocation in Loop**: All errors via lua_state.error(), no String construction
  4. **CPU Register Optimization**: code, constants, pc, base, trap in CPU registers
  5. **Unsafe but Sound**: Use raw pointers with invariant guarantees for stack

  Key Invariants (maintained by caller):
  - Stack pointer valid throughout execution (no reallocation)
  - CallInfo valid and matches current frame
  - Chunk lifetime extends through execution
  - base + register < stack.len() (validated at call time)

  This leverages Rust's type system for LLVM optimization opportunities
----------------------------------------------------------------------*/

pub mod call;
mod closure_handler;
mod cold;
mod concat;
mod execute_loop;
pub(crate) mod helper;
mod hook;
pub(crate) mod metamethod;
mod return_handler;

// Extracted opcode modules to reduce main loop size
mod closure_vararg_ops;
mod comparison_ops;
mod noinline;
mod table_ops;

use crate::lua_vm::{LuaResult, LuaState};

pub use helper::{get_metamethod_event, get_metatable};
pub use metamethod::TmKind;
pub use metamethod::call_tm_res;

/// Execute until call depth reaches target_depth
/// Used for protected calls (pcall) to execute only the called function
/// without affecting caller frames
///
/// ARCHITECTURE: Single-loop execution like Lua C's luaV_execute
/// - Uses labeled loops instead of goto for context switching
/// - Function calls/returns just update pointers and continue
/// - Zero Rust function call overhead
///
/// NOTE: n_ccalls tracking is NOT done here (unlike the wrapper approach).
/// Instead, each recursive CALL SITE (metamethods, pcall, resume, __close)
/// increments/decrements n_ccalls around its call to lua_execute, mirroring
/// Lua 5.5's luaD_call pattern.
pub fn lua_execute(lua_state: &mut LuaState, target_depth: usize) -> LuaResult<()> {
    execute_loop::lua_execute_new(lua_state, target_depth)
}
