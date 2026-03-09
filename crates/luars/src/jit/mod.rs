pub mod recorder;
/// Tracing JIT compiler for Lua.
///
/// Records interpreter execution at hot backward-jump sites and
/// compiles the resulting trace to native code via Cranelift.
///
/// Hot counting uses per-instruction counters embedded in each Chunk
/// (`jit_counters: Vec<u16>`), indexed by PC.  This avoids HashMap
/// lookups on every backward jump — just a single array read/write.
///
/// Counter encoding:
/// - `JIT_COUNTER_BLACKLISTED` (0xFFFF): permanently excluded from JIT.
/// - `JIT_COUNTER_COMPILED` (0xFFFE): compiled trace exists at this PC.
/// - `1..=JIT_COUNTER_INIT`: counting down; when it reaches 0 recording
///   starts and the counter is reset to `JIT_COUNTER_INIT`.
pub mod runtime;
pub mod trace;
pub mod trace_compiler;

use std::collections::HashMap;

use self::recorder::TraceRecorder;
use self::trace::{AbortReason, RecordState};
use self::trace_compiler::CompiledTrace;

// ── Counter constants (public for use by Chunk initializer) ──────────────────

/// Initial counter value = HOT_THRESHOLD.  Counts down to 0.
pub const JIT_COUNTER_INIT: u16 = 50;

/// Site permanently excluded from JIT (abort count exceeded MAX_ABORTS).
pub const JIT_COUNTER_BLACKLISTED: u16 = 0xFFFF;

/// Site has a compiled trace.
pub const JIT_COUNTER_COMPILED: u16 = 0xFFFE;

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
            abort_counts: HashMap::new(),
            recorder: None,
            compiled: HashMap::new(),
            next_trace_id: 1,
        }
    }

    /// Start recording a new trace at the given head.
    pub fn start_recording(&mut self, chunk_ptr: usize, pc: u32, base: usize) {
        let id = self.next_trace_id;
        self.next_trace_id += 1;
        self.state = RecordState::Recording;
        self.recorder = Some(TraceRecorder::new(id, pc, base, chunk_ptr as *const u8));
    }

    /// Called when recording aborts.  Updates the Chunk's inline counter.
    ///
    /// `counters_ptr` is the raw pointer to the Chunk's `jit_counters` data
    /// for the trace head's chunk (NOT the current frame's chunk).
    pub fn abort_recording(
        &mut self,
        chunk_ptr: usize,
        pc: u32,
        counters_ptr: *mut u16,
        _reason: AbortReason,
    ) {
        self.state = RecordState::Idle;
        self.recorder = None;
        let key = (chunk_ptr, pc);
        let cnt = self.abort_counts.entry(key).or_insert(0);
        *cnt += 1;
        if *cnt >= MAX_ABORTS {
            // Blacklist: set inline counter so the dispatch loop never enters JIT path again.
            unsafe {
                *counters_ptr.add(pc as usize) = JIT_COUNTER_BLACKLISTED;
            }
        } else {
            // Reset counter for next attempt.
            unsafe {
                *counters_ptr.add(pc as usize) = JIT_COUNTER_INIT;
            }
        }
    }

    /// Called when a trace loop is closed successfully.
    pub fn finish_recording(&mut self, chunk_ptr: usize, pc: u32, counters_ptr: *mut u16) {
        let recorder = self.recorder.take().expect("finish without recorder");
        self.state = RecordState::Idle;
        let trace = recorder.finish();
        let key = (chunk_ptr, pc);
        match trace_compiler::compile_trace(&trace) {
            Ok(compiled) => {
                self.compiled.insert(key, compiled);
                // Mark counter as compiled so the dispatch loop knows to look up the trace.
                unsafe {
                    *counters_ptr.add(pc as usize) = JIT_COUNTER_COMPILED;
                }
            }
            Err(msg) => {
                eprintln!("[jit] trace compile failed: {msg}");
                let cnt = self.abort_counts.entry(key).or_insert(0);
                *cnt += 1;
                if *cnt >= MAX_ABORTS {
                    unsafe {
                        *counters_ptr.add(pc as usize) = JIT_COUNTER_BLACKLISTED;
                    }
                } else {
                    unsafe {
                        *counters_ptr.add(pc as usize) = JIT_COUNTER_INIT;
                    }
                }
            }
        }
    }

    /// Look up a compiled trace for the given head.
    pub fn get_compiled(&self, chunk_ptr: usize, pc: u32) -> Option<&CompiledTrace> {
        self.compiled.get(&(chunk_ptr, pc))
    }
}
