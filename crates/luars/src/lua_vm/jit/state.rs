use ahash::AHashMap;
use std::collections::hash_map::Entry;
use std::fmt::Write;

use crate::lua_value::LuaProto;
use crate::OpCode;

use super::backend::{
    BackendCompileOutcome, CompiledTrace, CompiledTraceExecutor, NullTraceBackend, TraceBackend,
};
use super::helper_plan::{HelperPlan, HelperPlanDispatchSummary, HelperPlanStep};
use super::hotcount::tick_hotcount;
use super::ir::TraceIr;
use super::trace_recorder::{TraceAbortReason, TraceArtifact, TraceRecorder};

const OPCODE_COUNT: usize = OpCode::ExtraArg as usize + 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitAbortCounters {
    pub empty_loop_body: u32,
    pub pc_out_of_bounds: u32,
    pub unsupported_opcode: u32,
    pub missing_branch_after_guard: u32,
    pub forward_jump: u32,
    pub backedge_mismatch: u32,
    pub trace_too_long: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TraceKey {
    chunk_addr: usize,
    pc: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TraceStatus {
    Counting { hits: u16 },
    Recording { attempts: u8 },
    Recorded { instruction_count: u16 },
    #[allow(dead_code)]
    Compiled,
    Redirected { root_pc: u32 },
    Blacklisted {
        attempts: u8,
        reason: TraceAbortReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TraceInfo {
    status: TraceStatus,
    artifact: Option<TraceArtifact>,
    ir: Option<TraceIr>,
    helper_plan: Option<HelperPlan>,
    compiled_trace: Option<CompiledTrace>,
    enterable: bool,
}

impl TraceInfo {
    fn new() -> Self {
        Self {
            status: TraceStatus::Counting { hits: 0 },
            artifact: None,
            ir: None,
            helper_plan: None,
            compiled_trace: None,
            enterable: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitCounters {
    pub hot_headers: u32,
    pub record_attempts: u32,
    pub recorded_traces: u32,
    pub record_aborts: u32,
    pub blacklist_hits: u32,
    pub trace_enter_checks: u32,
    pub trace_enter_hits: u32,
    pub helper_plan_dispatches: u32,
    pub helper_plan_steps: u32,
    pub helper_plan_guards: u32,
    pub helper_plan_calls: u32,
    pub helper_plan_metamethods: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitStatsSnapshot {
    pub counters: JitCounters,
    pub aborts: JitAbortCounters,
    pub top_unsupported_opcode: Option<(OpCode, u32)>,
    pub trace_count: u32,
    pub recorded_count: u32,
    pub compiled_count: u32,
    pub blacklisted_count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TraceStepCounts {
    load_move: u16,
    upvalue_access: u16,
    upvalue_mutation: u16,
    cleanup: u16,
    table_access: u16,
    arithmetic: u16,
    call: u16,
    metamethod_fallback: u16,
    closure_creation: u16,
    loop_prep: u16,
    guard: u16,
    branch: u16,
    loop_backedge: u16,
}

pub(crate) struct JitState {
    traces: AHashMap<TraceKey, TraceInfo>,
    counters: JitCounters,
    backend: NullTraceBackend,
}

impl Default for JitState {
    fn default() -> Self {
        Self {
            traces: AHashMap::default(),
            counters: JitCounters::default(),
            backend: NullTraceBackend,
        }
    }
}

impl JitState {
    pub(crate) fn stats_snapshot(&self) -> JitStatsSnapshot {
        let mut recorded_count = 0u32;
        let mut compiled_count = 0u32;
        let mut blacklisted_count = 0u32;
        let mut aborts = JitAbortCounters::default();
        let mut unsupported_opcodes = [0u32; OPCODE_COUNT];

        for trace in self.traces.values() {
            match trace.status {
                TraceStatus::Recorded { .. } => recorded_count = recorded_count.saturating_add(1),
                TraceStatus::Compiled => compiled_count = compiled_count.saturating_add(1),
                TraceStatus::Blacklisted { reason, .. } => {
                    blacklisted_count = blacklisted_count.saturating_add(1);
                    apply_abort_reason(&mut aborts, &mut unsupported_opcodes, reason);
                }
                TraceStatus::Counting { .. }
                | TraceStatus::Recording { .. }
                | TraceStatus::Redirected { .. } => {}
            }
        }

        JitStatsSnapshot {
            counters: self.counters,
            aborts,
            top_unsupported_opcode: top_unsupported_opcode(&unsupported_opcodes),
            trace_count: self.traces.len() as u32,
            recorded_count,
            compiled_count,
            blacklisted_count,
        }
    }

    pub(crate) fn trace_report(&self) -> String {
        let mut slots = self
            .traces
            .iter()
            .map(|(key, trace)| (key.chunk_addr, key.pc, trace))
            .collect::<Vec<_>>();
        slots.sort_by_key(|(chunk_addr, pc, _)| (*chunk_addr, *pc));

        let mut report = String::from("JIT Trace Slots:\n");
        for (chunk_addr, pc, trace) in slots {
            let status = format_trace_status(trace.status);
            let op_count = trace
                .artifact
                .as_ref()
                .map(|artifact| artifact.ops.len())
                .unwrap_or(0);
            let exit_count = trace
                .artifact
                .as_ref()
                .map(|artifact| artifact.exits.len())
                .unwrap_or(0);
            let executor = trace
                .compiled_trace
                .as_ref()
                .map(CompiledTrace::executor_family)
                .unwrap_or("none");
            let step_counts = trace
                .helper_plan
                .as_ref()
                .map(helper_plan_step_counts)
                .unwrap_or_default();
            let mut details = format_step_counts(step_counts);
            if details.is_empty() {
                details.push_str("none");
            }

            let _ = writeln!(
                report,
                "- chunk=0x{chunk_addr:x} pc={pc} status={status} executor={executor} ops={op_count} exits={exit_count} details={details}",
            );
        }

        report
    }

    #[cfg(test)]
    pub(crate) fn try_enter_trace(&mut self, chunk_ptr: *const LuaProto, pc: u32) -> bool {
        let key = TraceKey {
            chunk_addr: chunk_ptr as usize,
            pc,
        };

        self.counters.trace_enter_checks = self.counters.trace_enter_checks.saturating_add(1);

        let Some(trace) = self.traces.get(&key) else {
            return false;
        };

        if let Some(compiled_trace) = trace.compiled_trace.as_ref() {
            let executor = compiled_trace.executor();
            if !matches!(executor, CompiledTraceExecutor::SummaryOnly) {
                let dispatch_summary = compiled_trace.execute();
                self.counters.trace_enter_hits = self.counters.trace_enter_hits.saturating_add(1);
                self.apply_helper_plan_summary(dispatch_summary);
                return true;
            }
        }

        false
    }

    pub(crate) fn compiled_trace_executor_or_record(
        &mut self,
        chunk_ptr: *const LuaProto,
        pc: u32,
    ) -> Option<(CompiledTraceExecutor, HelperPlanDispatchSummary)> {
        let key = TraceKey {
            chunk_addr: chunk_ptr as usize,
            pc,
        };

        let mut should_record = false;
        match self.traces.entry(key) {
            Entry::Occupied(mut entry) => {
                let trace = entry.get_mut();
                if trace.enterable
                    && let Some(compiled_trace) = trace.compiled_trace.as_ref()
                {
                    let executor = compiled_trace.executor();
                    let summary = compiled_trace.summary();
                    return Some((executor, summary));
                }

                match &mut trace.status {
                    TraceStatus::Counting { hits } => should_record = tick_hotcount(&mut *hits),
                    TraceStatus::Recording { .. }
                    | TraceStatus::Recorded { .. }
                    | TraceStatus::Compiled
                    | TraceStatus::Redirected { .. } => {}
                    TraceStatus::Blacklisted { .. } => {
                        self.counters.blacklist_hits = self.counters.blacklist_hits.saturating_add(1);
                    }
                }
            }
            Entry::Vacant(entry) => {
                let trace = entry.insert(TraceInfo::new());
                if let TraceStatus::Counting { hits } = &mut trace.status {
                    should_record = tick_hotcount(&mut *hits);
                }
            }
        }

        if should_record {
            self.counters.hot_headers = self.counters.hot_headers.saturating_add(1);
            self.begin_recording(key, chunk_ptr);
        }

        None
    }

    pub(crate) fn record_batched_trace_execution(
        &mut self,
        checks: u32,
        hits: u32,
        summary: HelperPlanDispatchSummary,
    ) {
        self.counters.trace_enter_checks = self.counters.trace_enter_checks.saturating_add(checks);
        self.counters.trace_enter_hits = self.counters.trace_enter_hits.saturating_add(hits);
        self.apply_helper_plan_summary_n(summary, hits);
    }

    pub(crate) fn record_loop_backedge(&mut self, chunk_ptr: *const LuaProto, pc: u32) {
        let key = TraceKey {
            chunk_addr: chunk_ptr as usize,
            pc,
        };

        let should_record = {
            let trace = self.traces.entry(key).or_insert_with(TraceInfo::new);
            match &mut trace.status {
                TraceStatus::Counting { hits } => tick_hotcount(hits),
                TraceStatus::Recording { .. }
                | TraceStatus::Recorded { .. }
                | TraceStatus::Compiled
                | TraceStatus::Redirected { .. } => false,
                TraceStatus::Blacklisted { .. } => {
                    self.counters.blacklist_hits = self.counters.blacklist_hits.saturating_add(1);
                    return;
                }
            }
        };

        if should_record {
            self.counters.hot_headers = self.counters.hot_headers.saturating_add(1);
            self.begin_recording(key, chunk_ptr);
        }
    }

    pub(crate) fn blacklist_trace(
        &mut self,
        chunk_ptr: *const LuaProto,
        pc: u32,
        reason: TraceAbortReason,
    ) {
        let key = TraceKey {
            chunk_addr: chunk_ptr as usize,
            pc,
        };

        let attempts = self
            .traces
            .get(&key)
            .map(|trace| match trace.status {
                TraceStatus::Recording { attempts }
                | TraceStatus::Blacklisted { attempts, .. } => attempts.saturating_add(1),
                TraceStatus::Counting { .. }
                | TraceStatus::Recorded { .. }
                | TraceStatus::Compiled
                | TraceStatus::Redirected { .. } => 1,
            })
            .unwrap_or(1);
        self.abort_recording(key, attempts, reason);
    }

    #[cfg(test)]
    pub(crate) fn counters(&self) -> JitCounters {
        self.counters
    }

    #[cfg(test)]
    fn trace_status_for(&self, chunk_addr: usize, pc: u32) -> Option<TraceStatus> {
        self.traces.get(&TraceKey { chunk_addr, pc }).map(|trace| trace.status)
    }

    #[cfg(test)]
    fn artifact_for(&self, chunk_addr: usize, pc: u32) -> Option<&TraceArtifact> {
        self.traces
            .get(&TraceKey { chunk_addr, pc })
            .and_then(|trace| trace.artifact.as_ref())
    }

    #[cfg(test)]
    fn ir_for(&self, chunk_addr: usize, pc: u32) -> Option<&TraceIr> {
        self.traces
            .get(&TraceKey { chunk_addr, pc })
            .and_then(|trace| trace.ir.as_ref())
    }

    #[cfg(test)]
    fn helper_plan_for(&self, chunk_addr: usize, pc: u32) -> Option<&HelperPlan> {
        self.traces
            .get(&TraceKey { chunk_addr, pc })
            .and_then(|trace| trace.helper_plan.as_ref())
    }

    #[cfg(test)]
    fn compiled_trace_for(&self, chunk_addr: usize, pc: u32) -> Option<&CompiledTrace> {
        self.traces
            .get(&TraceKey { chunk_addr, pc })
            .and_then(|trace| trace.compiled_trace.as_ref())
    }

    fn begin_recording(&mut self, key: TraceKey, chunk_ptr: *const LuaProto) {
        self.counters.record_attempts = self.counters.record_attempts.saturating_add(1);

        let attempts = if let Some(trace) = self.traces.get_mut(&key) {
            let attempts = match trace.status {
                TraceStatus::Recording { attempts } => attempts.saturating_add(1),
                TraceStatus::Blacklisted { attempts, .. } => attempts.saturating_add(1),
                TraceStatus::Counting { .. }
                | TraceStatus::Recorded { .. }
                | TraceStatus::Compiled
                | TraceStatus::Redirected { .. } => 1,
            };
            trace.status = TraceStatus::Recording { attempts };
            trace.artifact = None;
            trace.ir = None;
            trace.helper_plan = None;
            trace.compiled_trace = None;
            trace.enterable = false;
            attempts
        } else {
            return;
        };

        match TraceRecorder::record_root(chunk_ptr, key.pc) {
            Ok(artifact) => {
                let storage_key = TraceKey {
                    chunk_addr: key.chunk_addr,
                    pc: artifact.seed.start_pc,
                };
                let ir = TraceIr::lower(&artifact);
                let helper_plan = HelperPlan::lower(&ir);
                let backend_outcome = self.backend.compile(&artifact, &ir, &helper_plan);
                self.counters.recorded_traces = self.counters.recorded_traces.saturating_add(1);
                if storage_key != key {
                    if let Some(trace) = self.traces.get_mut(&key) {
                        trace.status = TraceStatus::Redirected {
                            root_pc: storage_key.pc,
                        };
                        trace.artifact = None;
                        trace.ir = None;
                        trace.helper_plan = None;
                        trace.compiled_trace = None;
                        trace.enterable = false;
                    }
                }
                if let Some(trace) = self.traces.get_mut(&storage_key) {
                    let compiled_trace = match backend_outcome {
                        BackendCompileOutcome::Compiled(compiled_trace) => {
                            trace.status = TraceStatus::Compiled;
                            Some(compiled_trace)
                        }
                        BackendCompileOutcome::NotYetSupported => {
                            trace.status = TraceStatus::Recorded {
                                instruction_count: artifact.ops.len() as u16,
                            };
                            None
                        }
                    };
                    trace.artifact = Some(artifact);
                    trace.ir = Some(ir);
                    trace.helper_plan = Some(helper_plan);
                    trace.enterable = compiled_trace
                        .as_ref()
                        .map(|compiled_trace| {
                            !matches!(compiled_trace.executor(), CompiledTraceExecutor::SummaryOnly)
                        })
                        .unwrap_or(false);
                    trace.compiled_trace = compiled_trace;
                } else {
                    let mut trace = TraceInfo::new();
                    let compiled_trace = match backend_outcome {
                        BackendCompileOutcome::Compiled(compiled_trace) => {
                            trace.status = TraceStatus::Compiled;
                            Some(compiled_trace)
                        }
                        BackendCompileOutcome::NotYetSupported => {
                            trace.status = TraceStatus::Recorded {
                                instruction_count: artifact.ops.len() as u16,
                            };
                            None
                        }
                    };
                    trace.artifact = Some(artifact);
                    trace.ir = Some(ir);
                    trace.helper_plan = Some(helper_plan);
                    trace.enterable = compiled_trace
                        .as_ref()
                        .map(|compiled_trace| {
                            !matches!(compiled_trace.executor(), CompiledTraceExecutor::SummaryOnly)
                        })
                        .unwrap_or(false);
                    trace.compiled_trace = compiled_trace;
                    self.traces.insert(storage_key, trace);
                }
            }
            Err(reason) => self.abort_recording(key, attempts, reason),
        }
    }

    fn abort_recording(&mut self, key: TraceKey, attempts: u8, reason: TraceAbortReason) {
        self.counters.record_aborts = self.counters.record_aborts.saturating_add(1);
        if let Some(trace) = self.traces.get_mut(&key) {
            trace.status = TraceStatus::Blacklisted { attempts, reason };
            trace.artifact = None;
            trace.ir = None;
            trace.helper_plan = None;
            trace.compiled_trace = None;
            trace.enterable = false;
        }
    }

    #[cfg(test)]
    fn apply_helper_plan_summary(&mut self, summary: HelperPlanDispatchSummary) {
        self.apply_helper_plan_summary_n(summary, 1);
    }

    fn apply_helper_plan_summary_n(&mut self, summary: HelperPlanDispatchSummary, count: u32) {
        if summary.steps_executed == 0 {
            return;
        }

        if count == 0 {
            return;
        }

        self.counters.helper_plan_dispatches =
            self.counters.helper_plan_dispatches.saturating_add(count);
        self.counters.helper_plan_steps = self
            .counters
            .helper_plan_steps
            .saturating_add(summary.steps_executed.saturating_mul(count));
        self.counters.helper_plan_guards = self
            .counters
            .helper_plan_guards
            .saturating_add(summary.guards_observed.saturating_mul(count));
        self.counters.helper_plan_calls = self
            .counters
            .helper_plan_calls
            .saturating_add(summary.call_steps.saturating_mul(count));
        self.counters.helper_plan_metamethods = self
            .counters
            .helper_plan_metamethods
            .saturating_add(summary.metamethod_steps.saturating_mul(count));
    }
}

fn apply_abort_reason(
    aborts: &mut JitAbortCounters,
    unsupported_opcodes: &mut [u32; OPCODE_COUNT],
    reason: TraceAbortReason,
) {
    match reason {
        TraceAbortReason::EmptyLoopBody => {
            aborts.empty_loop_body = aborts.empty_loop_body.saturating_add(1);
        }
        TraceAbortReason::PcOutOfBounds => {
            aborts.pc_out_of_bounds = aborts.pc_out_of_bounds.saturating_add(1);
        }
        TraceAbortReason::UnsupportedOpcode(opcode) => {
            aborts.unsupported_opcode = aborts.unsupported_opcode.saturating_add(1);
            unsupported_opcodes[opcode as usize] =
                unsupported_opcodes[opcode as usize].saturating_add(1);
        }
        TraceAbortReason::MissingBranchAfterGuard => {
            aborts.missing_branch_after_guard = aborts.missing_branch_after_guard.saturating_add(1);
        }
        TraceAbortReason::ForwardJump => {
            aborts.forward_jump = aborts.forward_jump.saturating_add(1);
        }
        TraceAbortReason::BackedgeMismatch { .. } => {
            aborts.backedge_mismatch = aborts.backedge_mismatch.saturating_add(1);
        }
        TraceAbortReason::TraceTooLong => {
            aborts.trace_too_long = aborts.trace_too_long.saturating_add(1);
        }
        TraceAbortReason::RuntimeGuardRejected => {}
    }
}

fn top_unsupported_opcode(counts: &[u32; OPCODE_COUNT]) -> Option<(OpCode, u32)> {
    let mut best: Option<(OpCode, u32)> = None;

    for (opcode_idx, count) in counts.iter().copied().enumerate() {
        if count == 0 {
            continue;
        }

        let opcode = OpCode::from_u8(opcode_idx as u8);
        match best {
            Some((_, best_count)) if best_count >= count => {}
            _ => best = Some((opcode, count)),
        }
    }

    best
}

fn helper_plan_step_counts(plan: &HelperPlan) -> TraceStepCounts {
    let mut counts = TraceStepCounts::default();

    for step in &plan.steps {
        match step {
            HelperPlanStep::LoadMove { .. } => counts.load_move = counts.load_move.saturating_add(1),
            HelperPlanStep::UpvalueAccess { .. } => {
                counts.upvalue_access = counts.upvalue_access.saturating_add(1)
            }
            HelperPlanStep::UpvalueMutation { .. } => {
                counts.upvalue_mutation = counts.upvalue_mutation.saturating_add(1)
            }
            HelperPlanStep::Cleanup { .. } => counts.cleanup = counts.cleanup.saturating_add(1),
            HelperPlanStep::TableAccess { .. } => {
                counts.table_access = counts.table_access.saturating_add(1)
            }
            HelperPlanStep::Arithmetic { .. } => {
                counts.arithmetic = counts.arithmetic.saturating_add(1)
            }
            HelperPlanStep::Call { .. } => counts.call = counts.call.saturating_add(1),
            HelperPlanStep::MetamethodFallback { .. } => {
                counts.metamethod_fallback = counts.metamethod_fallback.saturating_add(1)
            }
            HelperPlanStep::ClosureCreation { .. } => {
                counts.closure_creation = counts.closure_creation.saturating_add(1)
            }
            HelperPlanStep::LoopPrep { .. } => counts.loop_prep = counts.loop_prep.saturating_add(1),
            HelperPlanStep::Guard { .. } => counts.guard = counts.guard.saturating_add(1),
            HelperPlanStep::Branch { .. } => counts.branch = counts.branch.saturating_add(1),
            HelperPlanStep::LoopBackedge { .. } => {
                counts.loop_backedge = counts.loop_backedge.saturating_add(1)
            }
        }
    }

    counts
}

fn format_trace_status(status: TraceStatus) -> String {
    match status {
        TraceStatus::Counting { hits } => format!("Counting(hits={hits})"),
        TraceStatus::Recording { attempts } => format!("Recording(attempts={attempts})"),
        TraceStatus::Recorded { instruction_count } => {
            format!("Recorded(instr={instruction_count})")
        }
        TraceStatus::Compiled => "Compiled".to_string(),
        TraceStatus::Redirected { root_pc } => format!("Redirected(root_pc={root_pc})"),
        TraceStatus::Blacklisted { attempts, reason } => {
            format!("Blacklisted(attempts={attempts}, reason={reason:?})")
        }
    }
}

fn format_step_counts(counts: TraceStepCounts) -> String {
    let mut parts = Vec::new();

    push_step_count(&mut parts, "load", counts.load_move);
    push_step_count(&mut parts, "upget", counts.upvalue_access);
    push_step_count(&mut parts, "upset", counts.upvalue_mutation);
    push_step_count(&mut parts, "cleanup", counts.cleanup);
    push_step_count(&mut parts, "table", counts.table_access);
    push_step_count(&mut parts, "arith", counts.arithmetic);
    push_step_count(&mut parts, "call", counts.call);
    push_step_count(&mut parts, "meta", counts.metamethod_fallback);
    push_step_count(&mut parts, "closure", counts.closure_creation);
    push_step_count(&mut parts, "prep", counts.loop_prep);
    push_step_count(&mut parts, "guard", counts.guard);
    push_step_count(&mut parts, "branch", counts.branch);
    push_step_count(&mut parts, "backedge", counts.loop_backedge);

    parts.join(",")
}

fn push_step_count(parts: &mut Vec<String>, name: &str, count: u16) {
    if count != 0 {
        parts.push(format!("{name}={count}"));
    }
}

#[cfg(test)]
mod tests {
    use super::{JitAbortCounters, JitCounters, JitState, TraceStatus};
    use crate::lua_value::LuaProto;
    use crate::lua_vm::jit::hotcount::HOT_LOOP_THRESHOLD;
    use crate::lua_vm::jit::trace_recorder::TraceAbortReason;
    use crate::{Instruction, OpCode};

    #[test]
    fn hot_trace_blacklists_after_first_record_attempt() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abck(OpCode::TailCall, 0, 1, 1, false));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert_eq!(
            jit.trace_status_for(chunk_ptr as usize, 0),
            Some(TraceStatus::Blacklisted {
                attempts: 1,
                reason: TraceAbortReason::UnsupportedOpcode(OpCode::TailCall),
            })
        );
        assert_eq!(
            jit.counters(),
            JitCounters {
                hot_headers: 1,
                record_attempts: 1,
                recorded_traces: 0,
                record_aborts: 1,
                blacklist_hits: 0,
                trace_enter_checks: 0,
                trace_enter_hits: 0,
                helper_plan_dispatches: 0,
                helper_plan_steps: 0,
                helper_plan_guards: 0,
                helper_plan_calls: 0,
                helper_plan_metamethods: 0,
            }
        );
    }

    #[test]
    fn blacklisted_trace_stops_recounting_and_counts_hits() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abck(OpCode::TailCall, 0, 1, 1, false));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }
        for _ in 0..(HOT_LOOP_THRESHOLD * 2) {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert_eq!(
            jit.trace_status_for(chunk_ptr as usize, 0),
            Some(TraceStatus::Blacklisted {
                attempts: 1,
                reason: TraceAbortReason::UnsupportedOpcode(OpCode::TailCall),
            })
        );
        assert_eq!(jit.counters().blacklist_hits, (HOT_LOOP_THRESHOLD * 2) as u32);
    }

    #[test]
    fn counting_state_stays_cold_before_threshold() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..(HOT_LOOP_THRESHOLD - 1) {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert_eq!(
            jit.trace_status_for(chunk_ptr as usize, 0),
            Some(TraceStatus::Counting {
                hits: HOT_LOOP_THRESHOLD - 1,
            })
        );
    }

    #[test]
    fn supported_trace_is_recorded_and_cached() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert_eq!(jit.trace_status_for(chunk_ptr as usize, 0), Some(TraceStatus::Compiled));
        let artifact = jit.artifact_for(chunk_ptr as usize, 0).unwrap();
        let ir = jit.ir_for(chunk_ptr as usize, 0).unwrap();
        let helper_plan = jit.helper_plan_for(chunk_ptr as usize, 0).unwrap();
        assert_eq!(artifact.ops.len(), 2);
        assert!(artifact.exits.is_empty());
        assert_eq!(artifact.loop_tail_pc, 1);
        assert_eq!(ir.insts.len(), 2);
        assert_eq!(ir.loop_tail_pc, 1);
        assert_eq!(helper_plan.steps.len(), 2);
        assert_eq!(jit.counters().recorded_traces, 1);
        assert_eq!(jit.counters().record_aborts, 0);
    }

    #[test]
    fn guarded_trace_is_compiled_with_exit_metadata() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abck(OpCode::Test, 0, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_abck(OpCode::Add, 0, 0, 1, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert_eq!(
            jit.trace_status_for(chunk_ptr as usize, 0),
            Some(TraceStatus::Compiled)
        );
        let artifact = jit.artifact_for(chunk_ptr as usize, 0).unwrap();
        let ir = jit.ir_for(chunk_ptr as usize, 0).unwrap();
        let helper_plan = jit.helper_plan_for(chunk_ptr as usize, 0).unwrap();
        assert_eq!(artifact.exits.len(), 1);
        assert_eq!(artifact.loop_tail_pc, 4);
        assert_eq!(ir.guards.len(), 1);
        assert_eq!(ir.loop_tail_pc, 4);
        assert_eq!(helper_plan.guard_count, 1);
    }

    #[test]
    fn recorded_trace_entry_is_skipped() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::GetUpval, 0, 0, 0));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert!(!jit.try_enter_trace(chunk_ptr, 0));
        assert!(!jit.try_enter_trace(chunk_ptr, 1));
        assert_eq!(jit.counters().trace_enter_checks, 2);
        assert_eq!(jit.counters().trace_enter_hits, 0);
        assert_eq!(jit.counters().helper_plan_dispatches, 0);
        assert_eq!(jit.counters().helper_plan_steps, 0);
        assert_eq!(jit.counters().helper_plan_guards, 0);
        assert_eq!(jit.counters().helper_plan_calls, 0);
        assert_eq!(jit.counters().helper_plan_metamethods, 0);
    }

    #[test]
    fn guarded_trace_entry_is_skipped_without_specialized_executor() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::GetUpval, 0, 0, 0));
        chunk.code.push(Instruction::create_abck(OpCode::Test, 0, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk.code.push(Instruction::create_abck(OpCode::Add, 0, 0, 1, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert!(!jit.try_enter_trace(chunk_ptr, 0));
        assert_eq!(jit.counters().helper_plan_dispatches, 0);
        assert_eq!(jit.counters().helper_plan_steps, 0);
        assert_eq!(jit.counters().helper_plan_guards, 0);
        assert_eq!(jit.counters().helper_plan_calls, 0);
        assert_eq!(jit.counters().helper_plan_metamethods, 0);
    }

    #[test]
    fn summary_only_compiled_trace_entry_is_skipped() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abc(OpCode::Call, 0, 2, 2));
        chunk.code.push(Instruction::create_abck(OpCode::Add, 0, 1, 2, false));
        chunk.code.push(Instruction::create_abc(OpCode::MmBin, 1, 2, 0));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert!(!jit.try_enter_trace(chunk_ptr, 0));
        assert_eq!(jit.counters().trace_enter_hits, 0);
        assert_eq!(jit.counters().helper_plan_dispatches, 0);
        assert_eq!(jit.counters().helper_plan_calls, 0);
        assert_eq!(jit.counters().helper_plan_metamethods, 0);
    }

    #[test]
    fn helper_call_trace_is_marked_compiled() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abc(OpCode::Call, 0, 2, 2));
        chunk.code.push(Instruction::create_abck(OpCode::Add, 0, 1, 2, false));
        chunk.code.push(Instruction::create_abc(OpCode::MmBin, 1, 2, 0));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert_eq!(jit.trace_status_for(chunk_ptr as usize, 0), Some(TraceStatus::Compiled));
        let compiled_trace = jit.compiled_trace_for(chunk_ptr as usize, 0).unwrap();
        assert_eq!(compiled_trace.root_pc, 0);
        assert_eq!(compiled_trace.loop_tail_pc, 4);
    }

    #[test]
    fn snapshot_reports_trace_buckets() {
        let mut jit = JitState::default();
        let mut recorded_chunk = LuaProto::new();
        recorded_chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        recorded_chunk
            .code
            .push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        let recorded_ptr = &recorded_chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(recorded_ptr, 0);
        }

        let mut bad_chunk = LuaProto::new();
        bad_chunk
            .code
            .push(Instruction::create_abck(OpCode::TailCall, 0, 1, 1, false));
        let bad_ptr = &bad_chunk as *const LuaProto;
        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(bad_ptr, 0);
        }

        let mut mismatch_chunk = LuaProto::new();
        mismatch_chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        mismatch_chunk
            .code
            .push(Instruction::create_abx(OpCode::ForLoop, 0, 1));
        let mismatch_ptr = &mismatch_chunk as *const LuaProto;
        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(mismatch_ptr, 0);
        }

        let snapshot = jit.stats_snapshot();
        assert_eq!(snapshot.trace_count, 3);
        assert_eq!(snapshot.recorded_count, 0);
        assert_eq!(snapshot.compiled_count, 1);
        assert_eq!(snapshot.blacklisted_count, 2);
        assert_eq!(snapshot.counters.recorded_traces, 1);
        assert_eq!(snapshot.counters.record_aborts, 2);
        assert_eq!(
            snapshot.aborts,
            JitAbortCounters {
                empty_loop_body: 0,
                pc_out_of_bounds: 0,
                unsupported_opcode: 1,
                missing_branch_after_guard: 0,
                forward_jump: 0,
                backedge_mismatch: 1,
                trace_too_long: 0,
            }
        );
        assert_eq!(snapshot.top_unsupported_opcode, Some((OpCode::TailCall, 1)));
    }

    #[test]
    fn nested_loop_recording_redirects_to_inner_header() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abc(OpCode::Move, 1, 2, 0));
        chunk.code.push(Instruction::create_abc(OpCode::Move, 2, 3, 0));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert_eq!(
            jit.trace_status_for(chunk_ptr as usize, 0),
            Some(TraceStatus::Redirected { root_pc: 2 })
        );
        assert_eq!(jit.trace_status_for(chunk_ptr as usize, 2), Some(TraceStatus::Compiled));
    }

    #[test]
    fn trace_report_lists_slot_status_and_step_details() {
        let mut jit = JitState::default();
        let mut helper_chunk = LuaProto::new();
        helper_chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        helper_chunk.code.push(Instruction::create_abc(OpCode::Call, 0, 2, 2));
        helper_chunk.code.push(Instruction::create_abck(OpCode::Add, 0, 1, 2, false));
        helper_chunk.code.push(Instruction::create_abc(OpCode::MmBin, 1, 2, 0));
        helper_chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));
        let helper_ptr = &helper_chunk as *const LuaProto;

        let mut bad_chunk = LuaProto::new();
        bad_chunk
            .code
            .push(Instruction::create_abck(OpCode::TailCall, 0, 1, 1, false));
        let bad_ptr = &bad_chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(helper_ptr, 0);
            jit.record_loop_backedge(bad_ptr, 0);
        }

        let report = jit.trace_report();
        assert!(report.contains("status=Compiled"));
        assert!(report.contains("executor=SummaryOnly"));
        assert!(report.contains("details=load=1,arith=1,call=1,meta=1,backedge=1"));
        assert!(report.contains("status=Blacklisted(attempts=1, reason=UnsupportedOpcode(TailCall))"));
    }
}