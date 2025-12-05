use crate::LuaValue;
use crate::gc::UpvalueId;
use crate::lua_vm::LuaCallFrame;

/// Lua Thread (coroutine)
/// Each coroutine has its own call stack and register stack, independent from the main VM
pub struct LuaThread {
    /// Coroutine status
    pub status: CoroutineStatus,

    /// Independent call stack for this coroutine
    /// Using Vec<LuaCallFrame> directly (no Box) for efficiency
    pub frames: Vec<LuaCallFrame>,

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

/// Maximum call stack depth (similar to LUAI_MAXCCALLS in Lua)
pub const MAX_CALL_DEPTH: usize = 256;

impl LuaThread {
    /// Create a new thread with pre-allocated stacks
    pub fn new(status: CoroutineStatus) -> Self {
        let mut frames = Vec::with_capacity(MAX_CALL_DEPTH);
        frames.resize_with(MAX_CALL_DEPTH, LuaCallFrame::default);
        
        LuaThread {
            status,
            frames,
            frame_count: 0,
            register_stack: Vec::with_capacity(1024),
            return_values: Vec::new(),
            open_upvalues: Vec::new(),
            next_frame_id: 0,
            yield_values: Vec::new(),
            resume_values: Vec::new(),
            yield_pc: None,
            yield_frame_id: None,
            yield_call_reg: None,
            yield_call_nret: None,
        }
    }

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

    // ============ Frame Management ============
    
    /// Push a new frame onto the call stack and return stable pointer
    #[inline(always)]
    pub fn push_frame(&mut self, frame: LuaCallFrame) -> *mut LuaCallFrame {
        let idx = self.frame_count;
        debug_assert!(idx < MAX_CALL_DEPTH, "call stack overflow");

        if idx < self.frames.len() {
            self.frames[idx] = frame;
        } else {
            self.frames.push(frame);
        }
        self.frame_count = idx + 1;
        &mut self.frames[idx] as *mut LuaCallFrame
    }

    /// Pop frame without returning it
    #[inline(always)]
    pub fn pop_frame_discard(&mut self) {
        debug_assert!(self.frame_count > 0, "pop from empty call stack");
        self.frame_count -= 1;
    }

    /// Pop the current frame from the call stack
    #[inline(always)]
    #[allow(dead_code)]
    pub fn pop_frame(&mut self) -> Option<LuaCallFrame> {
        if self.frame_count > 0 {
            self.frame_count -= 1;
            Some(unsafe { std::ptr::read(self.frames.as_ptr().add(self.frame_count)) })
        } else {
            None
        }
    }

    /// Check if call stack is empty
    #[inline(always)]
    pub fn frames_is_empty(&self) -> bool {
        self.frame_count == 0
    }

    /// Get current frame reference
    #[inline(always)]
    pub fn current_frame(&self) -> &LuaCallFrame {
        debug_assert!(self.frame_count > 0);
        unsafe { self.frames.get_unchecked(self.frame_count - 1) }
    }

    /// Get current frame mutable reference
    #[inline(always)]
    pub fn current_frame_mut(&mut self) -> &mut LuaCallFrame {
        debug_assert!(self.frame_count > 0);
        unsafe { self.frames.get_unchecked_mut(self.frame_count - 1) }
    }

    /// Get stable pointer to current frame
    #[inline(always)]
    pub fn current_frame_ptr(&mut self) -> *mut LuaCallFrame {
        debug_assert!(self.frame_count > 0);
        unsafe { self.frames.as_mut_ptr().add(self.frame_count - 1) }
    }

    /// Ensure register stack has capacity for at least `size` elements
    #[inline(always)]
    pub fn ensure_stack_capacity(&mut self, size: usize) {
        if self.register_stack.len() < size {
            self.register_stack.resize(size, LuaValue::nil());
        }
    }
}

/// Coroutine status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoroutineStatus {
    // Main thread (cannot yield)
    Main,
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
            CoroutineStatus::Main => "main",
            CoroutineStatus::Suspended => "suspended",
            CoroutineStatus::Running => "running",
            CoroutineStatus::Normal => "normal",
            CoroutineStatus::Dead => "dead",
        }
    }
}
