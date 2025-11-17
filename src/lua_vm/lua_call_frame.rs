use std::rc::Rc;

use crate::LuaFunction;

// Common debug string constants (static lifetime, zero allocation)
pub const DEBUG_C_MARKER: &str = "[C]";
pub const DEBUG_UNKNOWN: &str = "?";
pub const DEBUG_DIRECT_CALL: &str = "[direct_call]";

pub struct LuaCallFrame {
    pub frame_id: usize, // Unique ID for this frame
    pub function: Rc<LuaFunction>,
    pub pc: usize,                 // Program counter
    pub base_ptr: usize,           // Index into global register_stack (register window start)
    pub top: usize,                // Top of stack for this frame (relative to base_ptr)
    pub result_reg: usize,         // Register to store return value in parent frame
    pub num_results: usize,        // Number of expected return values
    pub func_name: Option<&'static str>, // Function name for debugging (static string)
    pub source: Option<&'static str>,    // Source file/chunk name (static string)
    pub is_protected: bool,        // Is this a pcall frame?
    pub vararg_start: usize,       // Start index of variable arguments (relative to base_ptr)
    pub vararg_count: usize,       // Number of variable arguments
}

impl LuaCallFrame {
    pub fn new_lua_function(
        frame_id: usize,
        function: Rc<LuaFunction>,
        base_ptr: usize,
        max_stack_size: usize,
        result_reg: usize,
        num_results: usize,
    ) -> Self {
        LuaCallFrame {
            frame_id,
            function: function.clone(),
            pc: 0,
            base_ptr,
            top: max_stack_size,
            result_reg,
            num_results,
            func_name: None,
            source: None, // Will be resolved from chunk when needed
            is_protected: false,
            vararg_start: 0,
            vararg_count: 0,
        }
    }

    pub fn new_c_function(
        frame_id: usize,
        parent_function: Rc<LuaFunction>,
        parent_pc: usize,
        base_ptr: usize,
        num_args: usize,
    ) -> Self {
        LuaCallFrame {
            frame_id,
            function: parent_function,
            pc: parent_pc,
            base_ptr,
            top: num_args, // Set top to the number of arguments (including function at index 0)
            result_reg: 0,
            num_results: 0,
            func_name: Some(DEBUG_C_MARKER),
            source: Some(DEBUG_C_MARKER),
            is_protected: false,
            vararg_start: 0,
            vararg_count: 0,
        }
    }
}
