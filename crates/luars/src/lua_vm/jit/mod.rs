use crate::compiler::LuaLanguageLevel;
use std::collections::HashMap;
use std::sync::Arc;

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
    RecordingRequest, RecordingResult, SideTraceKey, TraceAnchorKind, TraceExit, TraceExitAction,
    TraceExitKind, TraceFallback, TraceGuard, TraceGuardKind, TraceGuardMode,
    TraceGuardOperands, TraceId, TraceInstruction, TracePlan, TraceSnapshot,
    TraceSnapshotKind,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceRunOutcome {
    Anchored,
    SideExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceRunResult {
    pub next_pc: usize,
    pub outcome: TraceRunOutcome,
    pub exit: Option<TraceExitSite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceDispatchResult {
    pub trace_id: TraceId,
    pub run_result: TraceRunResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceExitSite {
    pub kind: TraceExitKind,
    pub source_pc: usize,
    pub target_pc: usize,
    pub snapshot_index: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TraceRunStats {
    pub executions: u32,
    pub anchored_returns: u32,
    pub side_exits: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SideTraceStats {
    pub exits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SideTraceState {
    Interpreting,
    Recording,
    Compiled(TraceId),
    Blacklisted(TraceAbortReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SideTraceSlot {
    hot_counter: u16,
    stats: SideTraceStats,
    state: SideTraceState,
}

const TRACE_DEMOTE_WARMUP_RUNS: u32 = 32;
const TRACE_MAX_SIDE_EXIT_RATIO_PERCENT: u32 = 25;
const SIDE_TRACE_HOT_THRESHOLD: u16 = 8;

#[derive(Debug)]
pub struct JitRuntime {
    policy: JitPolicy,
    chunk_states: HashMap<usize, JitChunkState>,
    traces: Vec<Arc<TraceArtifact>>,
    trace_stats: HashMap<TraceId, TraceRunStats>,
    side_traces: HashMap<SideTraceKey, SideTraceSlot>,
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

    fn should_demote_trace(stats: TraceRunStats) -> bool {
        stats.executions >= TRACE_DEMOTE_WARMUP_RUNS
            && stats.side_exits.saturating_mul(100)
                >= stats
                    .executions
                    .saturating_mul(TRACE_MAX_SIDE_EXIT_RATIO_PERCENT)
    }

    pub fn trace_run_stats(&self, trace_id: TraceId) -> Option<TraceRunStats> {
        self.trace_stats.get(&trace_id).copied()
    }

    pub fn side_trace_stats(&self, key: SideTraceKey) -> Option<SideTraceStats> {
        self.side_traces.get(&key).map(|slot| slot.stats)
    }

    pub fn side_trace(&self, key: SideTraceKey) -> Option<TraceId> {
        match self.side_traces.get(&key).map(|slot| slot.state) {
            Some(SideTraceState::Compiled(trace_id)) => Some(trace_id),
            _ => None,
        }
    }

    pub fn side_trace_key(&self, trace_id: TraceId, result: TraceRunResult) -> Option<SideTraceKey> {
        let exit = result.exit?;
        if exit.kind != TraceExitKind::GuardExit {
            return None;
        }

        let plan = self.trace_plan(trace_id)?;
        if exit.target_pc == plan.anchor_pc {
            return None;
        }

        Some(SideTraceKey {
            parent_trace: trace_id,
            exit_snapshot_index: exit.snapshot_index,
        })
    }

    fn side_trace_slot_mut(&mut self, key: SideTraceKey) -> &mut SideTraceSlot {
        self.side_traces.entry(key).or_insert_with(|| SideTraceSlot {
            hot_counter: self.policy.hotloop_threshold.min(SIDE_TRACE_HOT_THRESHOLD),
            stats: SideTraceStats::default(),
            state: SideTraceState::Interpreting,
        })
    }

    pub fn note_trace_side_exit(
        &mut self,
        trace_id: TraceId,
        result: TraceRunResult,
        base: usize,
        frame_depth: usize,
    ) -> Option<(SideTraceKey, RecordingRequest)> {
        let exit = result.exit?;
        if exit.kind != TraceExitKind::GuardExit {
            return None;
        }

        let (chunk_key, anchor_pc) = {
            let plan = self.trace_plan(trace_id)?;
            (plan.chunk_key, plan.anchor_pc)
        };
        if exit.target_pc == anchor_pc {
            return None;
        }

        let key = SideTraceKey {
            parent_trace: trace_id,
            exit_snapshot_index: exit.snapshot_index,
        };

        let slot = self.side_trace_slot_mut(key);
        slot.stats.exits = slot.stats.exits.saturating_add(1);
        match slot.state {
            SideTraceState::Interpreting => {
                if slot.hot_counter > 0 {
                    slot.hot_counter -= 1;
                }
                if slot.hot_counter == 0 {
                    slot.state = SideTraceState::Recording;
                    Some((
                        key,
                        RecordingRequest {
                            chunk_key,
                            anchor_pc,
                            start_pc: exit.target_pc,
                            current_pc: exit.target_pc,
                            base,
                            frame_depth,
                            anchor_kind: TraceAnchorKind::SideExit,
                            parent_side_trace: Some(key),
                        },
                    ))
                } else {
                    None
                }
            }
            SideTraceState::Recording
            | SideTraceState::Compiled(_)
            | SideTraceState::Blacklisted(_) => None,
        }
    }

    pub fn finish_side_trace_recording(&mut self, key: SideTraceKey, result: RecordingResult) {
        let threshold = self.policy.hotloop_threshold;
        let linked_trace_id = match result {
            RecordingResult::Compiled(trace_id) => Some(trace_id),
            RecordingResult::Abort(_) => None,
        };
        let slot = self.side_trace_slot_mut(key);
        slot.hot_counter = threshold;
        slot.state = match result {
            RecordingResult::Compiled(trace_id) => SideTraceState::Compiled(trace_id),
            RecordingResult::Abort(reason) => SideTraceState::Blacklisted(reason),
        };

        if let Some(trace_id) = linked_trace_id {
            let _ = self.link_side_trace(key, trace_id);
        }
    }

    pub fn report_trace_result(
        &mut self,
        chunk_key: usize,
        code_len: usize,
        anchor_pc: usize,
        trace_id: TraceId,
        result: TraceRunResult,
    ) -> bool {
        let stats = self.trace_stats.entry(trace_id).or_default();
        stats.executions = stats.executions.saturating_add(1);
        match result.outcome {
            TraceRunOutcome::Anchored => {
                stats.anchored_returns = stats.anchored_returns.saturating_add(1);
            }
            TraceRunOutcome::SideExit => {
                stats.side_exits = stats.side_exits.saturating_add(1);
            }
        }

        if !Self::should_demote_trace(*stats) {
            return false;
        }

        let threshold = self.policy.hotloop_threshold;
        let state = self.chunk_state_mut(chunk_key, code_len);
        if anchor_pc >= state.anchor_states.len() {
            return false;
        }

        state.anchor_states[anchor_pc] = AnchorState::Blacklisted(TraceAbortReason::Blacklisted);
        state.hotloop_counters[anchor_pc] = threshold;
        true
    }

    fn compile_trace_artifact(&self, plan: TracePlan) -> TraceArtifact {
        let unit = TraceCompilationUnit::new(plan.clone());
        let primary_backend = self.backend.name();
        let (artifact, selected_backend) = match self.backend.compile(&unit) {
            Ok(Some(artifact)) if artifact.id() == plan.id => (artifact, primary_backend),
            Ok(Some(_)) | Ok(None) | Err(_) => (
                LoweredTraceBackend
                .compile(&unit)
                .ok()
                .flatten()
                .filter(|artifact| artifact.id() == plan.id)
                .unwrap_or(TraceArtifact::Replay(plan)),
                "lowered",
            ),
        };

        maybe_dump_trace_compilation(&unit, &artifact, primary_backend, selected_backend);
        artifact
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
                self.trace_stats.insert(trace_id, TraceRunStats::default());
                self.traces.push(Arc::new(artifact));
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
            .map(Arc::as_ref)
    }

        fn trace_artifact_mut(&mut self, trace_id: TraceId) -> Option<&mut TraceArtifact> {
        self.traces
            .iter_mut()
            .find(|artifact| artifact.id() == trace_id)
            .map(Arc::make_mut)
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
            *slot = Arc::new(artifact);
            true
        } else {
            false
        }
    }

    fn link_side_trace(&mut self, key: SideTraceKey, side_trace_id: TraceId) -> bool {
        self.trace_artifact_mut(key.parent_trace)
            .is_some_and(|artifact| artifact.link_side_trace(key.exit_snapshot_index, side_trace_id))
    }
}

fn maybe_dump_trace_compilation(
    unit: &TraceCompilationUnit,
    artifact: &TraceArtifact,
    primary_backend: &str,
    selected_backend: &str,
) {
    let Ok(mode) = std::env::var("LUARS_JIT_TRACE_DUMP") else {
        return;
    };

    let dump_all = mode.eq_ignore_ascii_case("all");
    let has_control_guard = unit
        .lowered
        .guards
        .iter()
        .any(|guard| guard.mode == TraceGuardMode::Control);
    if !dump_all && !has_control_guard {
        return;
    }

    eprintln!(
        "[jit-trace] id={:?} anchor={:?}@{}..{} primary_backend={} selected_backend={} artifact={} instructions={} guards={} exits={}",
        unit.plan.id,
        unit.plan.anchor_kind,
        unit.plan.anchor_pc,
        unit.plan.end_pc,
        primary_backend,
        selected_backend,
        trace_artifact_kind_name(artifact),
        unit.lowered.instructions.len(),
        unit.lowered.guards.len(),
        unit.lowered.exits.len(),
    );

    for instruction in &unit.lowered.instructions {
        eprintln!(
            "  step pc={} opcode={:?} line={:?} fallback={:?}",
            instruction.pc,
            instruction.opcode,
            instruction.line,
            instruction.fallback,
        );
    }

    for guard in &unit.lowered.guards {
        eprintln!(
            "  guard pc={} mode={:?} kind={:?} operands={:?} continue_when={} exit_snapshot={}",
            guard.pc,
            guard.mode,
            guard.kind,
            guard.operands,
            guard.continue_when,
            guard.exit_snapshot_index,
        );
    }

    for exit in &unit.lowered.exits {
        eprintln!(
            "  exit kind={:?} source_pc={} target_pc={} snapshot={} side_trace={:?} actions={:?}",
            exit.kind,
            exit.source_pc,
            exit.target_pc,
            exit.snapshot_index,
            exit.side_trace,
            exit.actions,
        );
    }

    match artifact {
        TraceArtifact::Compiled(compiled) => {
            for step in &compiled.steps {
                eprintln!(
                    "  compiled-step pc={} opcode={:?} guards={} loop_exit={}",
                    step.instruction.pc,
                    step.instruction.opcode,
                    step.guards.len(),
                    step.loop_exit.is_some(),
                );
            }
        }
        #[cfg(feature = "jit-cranelift")]
        TraceArtifact::Cranelift(artifact) => {
            for step in &artifact.compiled.steps {
                eprintln!(
                    "  compiled-step pc={} opcode={:?} guards={} loop_exit={}",
                    step.instruction.pc,
                    step.instruction.opcode,
                    step.guards.len(),
                    step.loop_exit.is_some(),
                );
            }
        }
        _ => {}
    }
}

fn trace_artifact_kind_name(artifact: &TraceArtifact) -> &'static str {
    match artifact {
        TraceArtifact::Replay(_) => "replay",
        TraceArtifact::Compiled(_) => "compiled",
        #[cfg(feature = "jit-cranelift")]
        TraceArtifact::Cranelift(_) => "cranelift",
        TraceArtifact::NativePlaceholder(_) => "native-placeholder",
    }
}

impl Default for JitRuntime {
    fn default() -> Self {
        Self {
            policy: JitPolicy::default(),
            chunk_states: HashMap::new(),
            traces: Vec::new(),
            trace_stats: HashMap::new(),
            side_traces: HashMap::new(),
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

    use crate::{Chunk, Instruction, LuaLanguageLevel, LuaVM, LuaValue, OpCode, SafeOption};

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
            start_pc: 1,
            current_pc: 1,
            base: 4,
            frame_depth: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            parent_side_trace: None,
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
            start_pc: 1,
            current_pc: 1,
            base: 4,
            frame_depth: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            parent_side_trace: None,
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
            start_pc: 1,
            current_pc: 1,
            base: 4,
            frame_depth: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            parent_side_trace: None,
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

    #[test]
    fn runtime_blacklists_trace_with_excessive_side_exits() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::Test, 0, 0, 0, false),
            Instruction::create_sj(OpCode::Jmp, 2),
            Instruction::create_abc(OpCode::Add, 1, 1, 1),
            Instruction::create_sj(OpCode::Jmp, -4),
        ];

        let mut runtime = JitRuntime::new();
        runtime.finish_recording(
            &chunk as *const Chunk as usize,
            chunk.code.len(),
            0,
            RecordingResult::Compiled(TraceId(7)),
        );

        for _ in 0..TRACE_DEMOTE_WARMUP_RUNS {
            runtime.report_trace_result(
                &chunk as *const Chunk as usize,
                chunk.code.len(),
                0,
                TraceId(7),
                TraceRunResult {
                    next_pc: 3,
                    outcome: TraceRunOutcome::SideExit,
                    exit: None,
                },
            );
        }

        let stats = runtime.trace_run_stats(TraceId(7)).expect("trace stats missing");
        assert_eq!(stats.executions, TRACE_DEMOTE_WARMUP_RUNS);
        assert_eq!(stats.side_exits, TRACE_DEMOTE_WARMUP_RUNS);
        match runtime.on_loop_backedge(&chunk as *const Chunk as usize, chunk.code.len(), 0) {
            HotLoopAction::None => {}
            other => panic!("expected blacklisted anchor, got {other:?}"),
        }
    }

    #[test]
    fn runtime_records_side_trace_from_hot_guard_exit() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::EqI, 0, 127, 0, false),
            Instruction::create_sj(OpCode::Jmp, 2),
            Instruction::create_abc(OpCode::AddI, 1, 1, 128),
            Instruction::create_sj(OpCode::Jmp, -4),
            Instruction::create_abc(OpCode::AddI, 1, 1, 126),
            Instruction::create_sj(OpCode::Jmp, -6),
        ];

        let mut runtime = JitRuntime::new();
        runtime.policy.hotloop_threshold = 1;
        let request = RecordingRequest {
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            start_pc: 0,
            current_pc: 0,
            base: 4,
            frame_depth: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            parent_side_trace: None,
        };

        let parent_trace = match runtime.try_start_recording(&chunk, request) {
            RecordingResult::Compiled(trace_id) => trace_id,
            RecordingResult::Abort(reason) => panic!("unexpected abort: {reason:?}"),
        };

        let run_result = TraceRunResult {
            next_pc: 4,
            outcome: TraceRunOutcome::SideExit,
            exit: Some(TraceExitSite {
                kind: TraceExitKind::GuardExit,
                source_pc: 0,
                target_pc: 4,
                snapshot_index: 1,
            }),
        };
        let (side_key, side_request) = runtime
            .note_trace_side_exit(parent_trace, run_result, 4, 1)
            .expect("expected hot side exit recording request");
        let result = runtime.try_start_recording(&chunk, side_request);
        runtime.finish_side_trace_recording(side_key, result);

        let side_trace_id = runtime.side_trace(side_key).expect("side trace missing");
        let side_plan = runtime.trace_plan(side_trace_id).expect("side trace plan missing");
        assert_eq!(side_plan.anchor_pc, 0);
        assert_eq!(side_plan.instructions.first().map(|instruction| instruction.pc), Some(4));
        assert_eq!(runtime.side_trace_stats(side_key), Some(SideTraceStats { exits: 1 }));
        let parent_plan = runtime.trace_plan(parent_trace).expect("parent trace plan missing");
        assert_eq!(
            parent_plan
                .exits
                .iter()
                .find(|exit| exit.snapshot_index == 1)
                .and_then(|exit| exit.side_trace),
            Some(side_trace_id)
        );
    }

    #[test]
    fn trace_tree_executes_linked_side_trace_inside_artifact() {
        let mut vm = LuaVM::new(SafeOption::default());
        vm.set_language_level(LuaLanguageLevel::LuaJIT);
        {
            let state = vm.main_state();
            state.grow_stack(8).expect("grow stack");
            state.set_top(8).expect("set top");
            state.stack_mut()[1] = LuaValue::boolean(false);
            state.stack_mut()[2] = LuaValue::integer(10);
        }

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::TestSet, 0, 1, 0, false),
            Instruction::create_sj(OpCode::Jmp, 2),
            Instruction::create_abc(OpCode::AddI, 2, 2, 128),
            Instruction::create_sj(OpCode::Jmp, -4),
            Instruction::create_abc(OpCode::AddI, 2, 2, 126),
            Instruction::create_sj(OpCode::Jmp, -6),
        ];

        let parent_trace = TraceId(100);
        let side_trace_id = TraceId(101);
        let parent_plan = TracePlan {
            id: parent_trace,
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 3,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction {
                    pc: 0,
                    opcode: OpCode::TestSet,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 2,
                    opcode: OpCode::AddI,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 3,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1, 2],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 4,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1, 2],
                },
            ],
            guards: vec![TraceGuard {
                pc: 0,
                mode: TraceGuardMode::Control,
                kind: TraceGuardKind::Falsey,
                operands: TraceGuardOperands::Register { reg: 1 },
                continue_when: false,
                exit_snapshot_index: 1,
            }],
            exits: vec![TraceExit {
                kind: TraceExitKind::GuardExit,
                source_pc: 0,
                target_pc: 4,
                snapshot_index: 1,
                side_trace: Some(side_trace_id),
                actions: vec![TraceExitAction::CopyReg { dst: 0, src: 1 }],
            }],
        };
        let side_plan = TracePlan {
            id: side_trace_id,
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 5,
            anchor_kind: TraceAnchorKind::SideExit,
            instructions: vec![
                TraceInstruction {
                    pc: 4,
                    opcode: OpCode::AddI,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 5,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![TraceSnapshot {
                kind: TraceSnapshotKind::Entry,
                pc: 4,
                resume_pc: 4,
                base: 0,
                frame_depth: 0,
                live_regs: vec![0, 1, 2],
            }],
            guards: vec![],
            exits: vec![],
        };

        {
            let runtime = vm.jit_runtime_mut();
            runtime.traces.push(Arc::new(TraceArtifact::Replay(parent_plan)));
            runtime.traces.push(Arc::new(TraceArtifact::Replay(side_plan)));
        }

        let parent_artifact = vm
            .jit_runtime()
            .trace_artifact(parent_trace)
            .cloned()
            .expect("parent artifact missing");
        let dispatch = {
            let state = vm.main_state();
            parent_artifact
                .execute_tree(
                    state,
                    &chunk,
                    0,
                    JitPolicy {
                        max_trace_replays: 1,
                        ..JitPolicy::default()
                    },
                )
                .expect("trace tree should execute")
        };

        assert_eq!(dispatch.trace_id, side_trace_id);
        assert_eq!(dispatch.run_result.next_pc, 0);
        assert_eq!(dispatch.run_result.outcome, TraceRunOutcome::Anchored);
        let state = vm.main_state();
        assert_eq!(state.stack()[0].as_boolean(), Some(false));
        assert_eq!(state.stack()[2].as_integer_strict(), Some(9));
    }

    #[cfg(feature = "jit-cranelift")]
    #[test]
    fn cranelift_trace_tree_executes_linked_side_trace_inside_artifact() {
        let mut vm = LuaVM::new(SafeOption::default());
        vm.set_language_level(LuaLanguageLevel::LuaJIT);
        {
            let state = vm.main_state();
            state.grow_stack(8).expect("grow stack");
            state.set_top(8).expect("set top");
            state.stack_mut()[1] = LuaValue::boolean(false);
            state.stack_mut()[2] = LuaValue::integer(10);
        }

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::TestSet, 0, 1, 0, false),
            Instruction::create_sj(OpCode::Jmp, 2),
            Instruction::create_abc(OpCode::AddI, 2, 2, 128),
            Instruction::create_sj(OpCode::Jmp, -4),
            Instruction::create_abc(OpCode::AddI, 2, 2, 126),
            Instruction::create_sj(OpCode::Jmp, -6),
        ];

        let parent_trace = TraceId(110);
        let side_trace_id = TraceId(111);
        let parent_plan = TracePlan {
            id: parent_trace,
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 3,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction {
                    pc: 0,
                    opcode: OpCode::TestSet,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 2,
                    opcode: OpCode::AddI,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 3,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1, 2],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 4,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1, 2],
                },
            ],
            guards: vec![TraceGuard {
                pc: 0,
                mode: TraceGuardMode::Control,
                kind: TraceGuardKind::Falsey,
                operands: TraceGuardOperands::Register { reg: 1 },
                continue_when: false,
                exit_snapshot_index: 1,
            }],
            exits: vec![TraceExit {
                kind: TraceExitKind::GuardExit,
                source_pc: 0,
                target_pc: 4,
                snapshot_index: 1,
                side_trace: Some(side_trace_id),
                actions: vec![TraceExitAction::CopyReg { dst: 0, src: 1 }],
            }],
        };
        let side_plan = TracePlan {
            id: side_trace_id,
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 5,
            anchor_kind: TraceAnchorKind::SideExit,
            instructions: vec![
                TraceInstruction {
                    pc: 4,
                    opcode: OpCode::AddI,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 5,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![TraceSnapshot {
                kind: TraceSnapshotKind::Entry,
                pc: 4,
                resume_pc: 4,
                base: 0,
                frame_depth: 0,
                live_regs: vec![0, 1, 2],
            }],
            guards: vec![],
            exits: vec![],
        };

        let parent_artifact = CraneliftTraceBackend
            .compile(&TraceCompilationUnit::new(parent_plan))
            .expect("compile should succeed")
            .expect("parent artifact should compile");

        {
            let runtime = vm.jit_runtime_mut();
            runtime.traces.push(Arc::new(parent_artifact));
            runtime.traces.push(Arc::new(TraceArtifact::Replay(side_plan)));
        }

        let parent_artifact = vm
            .jit_runtime()
            .trace_artifact(parent_trace)
            .cloned()
            .expect("parent artifact missing");
        let dispatch = {
            let state = vm.main_state();
            parent_artifact
                .execute_tree(
                    state,
                    &chunk,
                    0,
                    JitPolicy {
                        max_trace_replays: 1,
                        ..JitPolicy::default()
                    },
                )
                .expect("cranelift trace tree should execute")
        };

        assert_eq!(dispatch.trace_id, side_trace_id);
        assert_eq!(dispatch.run_result.next_pc, 0);
        assert_eq!(dispatch.run_result.outcome, TraceRunOutcome::Anchored);
        let state = vm.main_state();
        assert_eq!(state.stack()[0].as_boolean(), Some(false));
        assert_eq!(state.stack()[2].as_integer_strict(), Some(9));
    }
}
