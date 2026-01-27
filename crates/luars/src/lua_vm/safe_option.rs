#[derive(Debug, Clone)]
pub struct SafeOption {
    pub max_stack_size: usize,
    pub max_call_depth: usize,
    pub short_string_limit: usize,
    /// Maximum memory limit in bytes
    pub max_memory_limit: isize,
}

impl Default for SafeOption {
    fn default() -> Self {
        Self {
            max_stack_size: 10000000,
            max_call_depth: 256,
            short_string_limit: 40,
            max_memory_limit: std::isize::MAX,
        }
    }
}
