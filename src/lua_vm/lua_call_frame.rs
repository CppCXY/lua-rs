use std::rc::Rc;

use crate::LuaFunction;

pub struct LuaCallFrame {
    pub frame_id: usize,          // Unique ID for this frame
    pub function: Rc<LuaFunction>,
    pub pc: usize,                // Program counter
    pub base_ptr: usize,          // Index into global register_stack (register window start)
    pub top: usize,               // Top of stack for this frame (relative to base_ptr)
    pub result_reg: usize,        // Register to store return value in parent frame
    pub num_results: usize,       // Number of expected return values
    pub func_name: Option<String>, // Function name for debugging
    pub source: Option<String>,   // Source file/chunk name
    pub is_protected: bool,       // Is this a pcall frame?
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
            source: function.chunk.source_name.clone(),
            is_protected: false,
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
            top: num_args,  // Set top to the number of arguments (including function at index 0)
            result_reg: 0,
            num_results: 0,
            func_name: Some("[C]".to_string()),
            source: Some("[C]".to_string()),
            is_protected: false,
        }
    }
}
