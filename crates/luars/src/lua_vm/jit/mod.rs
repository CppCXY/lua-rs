use crate::compiler::LuaLanguageLevel;
use std::collections::HashMap;

mod artifact;
mod backend;
#[cfg(feature = "jit-cranelift")]
mod cranelift;
mod lowering;
mod record;
mod recorder;
mod replay;

pub use artifact::{CompiledTraceArtifact, CompiledTraceExit, CompiledTraceGuard, CompiledTraceStep};
pub use artifact::NativeTraceStub;
pub use artifact::TraceArtifact;
pub use backend::{
    LoweredTraceBackend, NoopTraceBackend, TraceBackend, TraceBackendError, TraceBackendKind,
    TraceCompilationUnit, make_trace_backend,
};
#[cfg(feature = "jit-cranelift")]
pub use cranelift::CraneliftTraceBackend;
pub use lowering::{LoweredTrace, LoweredTraceAnchor, LoweredTraceInstruction};
pub use record::{
    RecordingRequest, RecordingResult, TraceAnchorKind, TraceExit, TraceExitAction, TraceExitKind,
    TraceFallback, TraceGuard, TraceGuardKind, TraceGuardMode, TraceGuardOperands, TraceId,
    TraceInstruction, TracePlan, TraceSnapshot, TraceSnapshotKind,
};
use recorder::TraceRecorder;
pub use replay::execute_trace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceAbortReason {
    NotImplemented,
    UnsupportedOpcode,
    UnsupportedControlFlow,
    SideEffectBoundary,
    InvalidAnchor,
    TraceTooLong,
    Blacklisted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorState {
    Interpreting,
    Recording,
    Compiled(TraceId),
    Blacklisted(TraceAbortReason),
}

#[derive(Debug, Clone, Copy)]
pub struct JitPolicy {
    pub hotloop_threshold: u16,
    pub max_trace_instructions: u16,
    pub max_trace_replays: u16,
}

impl Default for JitPolicy {
    fn default() -> Self {
        Self {
            hotloop_threshold: 57,
            max_trace_instructions: 256,
            max_trace_replays: u16::MAX,
        }
    }
}

#[derive(Debug, Default)]
pub struct JitChunkState {
    pub hotloop_counters: Vec<u16>,
    pub anchor_states: Vec<AnchorState>,
}

impl JitChunkState {
    fn new(code_len: usize, threshold: u16) -> Self {
        Self {
            hotloop_counters: vec![threshold; code_len],
            anchor_states: vec![AnchorState::Interpreting; code_len],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotLoopAction {
    None,
    StartRecording { anchor_pc: usize },
    RunTrace { trace_id: TraceId },
}

#[derive(Debug)]
pub struct JitRuntime {
    policy: JitPolicy,
    chunk_states: HashMap<usize, JitChunkState>,
    traces: Vec<TraceArtifact>,
    next_trace_id: u32,
    backend: Box<dyn TraceBackend>,
}

impl JitRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_backend_kind(kind: TraceBackendKind) -> Self {
        Self {
            backend: make_trace_backend(kind),
            ..Self::default()
        }
    }

    pub fn with_backend(backend: Box<dyn TraceBackend>) -> Self {
        Self {
            backend,
            ..Self::default()
        }
    }

    pub fn policy(&self) -> JitPolicy {
        self.policy
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    pub fn backend_kind(&self) -> TraceBackendKind {
        self.backend.kind()
    }

    pub fn set_backend(&mut self, backend: Box<dyn TraceBackend>) {
        self.backend = backend;
    }

    pub fn set_backend_kind(&mut self, kind: TraceBackendKind) {
        self.backend = make_trace_backend(kind);
    }

    fn compile_trace_artifact(&self, plan: TracePlan) -> TraceArtifact {
        let unit = TraceCompilationUnit::new(plan.clone());
        match self.backend.compile(&unit) {
            Ok(Some(artifact)) if artifact.id() == plan.id => artifact,
            Ok(Some(_)) | Ok(None) | Err(_) => TraceArtifact::Replay(plan),
        }
    }

    fn chunk_state_mut(&mut self, chunk_key: usize, code_len: usize) -> &mut JitChunkState {
        self.chunk_states
            .entry(chunk_key)
            .or_insert_with(|| JitChunkState::new(code_len, self.policy.hotloop_threshold))
    }

    pub fn should_use_jit_execute_loop(&self, level: LuaLanguageLevel) -> bool {
        matches!(level, LuaLanguageLevel::LuaJIT)
    }

    #[cold]
    pub fn try_start_recording(
        &mut self,
        chunk: &crate::Chunk,
        request: RecordingRequest,
    ) -> RecordingResult {
        let trace_id = self.next_trace_id();
        match TraceRecorder::new(self.policy, chunk, request).record(trace_id) {
            Ok(plan) => {
                let artifact = self.compile_trace_artifact(plan);
                self.traces.push(artifact);
                RecordingResult::Compiled(trace_id)
            }
            Err(reason) => RecordingResult::Abort(reason),
        }
    }

    pub fn on_loop_backedge(
        &mut self,
        chunk_key: usize,
        code_len: usize,
        anchor_pc: usize,
    ) -> HotLoopAction {
        let state = self.chunk_state_mut(chunk_key, code_len);
        if anchor_pc >= state.anchor_states.len() {
            return HotLoopAction::None;
        }

        match state.anchor_states[anchor_pc] {
            AnchorState::Interpreting => {
                let counter = &mut state.hotloop_counters[anchor_pc];
                if *counter > 0 {
                    *counter -= 1;
                }
                if *counter == 0 {
                    state.anchor_states[anchor_pc] = AnchorState::Recording;
                    HotLoopAction::StartRecording { anchor_pc }
                } else {
                    HotLoopAction::None
                }
            }
            AnchorState::Recording | AnchorState::Blacklisted(_) => HotLoopAction::None,
            AnchorState::Compiled(trace_id) => HotLoopAction::RunTrace { trace_id },
        }
    }

    pub fn finish_recording(
        &mut self,
        chunk_key: usize,
        code_len: usize,
        anchor_pc: usize,
        result: RecordingResult,
    ) {
        let threshold = self.policy.hotloop_threshold;
        let state = self.chunk_state_mut(chunk_key, code_len);
        if anchor_pc >= state.anchor_states.len() {
            return;
        }

        match result {
            RecordingResult::Compiled(trace_id) => {
                state.anchor_states[anchor_pc] = AnchorState::Compiled(trace_id);
                state.hotloop_counters[anchor_pc] = threshold;
            }
            RecordingResult::Abort(reason) => {
                state.anchor_states[anchor_pc] = AnchorState::Blacklisted(reason);
                state.hotloop_counters[anchor_pc] = threshold;
            }
        }
    }

    pub fn abort_trace(
        &mut self,
        chunk_key: usize,
        code_len: usize,
        anchor_pc: usize,
        reason: TraceAbortReason,
    ) {
        self.finish_recording(
            chunk_key,
            code_len,
            anchor_pc,
            RecordingResult::Abort(reason),
        );
    }

    pub fn next_trace_id(&mut self) -> TraceId {
        let trace_id = TraceId(self.next_trace_id);
        self.next_trace_id = self.next_trace_id.wrapping_add(1);
        trace_id
    }

    pub fn trace_artifact(&self, trace_id: TraceId) -> Option<&TraceArtifact> {
        self.traces
            .iter()
            .find(|artifact| artifact.id() == trace_id)
    }

    pub fn trace_plan(&self, trace_id: TraceId) -> Option<&TracePlan> {
        self.trace_artifact(trace_id).map(TraceArtifact::plan)
    }

    pub fn trace_count(&self) -> usize {
        self.traces.len()
    }

    pub fn replace_trace_artifact(&mut self, trace_id: TraceId, artifact: TraceArtifact) -> bool {
        if let Some(slot) = self
            .traces
            .iter_mut()
            .find(|existing| existing.id() == trace_id)
        {
            *slot = artifact;
            true
        } else {
            false
        }
    }
}

impl Default for JitRuntime {
    fn default() -> Self {
        Self {
            policy: JitPolicy::default(),
            chunk_states: HashMap::new(),
            traces: Vec::new(),
            next_trace_id: 0,
            backend: make_trace_backend(TraceBackendKind::Lowered),
        }
    }
}

impl Default for AnchorState {
    fn default() -> Self {
        Self::Interpreting
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{Chunk, Instruction, OpCode};

    use super::*;

    #[derive(Debug)]
    struct PlaceholderBackend {
        seen: Arc<Mutex<Vec<TraceCompilationUnit>>>,
    }

    impl TraceBackend for PlaceholderBackend {
        fn name(&self) -> &'static str {
            "placeholder"
        }

        fn kind(&self) -> TraceBackendKind {
            TraceBackendKind::Noop
        }

        fn compile(
            &self,
            unit: &TraceCompilationUnit,
        ) -> Result<Option<TraceArtifact>, TraceBackendError> {
            self.seen
                .lock()
                .expect("backend trace log poisoned")
                .push(unit.clone());
            Ok(Some(TraceArtifact::NativePlaceholder(NativeTraceStub {
                unit: unit.clone(),
            })))
        }
    }

    #[test]
    fn runtime_stores_compiled_trace_plan() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_asbx(OpCode::LoadI, 0, 1),
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];

        let mut runtime = JitRuntime::new();
        let request = RecordingRequest {
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 1,
            current_pc: 1,
            base: 4,
            frame_depth: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
        };

        let result = runtime.try_start_recording(&chunk, request);
        let trace_id = match result {
            RecordingResult::Compiled(trace_id) => trace_id,
            RecordingResult::Abort(reason) => panic!("unexpected abort: {reason:?}"),
        };

        assert_eq!(runtime.trace_count(), 1);
        let plan = runtime.trace_plan(trace_id).expect("trace plan missing");
        assert_eq!(plan.anchor_pc, 1);
        assert_eq!(plan.end_pc, 2);
        assert_eq!(plan.snapshots.len(), 3);
        assert_eq!(plan.guards.len(), 2);
        assert!(
            plan.guards
                .iter()
                .all(|guard| guard.mode == TraceGuardMode::Precondition)
        );
        assert_eq!(plan.exits.len(), 2);
        assert!(plan.exits.iter().all(|exit| exit.target_pc == 1));

        let artifact = runtime
            .trace_artifact(trace_id)
            .expect("trace artifact missing");
        match artifact {
            TraceArtifact::Compiled(compiled) => {
                assert_eq!(compiled.unit.lowered.id, trace_id);
                assert_eq!(compiled.steps.len(), 2);
            }
            _ => panic!("expected compiled trace artifact"),
        }
    }

    #[test]
    fn runtime_can_swap_in_native_placeholder_artifact() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_asbx(OpCode::LoadI, 0, 1),
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];

        let mut runtime = JitRuntime::new();
        let request = RecordingRequest {
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 1,
            current_pc: 1,
            base: 4,
            frame_depth: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
        };

        let trace_id = match runtime.try_start_recording(&chunk, request) {
            RecordingResult::Compiled(trace_id) => trace_id,
            RecordingResult::Abort(reason) => panic!("unexpected abort: {reason:?}"),
        };

        let plan = runtime
            .trace_plan(trace_id)
            .expect("trace plan missing")
            .clone();
        assert!(runtime.replace_trace_artifact(
            trace_id,
            TraceArtifact::NativePlaceholder(NativeTraceStub {
                unit: TraceCompilationUnit::new(plan.clone()),
            }),
        ));

        let artifact = runtime
            .trace_artifact(trace_id)
            .expect("trace artifact missing");
        assert_eq!(artifact.plan(), &plan);
        match artifact {
            TraceArtifact::Compiled(compiled) => {
                assert_eq!(compiled.unit.lowered.anchor.pc, 1);
            }
            TraceArtifact::NativePlaceholder(_) => {}
            _ => panic!("expected compiled-capable artifact"),
        }
    }

    #[test]
    fn runtime_uses_backend_compiled_artifact_when_available() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_asbx(OpCode::LoadI, 0, 1),
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut runtime =
            JitRuntime::with_backend(Box::new(PlaceholderBackend { seen: seen.clone() }));
        let request = RecordingRequest {
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 1,
            current_pc: 1,
            base: 4,
            frame_depth: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
        };

        let trace_id = match runtime.try_start_recording(&chunk, request) {
            RecordingResult::Compiled(trace_id) => trace_id,
            RecordingResult::Abort(reason) => panic!("unexpected abort: {reason:?}"),
        };

        assert_eq!(runtime.backend_name(), "placeholder");
        let seen = seen.lock().expect("backend trace log poisoned");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].lowered.id, trace_id);
        assert_eq!(seen[0].lowered.anchor.pc, 1);
        drop(seen);

        let artifact = runtime
            .trace_artifact(trace_id)
            .expect("trace artifact missing");
        match artifact {
            TraceArtifact::NativePlaceholder(stub) => {
                assert_eq!(stub.unit.lowered.anchor.pc, 1);
                assert_eq!(stub.unit.lowered.anchor.end_pc, 2);
            }
            _ => panic!("expected native placeholder artifact"),
        }
    }

    #[test]
    fn runtime_can_switch_backend_kind() {
        let mut runtime = JitRuntime::with_backend_kind(TraceBackendKind::Lowered);
        assert_eq!(runtime.backend_kind(), TraceBackendKind::Lowered);
        assert_eq!(runtime.backend_name(), "lowered");

        runtime.set_backend_kind(TraceBackendKind::Noop);
        assert_eq!(runtime.backend_kind(), TraceBackendKind::Noop);
        assert_eq!(runtime.backend_name(), "noop");
    }

    #[cfg(feature = "jit-cranelift")]
    #[test]
    fn runtime_can_switch_to_cranelift_backend_kind() {
        let runtime = JitRuntime::with_backend_kind(TraceBackendKind::Cranelift);
        assert_eq!(runtime.backend_kind(), TraceBackendKind::Cranelift);
        assert_eq!(runtime.backend_name(), "cranelift");
    }
}
