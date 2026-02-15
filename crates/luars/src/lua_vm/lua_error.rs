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

    /// Out of memory
    OutOfMemory,

    IndexOutOfBounds,
    /// VM exit (internal use) - returned when top-level frame returns
    Exit,
    /// Coroutine self-close: bypasses all pcalls and goes directly to resume().
    /// Equivalent to C Lua's luaD_throwbaselevel after luaE_resetthread.
    /// TBC vars and upvalues are already closed; error_object carries the
    /// close status (nil = success, non-nil = __close error value).
    CloseThread,
}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LuaError::RuntimeError => write!(f, "Runtime Error"),
            LuaError::CompileError => write!(f, "Compile Error"),
            LuaError::Yield => write!(f, "Coroutine Yield"),
            LuaError::StackOverflow => write!(f, "Stack Overflow"),
            LuaError::IndexOutOfBounds => write!(f, "Index Out Of Bounds"),
            LuaError::OutOfMemory => write!(f, "Out Of Memory"),
            LuaError::Exit => write!(f, "VM Exit"),
            LuaError::CloseThread => write!(f, "Close Thread"),
        }
    }
}
