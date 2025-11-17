use crate::LuaValue;

// Common debug string constants (static lifetime, zero allocation)
pub const DEBUG_C_MARKER: &str = "[C]";

pub struct LuaCallFrame {
    pub frame_id: usize,                 // Unique ID for this frame
    pub function_value: LuaValue,        // Function value (contains FunctionId or CFunction)
    pub cached_function_id: Option<crate::object_pool::FunctionId>, // Cached function ID for fast access
    pub cached_code_ptr: Option<*const Vec<u32>>, // Cached pointer to chunk.code for ultra-fast instruction fetch
    pub cached_code_len: usize,          // Cached code length for bounds checking
    pub cached_constants_ptr: Option<*const Vec<LuaValue>>, // Cached pointer to chunk.constants for ultra-fast LoadK
    pub cached_constants_len: usize,     // Cached constants length for bounds checking
    pub pc: usize,                       // Program counter
    pub base_ptr: usize,                 // Index into global register_stack (register window start)
    pub top: usize,                      // Top of stack for this frame (relative to base_ptr)
    pub result_reg: usize,               // Register to store return value in parent frame
    pub num_results: usize,              // Number of expected return values
    pub func_name: Option<&'static str>, // Function name for debugging (static string)
    pub source: Option<&'static str>,    // Source file/chunk name (static string)
    pub is_protected: bool,              // Is this a pcall frame?
    pub vararg_start: usize,             // Start index of variable arguments (relative to base_ptr)
    pub vararg_count: usize,             // Number of variable arguments
}

impl LuaCallFrame {
    pub fn new_lua_function(
        frame_id: usize,
        function_value: LuaValue,
        base_ptr: usize,
        max_stack_size: usize,
        result_reg: usize,
        num_results: usize,
    ) -> Self {
        let cached_function_id = function_value.as_function_id();
        LuaCallFrame {
            frame_id,
            function_value,
            cached_function_id,
            cached_code_ptr: None,  // Will be set by VM after frame creation
            cached_code_len: 0,
            cached_constants_ptr: None,  // Will be set by VM after frame creation
            cached_constants_len: 0,
            pc: 0,
            base_ptr,
            top: max_stack_size,
            result_reg,
            num_results,
            func_name: None,
            source: None,
            is_protected: false,
            vararg_start: 0,
            vararg_count: 0,
        }
    }

    pub fn new_c_function(
        frame_id: usize,
        parent_function_value: LuaValue,
        parent_pc: usize,
        base_ptr: usize,
        num_args: usize,
    ) -> Self {
        LuaCallFrame {
            frame_id,
            function_value: parent_function_value,
            cached_function_id: None,
            cached_code_ptr: None,
            cached_code_len: 0,
            cached_constants_ptr: None,
            cached_constants_len: 0,
            pc: parent_pc,
            base_ptr,
            top: num_args,
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
