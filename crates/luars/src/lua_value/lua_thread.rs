use crate::LuaValue;
use crate::gc::UpvalueId;

/// Lua Thread (coroutine)
/// Each coroutine has its own call stack and register stack, independent from the main VM
pub struct LuaThread {
    /// Coroutine status
    pub status: CoroutineStatus,

    /// Independent call stack for this coroutine
    /// Using Vec<LuaCallFrame> directly (no Box) for efficiency
    pub frames: Vec<crate::lua_vm::LuaCallFrame>,

    /// Current frame count (tracks active frames in the pre-allocated Vec)
    pub frame_count: usize,

    /// Independent register stack for this coroutine
    pub register_stack: Vec<LuaValue>,

    /// Return values from function calls
    pub return_values: Vec<LuaValue>,

    /// Open upvalues list (for closing when frames exit) - uses UpvalueId for new architecture
    pub open_upvalues: Vec<UpvalueId>,

    /// Next frame ID (for tracking frames in this coroutine)
    pub next_frame_id: usize,

    /// Error handler for this coroutine
    pub error_handler: Option<LuaValue>,

    /// Values yielded by coroutine.yield()
    /// These are returned to the resume() caller
    pub yield_values: Vec<LuaValue>,

    /// Values passed to coroutine.resume()
    /// These become the return values of yield() in the coroutine
    pub resume_values: Vec<LuaValue>,

    /// PC (program counter) where the coroutine yielded
    /// Used to resume execution from the correct position
    pub yield_pc: Option<usize>,

    /// Frame ID where the coroutine yielded
    pub yield_frame_id: Option<usize>,

    /// For yield inside a CALL instruction:
    /// The register where return values should be stored (A param of CALL)
    pub yield_call_reg: Option<usize>,

    /// For yield inside a CALL instruction:
    /// Number of expected return values (C-1 param of CALL, 0 = multiple returns)
    pub yield_call_nret: Option<usize>,
}

impl LuaThread {
    /// Check if this coroutine can be resumed
    pub fn can_resume(&self) -> bool {
        matches!(self.status, CoroutineStatus::Suspended)
    }

    /// Check if this coroutine is dead
    pub fn is_dead(&self) -> bool {
        matches!(self.status, CoroutineStatus::Dead)
    }

    /// Mark coroutine as running
    pub fn set_running(&mut self) {
        self.status = CoroutineStatus::Running;
    }

    /// Mark coroutine as suspended (after yield)
    pub fn set_suspended(&mut self) {
        self.status = CoroutineStatus::Suspended;
    }

    /// Mark coroutine as dead (finished or error)
    pub fn set_dead(&mut self) {
        self.status = CoroutineStatus::Dead;
    }

    /// Prepare for yield: save current state
    pub fn prepare_yield(&mut self, pc: usize, frame_id: usize) {
        self.yield_pc = Some(pc);
        self.yield_frame_id = Some(frame_id);
    }

    /// Clear yield state after resume
    pub fn clear_yield_state(&mut self) {
        self.yield_pc = None;
        self.yield_frame_id = None;
        self.yield_call_reg = None;
        self.yield_call_nret = None;
    }
}

/// Coroutine status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoroutineStatus {
    /// Created or yielded (can be resumed)
    Suspended,
    /// Currently executing
    Running,
    /// Resumed another coroutine (not directly resumable)
    Normal,
    /// Finished or encountered error
    Dead,
}

impl CoroutineStatus {
    /// Convert status to Lua string
    pub fn as_str(&self) -> &'static str {
        match self {
            CoroutineStatus::Suspended => "suspended",
            CoroutineStatus::Running => "running",
            CoroutineStatus::Normal => "normal",
            CoroutineStatus::Dead => "dead",
        }
    }
}
