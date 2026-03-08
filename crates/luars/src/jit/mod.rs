/// JIT compiler for Lua — two-tier approach.
///
/// ## Tier 1: Loop-only JIT (existing)
///
/// Compiles simple numeric for-loops to native code. Triggered by
/// `ForPrep` when the expected iteration count ≥ `JIT_MIN_ITERS`.
///
/// ## Tier 2: Tracing JIT (new, in progress)
///
/// Records interpreter execution at hot backward-jump sites and
/// compiles the resulting trace to native code via Cranelift.
///
/// The tracer counts backward jumps (ForLoop, TForLoop, backward Jmp).
/// Once a site reaches `HOT_THRESHOLD`, recording begins.  The recorder
/// translates each subsequent interpreter instruction into `TraceIr`
/// nodes.  When execution loops back to the trace head, the trace is
/// closed and compiled.  On subsequent visits the compiled trace is
/// executed directly.

// ── Tier 1: loop-only JIT ───────────────────────────────────────────
pub mod analyzer;
pub mod compiler;
pub mod runtime;

// ── Tier 2: tracing JIT ─────────────────────────────────────────────
pub mod trace;
pub mod recorder;
pub mod trace_compiler;

use std::collections::HashMap;

use self::recorder::TraceRecorder;
use self::trace::{RecordState, AbortReason};
use self::trace_compiler::CompiledTrace;

/// Minimum loop iteration count to trigger loop-only JIT compilation.
pub const JIT_MIN_ITERS: usize = 1000;

/// Sentinel: "compilation was attempted and failed".
pub const JIT_FAILED: usize = 0;

/// Compiled loop function: `fn(stack_base: *mut u8) -> i32`.
pub type JitLoopFn = unsafe extern "C" fn(*mut u8) -> i32;

/// Try to JIT-compile the integer for-loop whose `ForPrep` is at `prep_pc`.
pub fn try_compile_loop(
    chunk: &crate::lua_value::Chunk,
    prep_pc: usize,
) -> Option<JitLoopFn> {
    let analysis = analyzer::analyze(chunk, prep_pc)?;
    compiler::compile(&analysis)
}

// ── Tracing JIT state ────────────────────────────────────────────────────────

/// Number of backward-jump hits before tracing is triggered.
const HOT_THRESHOLD: u32 = 50;

/// Maximum number of abort strikes before blacklisting a trace head.
const MAX_ABORTS: u32 = 10;

/// A (chunk_ptr, pc) pair identifying a backward-jump site.
type TraceKey = (usize, u32);

/// Per-VM tracing JIT state.
///
/// Stored in `LuaState` and threaded through the interpreter.
pub struct JitState {
    /// Recording state machine.
    pub state: RecordState,

    /// Hot-counter for backward-jump sites.  Each site is identified by
    /// `(chunk_ptr as usize, pc)`.  Incremented on every backward jump.
    hot_counts: HashMap<TraceKey, u32>,

    /// Abort history — how many times recording failed for each site.
    abort_counts: HashMap<TraceKey, u32>,

    /// Active recorder (Some only when `state == Recording`).
    pub recorder: Option<TraceRecorder>,

    /// Successfully compiled traces, keyed by trace head.
    compiled: HashMap<TraceKey, CompiledTrace>,

    /// Monotonically increasing trace ID.
    next_trace_id: u32,
}

impl JitState {
    /// Create a fresh JIT state.
    pub fn new() -> Self {
        Self {
            state: RecordState::Idle,
            hot_counts: HashMap::new(),
            abort_counts: HashMap::new(),
            recorder: None,
            compiled: HashMap::new(),
            next_trace_id: 1,
        }
    }

    /// Called by the interpreter at every backward jump.
    ///
    /// Returns `true` if recording should start at this site.
    pub fn count_hot(&mut self, chunk_ptr: usize, pc: u32) -> bool {
        if self.state != RecordState::Idle {
            return false; // already recording
        }
        let key = (chunk_ptr, pc);
        // Already compiled or blacklisted?
        if self.compiled.contains_key(&key) {
            return false;
        }
        if self.abort_counts.get(&key).copied().unwrap_or(0) >= MAX_ABORTS {
            return false;
        }
        let count = self.hot_counts.entry(key).or_insert(0);
        *count += 1;
        *count >= HOT_THRESHOLD
    }

    /// Start recording a new trace at the given head.
    pub fn start_recording(&mut self, chunk_ptr: usize, pc: u32, base: usize) {
        let id = self.next_trace_id;
        self.next_trace_id += 1;
        self.state = RecordState::Recording;
        self.recorder = Some(TraceRecorder::new(
            id,
            pc,
            base,
            chunk_ptr as *const u8,
        ));
    }

    /// Called when recording aborts.
    pub fn abort_recording(&mut self, chunk_ptr: usize, pc: u32, _reason: AbortReason) {
        self.state = RecordState::Idle;
        self.recorder = None;
        let key = (chunk_ptr, pc);
        let cnt = self.abort_counts.entry(key).or_insert(0);
        *cnt += 1;
        // Reset hot counter so we don't immediately re-trigger.
        self.hot_counts.remove(&key);
    }

    /// Called when a trace loop is closed successfully.
    pub fn finish_recording(&mut self, chunk_ptr: usize, pc: u32) {
        let recorder = self.recorder.take().expect("finish without recorder");
        self.state = RecordState::Idle;
        let trace = recorder.finish();
        let key = (chunk_ptr, pc);
        match trace_compiler::compile_trace(&trace) {
            Ok(compiled) => {
                self.compiled.insert(key, compiled);
            }
            Err(msg) => {
                eprintln!("[jit] trace compile failed: {msg}");
                let cnt = self.abort_counts.entry(key).or_insert(0);
                *cnt += 1;
            }
        }
        self.hot_counts.remove(&key);
    }

    /// Look up a compiled trace for the given head.
    pub fn get_compiled(&self, chunk_ptr: usize, pc: u32) -> Option<&CompiledTrace> {
        self.compiled.get(&(chunk_ptr, pc))
    }
}
