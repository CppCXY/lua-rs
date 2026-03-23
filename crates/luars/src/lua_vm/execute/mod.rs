pub mod call;
mod closure;
mod concat;
mod execute_loop;
pub(crate) mod helper;
mod hook;
#[cfg(feature = "jit")]
mod jit_execute_loop;
pub(crate) mod metamethod;
mod number;
mod vararg;

pub use helper::{get_metamethod_event, get_metatable};
pub use metamethod::TmKind;
pub use metamethod::call_tm_res;
pub use metamethod::call_tm_res1;

use crate::compiler::LuaLanguageLevel;
use crate::lua_vm::{LuaResult, LuaState};

pub fn lua_execute(lua_state: &mut LuaState, target_depth: usize) -> LuaResult<()> {
    #[cfg(feature = "jit")]
    {
        let vm = unsafe { &*lua_state.vm_ptr() };
        if vm
            .jit_runtime()
            .should_use_jit_execute_loop(vm.language_level())
        {
            return jit_execute_loop::lua_execute(lua_state, target_depth);
        }
    }

    let _ = LuaLanguageLevel::Lua55;
    execute_loop::lua_execute(lua_state, target_depth)
}
