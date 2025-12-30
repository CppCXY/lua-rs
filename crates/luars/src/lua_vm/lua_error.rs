/// Lightweight error enum - only 1 byte!
/// Actual error data stored in VM to reduce Result size
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LuaError {
    /// Runtime error - message stored in vm.error_message
    RuntimeError,
    /// Compile error - message stored in vm.error_message
    CompileError,
    /// Coroutine yield - values stored in vm.yield_values
    Yield,
    /// Stack overflow
    StackOverflow,
    /// VM exit (internal use) - returned when top-level frame returns
    Exit,
}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LuaError::RuntimeError => write!(f, "Runtime Error"),
            LuaError::CompileError => write!(f, "Compile Error"),
            LuaError::Yield => write!(f, "Coroutine Yield"),
            LuaError::StackOverflow => write!(f, "Stack Overflow"),
            LuaError::Exit => write!(f, "VM Exit"),
        }
    }
}
