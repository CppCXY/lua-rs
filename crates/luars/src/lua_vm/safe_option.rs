#[derive(Debug, Clone)]
pub struct SafeOption {
    pub max_stack_size: usize,
    pub max_call_depth: usize,
    pub small_string_limit: usize,
}

impl Default for SafeOption {
    fn default() -> Self {
        Self {
            max_stack_size: 10000000,
            max_call_depth: 256,
            small_string_limit: 40,
        }
    }
}
