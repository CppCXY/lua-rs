use super::lua_limits::{LUAI_MAXCSTACK, LUAI_MAXSTACK, MAX_CALL_DEPTH};

#[derive(Debug, Clone)]
pub struct SafeOption {
    pub max_stack_size: usize,
    /// Maximum Lua call-stack depth (number of CallInfo frames).
    /// A pure-Lua recursion guard.  Default: `MAX_CALL_DEPTH` (1024).
    pub max_call_depth: usize,
    /// Maximum C-stack depth (Rust recursion depth, tracked by `n_ccalls`).
    /// Mirrors C Lua 5.5's `LUAI_MAXCSTACK`.  Default: 200.
    pub max_c_stack_depth: usize,
    /// Maximum memory limit in bytes
    pub max_memory_limit: isize,
}

impl Default for SafeOption {
    fn default() -> Self {
        Self {
            max_stack_size: LUAI_MAXSTACK,
            max_call_depth: MAX_CALL_DEPTH,
            max_c_stack_depth: LUAI_MAXCSTACK,
            max_memory_limit: isize::MAX,
        }
    }
}

pub(crate) struct LuaSafeState {
    pub max_stack_size: usize,
    /// Maximum Lua call-stack depth (number of CallInfo frames).
    /// A pure-Lua recursion guard.  Default: `MAX_CALL_DEPTH` (1024).
    pub max_call_depth: usize,
    /// Maximum C-stack depth (Rust recursion depth, tracked by `n_ccalls`).
    /// Mirrors C Lua 5.5's `LUAI_MAXCSTACK`.  Default: 200.
    pub max_c_stack_depth: usize,
    /// The *original* `max_c_stack_depth` before any error-handler increase.
    /// When a C-stack overflow occurs above this limit, it means we're in
    /// the extra zone for error handlers → produce "error in error handling".
    pub base_c_stack_depth: usize,
    /// The *original* `max_call_depth` before any error-handler increase.
    /// When a Lua call-stack overflow occurs above this limit, it means
    /// we're in the extra zone for error handlers → produce "error in error handling".
    pub base_call_depth: usize,
    #[allow(dead_code)]
    /// Maximum memory limit in bytes
    pub max_memory_limit: isize,
}

impl From<SafeOption> for LuaSafeState {
    fn from(option: SafeOption) -> Self {
        Self {
            max_stack_size: option.max_stack_size,
            max_call_depth: option.max_call_depth,
            max_c_stack_depth: option.max_c_stack_depth,
            base_c_stack_depth: option.max_c_stack_depth,
            base_call_depth: option.max_call_depth,
            max_memory_limit: option.max_memory_limit,
        }
    }
}
