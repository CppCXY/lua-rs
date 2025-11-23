use crate::LuaValue;

#[derive(Debug, Clone)]
pub enum LuaError {
    RuntimeError(String),
    CompileError(String),
    /// Special error type for coroutine yield (not a real error)
    /// Contains the yield values
    Yield(Vec<LuaValue>),
    // Add more error types as needed
}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LuaError::RuntimeError(msg) => write!(f, "Runtime Error: {}", msg),
            LuaError::CompileError(msg) => write!(f, "Compile Error: {}", msg),
            LuaError::Yield(_) => write!(f, "Coroutine Yield"),
        }
    }
}
