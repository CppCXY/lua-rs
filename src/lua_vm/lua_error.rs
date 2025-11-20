#[derive(Debug)]
pub enum LuaError {
    RuntimeError(String),
    CompileError(String),
    // Add more error types as needed
}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LuaError::RuntimeError(msg) => write!(f, "Runtime Error: {}", msg),
            LuaError::CompileError(msg) => write!(f, "Compile Error: {}", msg),
        }
    }
}
