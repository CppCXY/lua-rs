#[derive(Debug, Clone)]
pub struct SafeOption {
    pub max_stack_size: usize,
    pub max_call_depth: usize,
    /// The original max_call_depth before any error-handler increases.
    /// When a stack overflow occurs above this limit, it means we're in
    /// the extra zone for error handlers → produce "error in error handling".
    pub base_call_depth: usize,
    /// Maximum memory limit in bytes
    pub max_memory_limit: isize,
}

impl Default for SafeOption {
    fn default() -> Self {
        Self {
            max_stack_size: 1000000, // LUAI_MAXSTACK (Lua 5.5)
            max_call_depth: 256,
            base_call_depth: 256,
            max_memory_limit: isize::MAX,
        }
    }
}
