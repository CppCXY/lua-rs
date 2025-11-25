use crate::LuaValue;

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
    /// VM exit (internal use) - returned when top-level frame returns
    Exit,
}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LuaError::RuntimeError => write!(f, "Runtime Error"),
            LuaError::CompileError => write!(f, "Compile Error"),
            LuaError::Yield => write!(f, "Coroutine Yield"),
            LuaError::Exit => write!(f, "VM Exit"),
        }
    }
}

/// Legacy error type for compatibility (will be phased out)
/// Used during transition period
#[derive(Debug, Clone)]
pub enum LuaErrorLegacy {
    RuntimeError(String),
    CompileError(String),
    Yield(Vec<LuaValue>),
}

impl From<LuaErrorLegacy> for LuaError {
    fn from(legacy: LuaErrorLegacy) -> Self {
        match legacy {
            LuaErrorLegacy::RuntimeError(_) => LuaError::RuntimeError,
            LuaErrorLegacy::CompileError(_) => LuaError::CompileError,
            LuaErrorLegacy::Yield(_) => LuaError::Yield,
        }
    }
}
