use std::rc::Rc;

use crate::{LuaValue, lua_value::LuaUpvalue, lua_vm::LuaCallFrame};

/// Lua Thread (coroutine)
pub struct LuaThread {
    pub status: CoroutineStatus,
    pub frames: Vec<LuaCallFrame>,
    pub register_stack: Vec<LuaValue>,
    pub return_values: Vec<LuaValue>,
    pub open_upvalues: Vec<Rc<LuaUpvalue>>,
    pub next_frame_id: usize,
    pub error_handler: Option<LuaValue>,
    pub yield_values: Vec<LuaValue>,  // Values yielded from coroutine
    pub resume_values: Vec<LuaValue>, // Values passed to resume() that yield should return
    // For yield: store CALL instruction info to properly restore return values on resume
    pub yield_call_reg: Option<usize>, // Register where return values should be stored (A param of CALL)
    pub yield_call_nret: Option<usize>, // Number of expected return values (C-1 param of CALL)
}

/// Coroutine status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoroutineStatus {
    Suspended, // Created or yielded
    Running,   // Currently executing
    Normal,    // Resumed another coroutine
    Dead,      // Finished or error
}
