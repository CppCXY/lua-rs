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
    /// Stack overflow while in the error-handler extra zone (C Lua's stackerror).
    /// Produces "error in error handling" without invoking any further handler.
    ErrorInErrorHandling,
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
            LuaError::ErrorInErrorHandling => write!(f, "Error In Error Handling"),
        }
    }
}

impl std::error::Error for LuaError {}

/// Rich error type combining [`LuaError`] kind with the actual Lua error message.
///
/// Created via [`LuaVM::into_full_error`](super::LuaVM::into_full_error)
/// after catching a `LuaError`.
///
/// Implements `Display` and `std::error::Error`, so it integrates with
/// `anyhow`, `thiserror`, and the `?` operator:
///
/// ```ignore
/// let result = vm.execute("bad code")
///     .map_err(|e| vm.into_full_error(e))?; // propagates with full message
/// ```
#[derive(Debug, Clone)]
pub struct LuaFullError {
    /// The error variant (RuntimeError, CompileError, etc.)
    pub kind: LuaError,
    /// The human-readable error message with source location and traceback
    pub message: String,
}

impl std::fmt::Display for LuaFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", self.kind)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for LuaFullError {}

impl LuaFullError {
    /// Returns the error kind ([`LuaError`] variant).
    #[inline]
    pub fn kind(&self) -> LuaError {
        self.kind
    }

    /// Returns the error message.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }
}
