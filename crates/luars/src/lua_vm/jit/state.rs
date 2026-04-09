use ahash::AHashMap;
use std::collections::hash_map::Entry;
use std::fmt::Write;

use crate::gc::UpvaluePtr;
use crate::lua_value::{LuaProto, LuaValue};
use crate::OpCode;

use super::backend::{
    BackendCompileOutcome, CompiledTrace, CompiledTraceExecution,
    NativeCompiledTrace, NativeLoweringProfile, NativeTraceBackend, TraceBackend,
};
use super::helper_plan::{HelperPlan, HelperPlanDispatchSummary, HelperPlanStep};
use super::hotcount::tick_hotcount;
use super::ir::{TraceIr, is_fused_arithmetic_metamethod_fallback};
use super::lowering::{
    DeoptRecovery, DeoptTarget, LoweredTrace, SsaMemoryEffectSummary,
    SsaTableIntOptimizationSummary, SsaValueSummary, ValueHintSummary,
};
use super::trace_recorder::{TraceAbortReason, TraceArtifact, TraceRecorder};

const OPCODE_COUNT: usize = OpCode::ExtraArg as usize + 1;
const HOT_EXIT_THRESHOLD: u16 = 10;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SideTraceKey {
    parent: TraceKey,
    exit_index: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TraceStatus {
    Counting { hits: u16 },
    Recording { attempts: u8 },
    Recorded { instruction_count: u16 },
    Lowered { instruction_count: u16 },
    Executable { instruction_count: u16 },
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
    lowered_trace: Option<LoweredTrace>,
    helper_plan: Option<HelperPlan>,
    compiled_trace: Option<CompiledTrace>,
    linked_ready_side_traces: AHashMap<u16, ReadySideTraceDispatch>,
}

impl TraceInfo {
    fn new() -> Self {
        Self {
            status: TraceStatus::Counting { hits: 0 },
            artifact: None,
            ir: None,
            lowered_trace: None,
            helper_plan: None,
            compiled_trace: None,
            linked_ready_side_traces: AHashMap::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SideTraceStatus {
    Counting { hits: u16 },
    Recording { attempts: u8 },
    Recorded { instruction_count: u16 },
    Lowered { instruction_count: u16 },
    Executable { instruction_count: u16 },
    Blacklisted { attempts: u8, reason: TraceAbortReason },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SideTraceInfo {
    status: SideTraceStatus,
    start_pc: u32,
    artifact: Option<TraceArtifact>,
    ir: Option<TraceIr>,
    lowered_trace: Option<LoweredTrace>,
    helper_plan: Option<HelperPlan>,
    compiled_trace: Option<CompiledTrace>,
}

impl SideTraceInfo {
    fn new(start_pc: u32) -> Self {
        Self {
            status: SideTraceStatus::Counting { hits: 0 },
            start_pc,
            artifact: None,
            ir: None,
            lowered_trace: None,
            helper_plan: None,
            compiled_trace: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExecutableTraceDispatch {
    pub start_pc: u32,
    pub loop_tail_pc: u32,
    pub execution: CompiledTraceExecution,
    pub summary: HelperPlanDispatchSummary,
    pub native_profile: Option<NativeLoweringProfile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeExecutableTraceDispatch {
    pub start_pc: u32,
    pub loop_tail_pc: u32,
    pub native: NativeCompiledTrace,
    pub summary: HelperPlanDispatchSummary,
    pub profile: NativeLoweringProfile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReadySideTraceDispatch {
    Executable(ExecutableTraceDispatch),
    Native(NativeExecutableTraceDispatch),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TraceExitDispatch {
    pub recovery: DeoptRecovery,
    pub ready_side_trace: Option<ReadySideTraceDispatch>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitCounters {
    pub hot_headers: u32,
    pub hot_exits: u32,
    pub record_attempts: u32,
    pub side_record_attempts: u32,
    pub recorded_traces: u32,
    pub recorded_side_traces: u32,
    pub record_aborts: u32,
    pub side_record_aborts: u32,
    pub blacklist_hits: u32,
    pub trace_enter_checks: u32,
    pub trace_enter_hits: u32,
    pub helper_plan_dispatches: u32,
    pub helper_plan_steps: u32,
    pub helper_plan_guards: u32,
    pub helper_plan_calls: u32,
    pub helper_plan_metamethods: u32,
    pub root_native_dispatches: u32,
    pub root_native_return_dispatches: u32,
    pub root_native_linear_int_for_dispatches: u32,
    pub root_native_linear_int_jmp_dispatches: u32,
    pub root_native_numeric_for_dispatches: u32,
    pub root_native_guarded_numeric_for_dispatches: u32,
    pub root_native_numeric_jmp_dispatches: u32,
    pub side_native_dispatches: u32,
    pub native_exit_index_resolve_attempts: u32,
    pub native_exit_index_resolve_hits: u32,
    pub native_redundant_side_exit_recoveries: u32,
    pub native_redundant_side_exit_fast_dispatches: u32,
    pub native_profile_guard_steps: u32,
    pub native_profile_linear_guards: u32,
    pub native_profile_numeric_int_compare_guards: u32,
    pub native_profile_numeric_reg_compare_guards: u32,
    pub native_profile_truthy_guards: u32,
    pub native_profile_arithmetic_helpers: u32,
    pub native_profile_table_helpers: u32,
    pub native_profile_upvalue_helpers: u32,
    pub native_profile_shift_helpers: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitStatsSnapshot {
    pub counters: JitCounters,
    pub aborts: JitAbortCounters,
    pub top_unsupported_opcode: Option<(OpCode, u32)>,
    pub trace_count: u32,
    pub side_trace_count: u32,
    pub recorded_count: u32,
    pub lowered_count: u32,
    pub executable_count: u32,
    pub blacklisted_count: u32,
    pub side_recorded_count: u32,
    pub side_lowered_count: u32,
    pub side_executable_count: u32,
    pub side_blacklisted_count: u32,
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
    side_traces: AHashMap<SideTraceKey, SideTraceInfo>,
    counters: JitCounters,
    backend: NativeTraceBackend,
}

impl Default for JitState {
    fn default() -> Self {
        Self {
            traces: AHashMap::default(),
            side_traces: AHashMap::default(),
            counters: JitCounters::default(),
            backend: NativeTraceBackend::default(),
        }
    }
}

impl JitState {
    pub(crate) fn stats_snapshot(&self) -> JitStatsSnapshot {
        let mut recorded_count = 0u32;
        let mut lowered_count = 0u32;
        let mut executable_count = 0u32;
        let mut blacklisted_count = 0u32;
        let mut side_recorded_count = 0u32;
        let mut side_lowered_count = 0u32;
        let mut side_executable_count = 0u32;
        let mut side_blacklisted_count = 0u32;
        let mut aborts = JitAbortCounters::default();
        let mut unsupported_opcodes = [0u32; OPCODE_COUNT];

        for trace in self.traces.values() {
            match trace.status {
                TraceStatus::Recorded { .. } => recorded_count = recorded_count.saturating_add(1),
                TraceStatus::Lowered { .. } => lowered_count = lowered_count.saturating_add(1),
                TraceStatus::Executable { .. } => {
                    executable_count = executable_count.saturating_add(1)
                }
                TraceStatus::Blacklisted { reason, .. } => {
                    blacklisted_count = blacklisted_count.saturating_add(1);
                    apply_abort_reason(&mut aborts, &mut unsupported_opcodes, reason);
                }
                TraceStatus::Counting { .. }
                | TraceStatus::Recording { .. }
                | TraceStatus::Redirected { .. } => {}
            }
        }

        for trace in self.side_traces.values() {
            match trace.status {
                SideTraceStatus::Recorded { .. } => {
                    side_recorded_count = side_recorded_count.saturating_add(1)
                }
                SideTraceStatus::Lowered { .. } => {
                    side_lowered_count = side_lowered_count.saturating_add(1)
                }
                SideTraceStatus::Executable { .. } => {
                    side_executable_count = side_executable_count.saturating_add(1)
                }
                SideTraceStatus::Blacklisted { reason, .. } => {
                    side_blacklisted_count = side_blacklisted_count.saturating_add(1);
                    apply_abort_reason(&mut aborts, &mut unsupported_opcodes, reason);
                }
                SideTraceStatus::Counting { .. } | SideTraceStatus::Recording { .. } => {}
            }
        }

        JitStatsSnapshot {
            counters: self.counters,
            aborts,
            top_unsupported_opcode: top_unsupported_opcode(&unsupported_opcodes),
            trace_count: self.traces.len() as u32,
            side_trace_count: self.side_traces.len() as u32,
            recorded_count,
            lowered_count,
            executable_count,
            blacklisted_count,
            side_recorded_count,
            side_lowered_count,
            side_executable_count,
            side_blacklisted_count,
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
                .ir
                .as_ref()
                .map(semantic_trace_instruction_count)
                .or_else(|| trace.artifact.as_ref().map(|artifact| artifact.ops.len()))
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
            let value_hints = trace
                .lowered_trace
                .as_ref()
                .map(LoweredTrace::root_value_hint_summary)
                .map(format_value_hint_summary)
                .unwrap_or_else(|| String::from("none"));
            let ssa = trace
                .lowered_trace
                .as_ref()
                .map(LoweredTrace::ssa_value_summary)
                .map(format_ssa_value_summary)
                .unwrap_or_else(|| String::from("none"));
            let ssa_mem = trace
                .lowered_trace
                .as_ref()
                .map(LoweredTrace::ssa_memory_effect_summary)
                .map(format_ssa_memory_effect_summary)
                .unwrap_or_else(|| String::from("none"));
            let ssa_ti_opt = trace
                .lowered_trace
                .as_ref()
                .map(LoweredTrace::ssa_table_int_optimization_summary)
                .map(format_ssa_table_int_optimization_summary)
                .unwrap_or_else(|| String::from("none"));

            let _ = writeln!(
                report,
                "- chunk=0x{chunk_addr:x} pc={pc} status={status} executor={executor} ops={op_count} exits={exit_count} details={details} value_hints={value_hints} ssa={ssa} ssa_mem={ssa_mem} ssa_ti_opt={ssa_ti_opt}",
            );
        }

        if !self.side_traces.is_empty() {
            report.push_str("JIT Side Trace Slots:\n");
            let mut side_slots = self
                .side_traces
                .iter()
                .map(|(key, trace)| (key.parent.chunk_addr, key.parent.pc, key.exit_index, trace))
                .collect::<Vec<_>>();
            side_slots.sort_by_key(|(chunk_addr, parent_pc, exit_index, _)| {
                (*chunk_addr, *parent_pc, *exit_index)
            });
            for (chunk_addr, parent_pc, exit_index, trace) in side_slots {
                let status = format_side_trace_status(trace.status);
                let executor = trace
                    .compiled_trace
                    .as_ref()
                    .map(CompiledTrace::executor_family)
                    .unwrap_or("none");
                let _ = writeln!(
                    report,
                    "- chunk=0x{chunk_addr:x} parent_pc={parent_pc} exit={exit_index} start_pc={} status={status} executor={executor}",
                    trace.start_pc,
                );
            }
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
            if compiled_trace.is_enterable() {
                let dispatch_summary = compiled_trace.execute();
                self.counters.trace_enter_hits = self.counters.trace_enter_hits.saturating_add(1);
                self.apply_helper_plan_summary(dispatch_summary);
                return true;
            }
        }

        false
    }

    pub(crate) fn executable_trace_dispatch_or_record(
        &mut self,
        chunk_ptr: *const LuaProto,
        pc: u32,
    ) -> Option<ExecutableTraceDispatch> {
        let key = TraceKey {
            chunk_addr: chunk_ptr as usize,
            pc,
        };

        let mut should_record = false;
        match self.traces.entry(key) {
            Entry::Occupied(mut entry) => {
                let trace = entry.get_mut();
                if let Some(compiled_trace) = trace.compiled_trace.as_ref()
                    && compiled_trace.is_enterable()
                {
                    return Some(ExecutableTraceDispatch {
                        start_pc: compiled_trace.root_pc,
                        loop_tail_pc: compiled_trace.loop_tail_pc,
                        execution: compiled_trace.execution(),
                        summary: compiled_trace.summary(),
                        native_profile: compiled_trace.native_profile(),
                    });
                }

                match &mut trace.status {
                    TraceStatus::Counting { hits } => should_record = tick_hotcount(&mut *hits),
                    TraceStatus::Recording { .. }
                    | TraceStatus::Recorded { .. }
                    | TraceStatus::Lowered { .. }
                    | TraceStatus::Executable { .. }
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

    pub(crate) fn record_root_dispatch(&mut self, execution: &CompiledTraceExecution) {
        match execution {
            CompiledTraceExecution::Native(native) => {
                self.counters.root_native_dispatches =
                    self.counters.root_native_dispatches.saturating_add(1);
                match native {
                    NativeCompiledTrace::Return { .. }
                    | NativeCompiledTrace::Return0 { .. }
                    | NativeCompiledTrace::Return1 { .. } => {
                        self.counters.root_native_return_dispatches = self
                            .counters
                            .root_native_return_dispatches
                            .saturating_add(1);
                    }
                    NativeCompiledTrace::LinearIntForLoop { .. } => {
                        self.counters.root_native_linear_int_for_dispatches = self
                            .counters
                            .root_native_linear_int_for_dispatches
                            .saturating_add(1);
                    }
                    NativeCompiledTrace::LinearIntJmpLoop { .. } => {
                        self.counters.root_native_linear_int_jmp_dispatches = self
                            .counters
                            .root_native_linear_int_jmp_dispatches
                            .saturating_add(1);
                    }
                    NativeCompiledTrace::NumericForLoop { .. } => {
                        self.counters.root_native_numeric_for_dispatches = self
                            .counters
                            .root_native_numeric_for_dispatches
                            .saturating_add(1);
                    }
                    NativeCompiledTrace::GuardedNumericForLoop { .. } => {
                        self.counters.root_native_guarded_numeric_for_dispatches = self
                            .counters
                            .root_native_guarded_numeric_for_dispatches
                            .saturating_add(1);
                    }
                    NativeCompiledTrace::NumericJmpLoop { .. } => {
                        self.counters.root_native_numeric_jmp_dispatches = self
                            .counters
                            .root_native_numeric_jmp_dispatches
                            .saturating_add(1);
                    }
                }
            }
            CompiledTraceExecution::LoweredOnly => {}
        }
    }

    pub(crate) fn record_root_native_profile(&mut self, profile: NativeLoweringProfile) {
        apply_native_lowering_profile(&mut self.counters, profile);
    }

    pub(crate) fn record_ready_side_dispatch(&mut self, dispatch: &ReadySideTraceDispatch) {
        match dispatch {
            ReadySideTraceDispatch::Native(native) => {
                self.counters.side_native_dispatches =
                    self.counters.side_native_dispatches.saturating_add(1);
                apply_native_lowering_profile(&mut self.counters, native.profile);
            }
            ReadySideTraceDispatch::Executable(_) => {}
        }
    }

    pub(crate) fn record_redundant_side_exit_recovery(&mut self) {
        self.counters.native_redundant_side_exit_recoveries = self
            .counters
            .native_redundant_side_exit_recoveries
            .saturating_add(1);
    }

    pub(crate) fn record_redundant_side_exit_fast_dispatch(&mut self) {
        self.counters.native_redundant_side_exit_fast_dispatches = self
            .counters
            .native_redundant_side_exit_fast_dispatches
            .saturating_add(1);
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
                | TraceStatus::Lowered { .. }
                | TraceStatus::Executable { .. }
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

    pub(crate) fn record_hot_exit(
        &mut self,
        chunk_ptr: *const LuaProto,
        parent_pc: u32,
        exit_index: u16,
        exit_pc: u32,
    ) {
        let key = SideTraceKey {
            parent: TraceKey {
                chunk_addr: chunk_ptr as usize,
                pc: parent_pc,
            },
            exit_index,
        };

        let should_record = {
            let trace = self
                .side_traces
                .entry(key)
                .or_insert_with(|| SideTraceInfo::new(exit_pc));
            trace.start_pc = exit_pc;
            match &mut trace.status {
                SideTraceStatus::Counting { hits } => {
                    *hits = hits.saturating_add(1);
                    *hits >= HOT_EXIT_THRESHOLD
                }
                SideTraceStatus::Recording { .. }
                | SideTraceStatus::Recorded { .. }
                | SideTraceStatus::Lowered { .. }
                | SideTraceStatus::Executable { .. } => false,
                SideTraceStatus::Blacklisted { .. } => {
                    self.counters.blacklist_hits = self.counters.blacklist_hits.saturating_add(1);
                    return;
                }
            }
        };

        if should_record {
            self.counters.hot_exits = self.counters.hot_exits.saturating_add(1);
            self.begin_side_recording(key, chunk_ptr, exit_pc);
        }
    }

    pub(crate) fn record_trace_exit(
        &mut self,
        chunk_ptr: *const LuaProto,
        parent_pc: u32,
        exit_pc: u32,
    ) -> Option<DeoptTarget> {
        let key = TraceKey {
            chunk_addr: chunk_ptr as usize,
            pc: parent_pc,
        };

        let deopt_target = self
            .traces
            .get(&key)
            .and_then(|trace| trace.lowered_trace.as_ref())
            .and_then(|lowered_trace| lowered_trace.deopt_target_for_exit_pc(exit_pc))?;

        self.record_hot_exit(
            chunk_ptr,
            parent_pc,
            deopt_target.exit_index,
            deopt_target.resume_pc,
        );

        Some(deopt_target)
    }

    pub(crate) unsafe fn resolve_trace_exit(
        &mut self,
        chunk_ptr: *const LuaProto,
        parent_pc: u32,
        exit_pc: u32,
        stack: *const LuaValue,
        base: usize,
        constants: &[LuaValue],
        upvalue_ptrs: *const UpvaluePtr,
    ) -> Option<TraceExitDispatch> {
        let key = TraceKey {
            chunk_addr: chunk_ptr as usize,
            pc: parent_pc,
        };

        let recovery = self
            .traces
            .get(&key)
            .and_then(|trace| trace.lowered_trace.as_ref())
            .and_then(|lowered_trace| unsafe {
                lowered_trace.recover_exit(exit_pc, stack, base, constants, upvalue_ptrs)
            })?;

        self.finish_resolved_trace_exit(chunk_ptr, parent_pc, recovery)
    }

    pub(crate) unsafe fn resolve_trace_exit_by_index(
        &mut self,
        chunk_ptr: *const LuaProto,
        parent_pc: u32,
        exit_index: u16,
        stack: *const LuaValue,
        base: usize,
        constants: &[LuaValue],
        upvalue_ptrs: *const UpvaluePtr,
    ) -> Option<TraceExitDispatch> {
        let key = TraceKey {
            chunk_addr: chunk_ptr as usize,
            pc: parent_pc,
        };

        self.counters.native_exit_index_resolve_attempts = self
            .counters
            .native_exit_index_resolve_attempts
            .saturating_add(1);

        let recovery = self
            .traces
            .get(&key)
            .and_then(|trace| trace.lowered_trace.as_ref())
            .and_then(|lowered_trace| unsafe {
                lowered_trace.recover_exit_by_index(exit_index, stack, base, constants, upvalue_ptrs)
            })?;

        self.counters.native_exit_index_resolve_hits = self
            .counters
            .native_exit_index_resolve_hits
            .saturating_add(1);

        self.finish_resolved_trace_exit(chunk_ptr, parent_pc, recovery)
    }

    fn finish_resolved_trace_exit(
        &mut self,
        chunk_ptr: *const LuaProto,
        parent_pc: u32,
        recovery: DeoptRecovery,
    ) -> Option<TraceExitDispatch> {

        self.record_hot_exit(
            chunk_ptr,
            parent_pc,
            recovery.target.exit_index,
            recovery.target.resume_pc,
        );

        let ready_side_trace = self.ready_side_trace_dispatch_for(
            chunk_ptr,
            parent_pc,
            recovery.target.exit_index,
        );

        Some(TraceExitDispatch {
            recovery,
            ready_side_trace,
        })
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
                | TraceStatus::Lowered { .. }
                | TraceStatus::Executable { .. }
                | TraceStatus::Redirected { .. } => 1,
            };
            trace.status = TraceStatus::Recording { attempts };
            trace.artifact = None;
            trace.ir = None;
            trace.lowered_trace = None;
            trace.helper_plan = None;
            trace.compiled_trace = None;
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
                let lowered_trace = LoweredTrace::lower(&artifact, &ir, &helper_plan);
                let backend_outcome = self.backend.compile(&artifact, &ir, &lowered_trace, &helper_plan);
                self.counters.recorded_traces = self.counters.recorded_traces.saturating_add(1);
                if storage_key != key {
                    if let Some(trace) = self.traces.get_mut(&key) {
                        trace.status = TraceStatus::Redirected {
                            root_pc: storage_key.pc,
                        };
                        trace.artifact = None;
                        trace.ir = None;
                        trace.lowered_trace = None;
                        trace.helper_plan = None;
                        trace.compiled_trace = None;
                        trace.linked_ready_side_traces.clear();
                    }
                }
                if let Some(trace) = self.traces.get_mut(&storage_key) {
                    let instruction_count = semantic_instruction_count(&ir);
                    let compiled_trace = match backend_outcome {
                        BackendCompileOutcome::Compiled(compiled_trace) => {
                            trace.status = status_for_compiled_trace(instruction_count, &compiled_trace);
                            Some(compiled_trace)
                        }
                        BackendCompileOutcome::NotYetSupported => {
                            trace.status = TraceStatus::Recorded {
                                instruction_count,
                            };
                            None
                        }
                    };
                    trace.artifact = Some(artifact);
                    trace.ir = Some(ir);
                    trace.lowered_trace = Some(lowered_trace);
                    trace.helper_plan = Some(helper_plan);
                    trace.compiled_trace = compiled_trace;
                    trace.linked_ready_side_traces.clear();
                } else {
                    let mut trace = TraceInfo::new();
                    let instruction_count = semantic_instruction_count(&ir);
                    let compiled_trace = match backend_outcome {
                        BackendCompileOutcome::Compiled(compiled_trace) => {
                            trace.status = status_for_compiled_trace(instruction_count, &compiled_trace);
                            Some(compiled_trace)
                        }
                        BackendCompileOutcome::NotYetSupported => {
                            trace.status = TraceStatus::Recorded {
                                instruction_count,
                            };
                            None
                        }
                    };
                    trace.artifact = Some(artifact);
                    trace.ir = Some(ir);
                    trace.lowered_trace = Some(lowered_trace);
                    trace.helper_plan = Some(helper_plan);
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
            trace.lowered_trace = None;
            trace.helper_plan = None;
            trace.compiled_trace = None;
            trace.linked_ready_side_traces.clear();
        }
    }

    fn begin_side_recording(&mut self, key: SideTraceKey, chunk_ptr: *const LuaProto, start_pc: u32) {
        self.counters.side_record_attempts = self.counters.side_record_attempts.saturating_add(1);

        let attempts = if let Some(trace) = self.side_traces.get_mut(&key) {
            let attempts = match trace.status {
                SideTraceStatus::Recording { attempts } => attempts.saturating_add(1),
                SideTraceStatus::Blacklisted { attempts, .. } => attempts.saturating_add(1),
                SideTraceStatus::Counting { .. }
                | SideTraceStatus::Recorded { .. }
                | SideTraceStatus::Lowered { .. }
                | SideTraceStatus::Executable { .. } => 1,
            };
            trace.status = SideTraceStatus::Recording { attempts };
            trace.start_pc = start_pc;
            trace.artifact = None;
            trace.ir = None;
            trace.lowered_trace = None;
            trace.helper_plan = None;
            trace.compiled_trace = None;
            self.clear_linked_ready_side_trace(key);
            attempts
        } else {
            return;
        };

        match TraceRecorder::record_root(chunk_ptr, start_pc) {
            Ok(artifact) => {
                let ir = TraceIr::lower(&artifact);
                let helper_plan = HelperPlan::lower(&ir);
                let lowered_trace = LoweredTrace::lower(&artifact, &ir, &helper_plan);
                let backend_outcome = self.backend.compile(&artifact, &ir, &lowered_trace, &helper_plan);
                let instruction_count = semantic_instruction_count(&ir);
                self.counters.recorded_side_traces =
                    self.counters.recorded_side_traces.saturating_add(1);
                if let Some(trace) = self.side_traces.get_mut(&key) {
                    let compiled_trace = match backend_outcome {
                        BackendCompileOutcome::Compiled(compiled_trace) => {
                            trace.status = side_status_for_compiled_trace(instruction_count, &compiled_trace);
                            Some(compiled_trace)
                        }
                        BackendCompileOutcome::NotYetSupported => {
                            trace.status = SideTraceStatus::Recorded {
                                instruction_count,
                            };
                            None
                        }
                    };
                    trace.artifact = Some(artifact);
                    trace.ir = Some(ir);
                    trace.lowered_trace = Some(lowered_trace);
                    trace.helper_plan = Some(helper_plan);
                    trace.compiled_trace = compiled_trace;
                }
                self.refresh_linked_ready_side_trace(key);
            }
            Err(reason) => self.abort_side_recording(key, attempts, reason),
        }
    }

    fn abort_side_recording(&mut self, key: SideTraceKey, attempts: u8, reason: TraceAbortReason) {
        self.counters.side_record_aborts = self.counters.side_record_aborts.saturating_add(1);
        if let Some(trace) = self.side_traces.get_mut(&key) {
            trace.status = SideTraceStatus::Blacklisted { attempts, reason };
            trace.artifact = None;
            trace.ir = None;
            trace.lowered_trace = None;
            trace.helper_plan = None;
            trace.compiled_trace = None;
        }
        self.clear_linked_ready_side_trace(key);
    }

    fn side_trace_dispatch_for(
        &self,
        chunk_ptr: *const LuaProto,
        parent_pc: u32,
        exit_index: u16,
    ) -> Option<ExecutableTraceDispatch> {
        let key = SideTraceKey {
            parent: TraceKey {
                chunk_addr: chunk_ptr as usize,
                pc: parent_pc,
            },
            exit_index,
        };
        let trace = self.side_traces.get(&key)?;
        if !matches!(trace.status, SideTraceStatus::Executable { .. }) {
            return None;
        }
        let compiled_trace = trace.compiled_trace.as_ref()?;
        if !compiled_trace.is_enterable() {
            return None;
        }
        Some(ExecutableTraceDispatch {
            start_pc: trace.start_pc,
            loop_tail_pc: compiled_trace.loop_tail_pc,
            execution: compiled_trace.execution(),
            summary: compiled_trace.summary(),
            native_profile: compiled_trace.native_profile(),
        })
    }

    fn ready_side_trace_dispatch_for(
        &self,
        chunk_ptr: *const LuaProto,
        parent_pc: u32,
        exit_index: u16,
    ) -> Option<ReadySideTraceDispatch> {
        let parent_key = TraceKey {
            chunk_addr: chunk_ptr as usize,
            pc: parent_pc,
        };
        if let Some(dispatch) = self
            .traces
            .get(&parent_key)
            .and_then(|trace| trace.linked_ready_side_traces.get(&exit_index))
        {
            return Some(dispatch.clone());
        }

        let dispatch = self.side_trace_dispatch_for(chunk_ptr, parent_pc, exit_index)?;
        match dispatch {
            ExecutableTraceDispatch {
                start_pc,
                loop_tail_pc,
                execution: CompiledTraceExecution::Native(native),
                summary,
                native_profile: Some(profile),
            } => Some(ReadySideTraceDispatch::Native(NativeExecutableTraceDispatch {
                start_pc,
                loop_tail_pc,
                native,
                summary,
                profile,
            })),
            dispatch => Some(ReadySideTraceDispatch::Executable(dispatch)),
        }
    }

    fn native_side_trace_dispatch_for(
        &self,
        chunk_ptr: *const LuaProto,
        parent_pc: u32,
        exit_index: u16,
    ) -> Option<NativeExecutableTraceDispatch> {
        match self.ready_side_trace_dispatch_for(chunk_ptr, parent_pc, exit_index)? {
            ReadySideTraceDispatch::Native(dispatch) => Some(dispatch),
            ReadySideTraceDispatch::Executable(_) => None,
        }
    }

    fn clear_linked_ready_side_trace(&mut self, key: SideTraceKey) {
        if let Some(parent_trace) = self.traces.get_mut(&key.parent) {
            parent_trace.linked_ready_side_traces.remove(&key.exit_index);
        }
    }

    fn refresh_linked_ready_side_trace(&mut self, key: SideTraceKey) {
        let dispatch = {
            let side_trace = match self.side_traces.get(&key) {
                Some(side_trace) => side_trace,
                None => return,
            };
            if !matches!(side_trace.status, SideTraceStatus::Executable { .. }) {
                None
            } else {
                let compiled_trace = match side_trace.compiled_trace.as_ref() {
                    Some(compiled_trace) if compiled_trace.is_enterable() => compiled_trace,
                    _ => return,
                };
                let dispatch = ExecutableTraceDispatch {
                    start_pc: side_trace.start_pc,
                    loop_tail_pc: compiled_trace.loop_tail_pc,
                    execution: compiled_trace.execution(),
                    summary: compiled_trace.summary(),
                    native_profile: compiled_trace.native_profile(),
                };
                Some(match dispatch {
                    ExecutableTraceDispatch {
                        start_pc,
                        loop_tail_pc,
                        execution: CompiledTraceExecution::Native(native),
                        summary,
                        native_profile: Some(profile),
                    } => ReadySideTraceDispatch::Native(NativeExecutableTraceDispatch {
                        start_pc,
                        loop_tail_pc,
                        native,
                        summary,
                        profile,
                    }),
                    dispatch => ReadySideTraceDispatch::Executable(dispatch),
                })
            }
        };

        let Some(parent_trace) = self.traces.get_mut(&key.parent) else {
            return;
        };

        match dispatch {
            Some(dispatch) => {
                parent_trace
                    .linked_ready_side_traces
                    .insert(key.exit_index, dispatch);
            }
            None => {
                parent_trace.linked_ready_side_traces.remove(&key.exit_index);
            }
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
        TraceStatus::Lowered { instruction_count } => {
            format!("Lowered(instr={instruction_count})")
        }
        TraceStatus::Executable { instruction_count } => {
            format!("Executable(instr={instruction_count})")
        }
        TraceStatus::Redirected { root_pc } => format!("Redirected(root_pc={root_pc})"),
        TraceStatus::Blacklisted { attempts, reason } => {
            format!("Blacklisted(attempts={attempts}, reason={reason:?})")
        }
    }
}

fn format_side_trace_status(status: SideTraceStatus) -> String {
    match status {
        SideTraceStatus::Counting { hits } => format!("Counting(hits={hits})"),
        SideTraceStatus::Recording { attempts } => format!("Recording(attempts={attempts})"),
        SideTraceStatus::Recorded { instruction_count } => {
            format!("Recorded(instr={instruction_count})")
        }
        SideTraceStatus::Lowered { instruction_count } => {
            format!("Lowered(instr={instruction_count})")
        }
        SideTraceStatus::Executable { instruction_count } => {
            format!("Executable(instr={instruction_count})")
        }
        SideTraceStatus::Blacklisted { attempts, reason } => {
            format!("Blacklisted(attempts={attempts}, reason={reason:?})")
        }
    }
}

fn status_for_compiled_trace(
    instruction_count: u16,
    compiled_trace: &CompiledTrace,
) -> TraceStatus {
    if !compiled_trace.is_enterable() {
        TraceStatus::Lowered { instruction_count }
    } else {
        TraceStatus::Executable { instruction_count }
    }
}

fn side_status_for_compiled_trace(
    instruction_count: u16,
    compiled_trace: &CompiledTrace,
) -> SideTraceStatus {
    if !compiled_trace.is_enterable() {
        SideTraceStatus::Lowered { instruction_count }
    } else {
        SideTraceStatus::Executable { instruction_count }
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

fn format_value_hint_summary(summary: ValueHintSummary) -> String {
    let mut parts = Vec::new();

    push_step_count(&mut parts, "int", summary.integer_count);
    push_step_count(&mut parts, "float", summary.float_count);
    push_step_count(&mut parts, "num", summary.numeric_count);
    push_step_count(&mut parts, "bool", summary.boolean_count);
    push_step_count(&mut parts, "table", summary.table_count);
    push_step_count(&mut parts, "closure", summary.closure_count);
    push_step_count(&mut parts, "unknown", summary.unknown_count);

    if parts.is_empty() {
        String::from("none")
    } else {
        parts.join(",")
    }
}

fn format_ssa_value_summary(summary: SsaValueSummary) -> String {
    let mut parts = Vec::new();

    push_step_count(&mut parts, "entry", summary.entry_count);
    push_step_count(&mut parts, "derived", summary.derived_count);
    push_step_count(&mut parts, "int", summary.integer_count);
    push_step_count(&mut parts, "float", summary.float_count);
    push_step_count(&mut parts, "num", summary.numeric_count);
    push_step_count(&mut parts, "bool", summary.boolean_count);
    push_step_count(&mut parts, "table", summary.table_count);
    push_step_count(&mut parts, "closure", summary.closure_count);
    push_step_count(&mut parts, "unknown", summary.unknown_count);

    if parts.is_empty() {
        String::from("none")
    } else {
        parts.join(",")
    }
}

fn format_ssa_memory_effect_summary(summary: SsaMemoryEffectSummary) -> String {
    let mut parts = Vec::new();

    push_step_count(&mut parts, "tread", summary.table_read_count);
    push_step_count(&mut parts, "twrite", summary.table_write_count);
    push_step_count(&mut parts, "tiread", summary.table_int_read_count);
    push_step_count(&mut parts, "tiwrite", summary.table_int_write_count);
    push_step_count(&mut parts, "upread", summary.upvalue_read_count);
    push_step_count(&mut parts, "upwrite", summary.upvalue_write_count);
    push_step_count(&mut parts, "call", summary.call_count);
    push_step_count(&mut parts, "meta", summary.metamethod_count);

    if parts.is_empty() {
        String::from("none")
    } else {
        parts.join(",")
    }
}

fn format_ssa_table_int_optimization_summary(summary: SsaTableIntOptimizationSummary) -> String {
    let mut parts = Vec::new();

    push_step_count(&mut parts, "forward", summary.forwardable_read_count);
    push_step_count(&mut parts, "dead", summary.dead_store_count);

    if parts.is_empty() {
        String::from("none")
    } else {
        parts.join(",")
    }
}

fn push_step_count(parts: &mut Vec<String>, name: &str, count: u16) {
    if count != 0 {
        parts.push(format!("{name}={count}"));
    }
}

fn semantic_trace_instruction_count(ir: &TraceIr) -> usize {
    ir.insts
        .iter()
        .enumerate()
        .filter(|(index, _)| !is_fused_arithmetic_metamethod_fallback(&ir.insts, *index))
        .count()
}

fn semantic_instruction_count(ir: &TraceIr) -> u16 {
    semantic_trace_instruction_count(ir).min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::{
        HOT_EXIT_THRESHOLD, JitAbortCounters, JitCounters, JitState,
        NativeExecutableTraceDispatch, ReadySideTraceDispatch, SideTraceKey, SideTraceStatus,
        TraceKey, TraceStatus,
    };
    use crate::lua_vm::jit::backend::NativeCompiledTrace;
    use crate::{LuaVM, SafeOption};
    use crate::LuaValue;
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
                hot_exits: 0,
                record_attempts: 1,
                side_record_attempts: 0,
                recorded_traces: 0,
                recorded_side_traces: 0,
                record_aborts: 1,
                side_record_aborts: 0,
                blacklist_hits: 0,
                trace_enter_checks: 0,
                trace_enter_hits: 0,
                helper_plan_dispatches: 0,
                helper_plan_steps: 0,
                helper_plan_guards: 0,
                helper_plan_calls: 0,
                helper_plan_metamethods: 0,
                root_native_dispatches: 0,
                root_native_return_dispatches: 0,
                root_native_linear_int_for_dispatches: 0,
                root_native_linear_int_jmp_dispatches: 0,
                root_native_numeric_for_dispatches: 0,
                root_native_guarded_numeric_for_dispatches: 0,
                root_native_numeric_jmp_dispatches: 0,
                side_native_dispatches: 0,
                native_exit_index_resolve_attempts: 0,
                native_exit_index_resolve_hits: 0,
                native_redundant_side_exit_recoveries: 0,
                native_redundant_side_exit_fast_dispatches: 0,
                native_profile_guard_steps: 0,
                native_profile_linear_guards: 0,
                native_profile_numeric_int_compare_guards: 0,
                native_profile_numeric_reg_compare_guards: 0,
                native_profile_truthy_guards: 0,
                native_profile_arithmetic_helpers: 0,
                native_profile_table_helpers: 0,
                native_profile_upvalue_helpers: 0,
                native_profile_shift_helpers: 0,
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

        assert_eq!(
            jit.trace_status_for(chunk_ptr as usize, 0),
            Some(TraceStatus::Executable {
                instruction_count: 2,
            })
        );
        let dispatch = jit.executable_trace_dispatch_or_record(chunk_ptr, 0).unwrap();
        let artifact = jit.artifact_for(chunk_ptr as usize, 0).unwrap();
        let ir = jit.ir_for(chunk_ptr as usize, 0).unwrap();
        let helper_plan = jit.helper_plan_for(chunk_ptr as usize, 0).unwrap();
        assert_eq!(artifact.ops.len(), 2);
        assert!(artifact.exits.is_empty());
        assert_eq!(artifact.loop_tail_pc, 1);
        assert_eq!(ir.insts.len(), 2);
        assert_eq!(ir.loop_tail_pc, 1);
        assert_eq!(helper_plan.steps.len(), 2);
        assert_eq!(dispatch.start_pc, 0);
        assert_eq!(dispatch.loop_tail_pc, 1);
        assert_eq!(jit.counters().recorded_traces, 1);
        assert_eq!(jit.counters().record_aborts, 0);
    }

    #[test]
    fn guarded_trace_is_recorded_with_exit_metadata() {
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
            Some(TraceStatus::Lowered {
                instruction_count: 4,
            })
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
    fn executable_upvalue_trace_entry_is_enterable() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::GetUpval, 0, 0, 0));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert!(jit.try_enter_trace(chunk_ptr, 0));
        assert!(!jit.try_enter_trace(chunk_ptr, 1));
        assert_eq!(jit.counters().trace_enter_checks, 2);
        assert_eq!(jit.counters().trace_enter_hits, 1);
        assert!(jit.counters().helper_plan_dispatches > 0);
        assert!(jit.counters().helper_plan_steps > 0);
        assert_eq!(jit.counters().helper_plan_guards, 0);
        assert_eq!(jit.counters().helper_plan_calls, 0);
        assert_eq!(jit.counters().helper_plan_metamethods, 0);
    }

    #[test]
    fn executable_numeric_trace_does_not_replay_consumed_metamethod_fallbacks() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.constants.push(LuaValue::integer(1));
        chunk.code.push(Instruction::create_abc(OpCode::AddK, 0, 0, 0));
        chunk.code.push(Instruction::create_abc(OpCode::MmBinK, 0, 0, 6));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 3));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        assert!(jit.try_enter_trace(chunk_ptr, 0));
        assert_eq!(jit.counters().trace_enter_hits, 1);
        assert_eq!(jit.counters().helper_plan_dispatches, 0);
        assert_eq!(jit.counters().helper_plan_steps, 0);
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
    fn helper_call_trace_is_marked_lowered() {
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

        assert_eq!(
            jit.trace_status_for(chunk_ptr as usize, 0),
            Some(TraceStatus::Lowered {
                instruction_count: 4,
            })
        );
        let compiled_trace = jit.compiled_trace_for(chunk_ptr as usize, 0).unwrap();
        assert_eq!(compiled_trace.root_pc, 0);
        assert_eq!(compiled_trace.loop_tail_pc, 4);
    }

    #[test]
    fn trace_retains_lowered_artifact() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }

        let lowered = jit
            .traces
            .get(&TraceKey {
                chunk_addr: chunk_ptr as usize,
                pc: 0,
            })
            .and_then(|trace| trace.lowered_trace.as_ref())
            .unwrap();
        assert_eq!(lowered.root_pc, 0);
        assert_eq!(lowered.snapshots.len(), 1);
    }

    #[test]
    fn hot_exit_records_side_trace_slot() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_EXIT_THRESHOLD {
            jit.record_hot_exit(chunk_ptr, 0, 0, 0);
        }

        let side = jit
            .side_traces
            .get(&SideTraceKey {
                parent: TraceKey {
                    chunk_addr: chunk_ptr as usize,
                    pc: 0,
                },
                exit_index: 0,
            })
            .unwrap();
        assert_eq!(jit.counters().hot_exits, 1);
        assert_eq!(jit.counters().recorded_side_traces, 1);
        assert!(matches!(
            side.status,
            SideTraceStatus::Executable {
                instruction_count: 2,
            }
        ));
        assert!(side.lowered_trace.is_some());
    }

    #[test]
    fn root_trace_exit_resolves_snapshot_and_starts_side_counting() {
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

        let deopt = jit.record_trace_exit(chunk_ptr, 0, 3).unwrap();
        assert_eq!(deopt.exit_index, 0);
        assert_eq!(deopt.snapshot_id, 1);
        assert_eq!(deopt.resume_pc, 3);

        let side = jit
            .side_traces
            .get(&SideTraceKey {
                parent: TraceKey {
                    chunk_addr: chunk_ptr as usize,
                    pc: 0,
                },
                exit_index: 0,
            })
            .unwrap();
        assert_eq!(side.start_pc, 3);
        assert!(matches!(side.status, SideTraceStatus::Counting { hits: 1 }));
    }

    #[test]
    fn resolved_trace_exit_returns_recovery_and_ready_side_trace() {
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
        for _ in 0..HOT_EXIT_THRESHOLD {
            jit.record_hot_exit(chunk_ptr, 0, 0, 3);
        }

        let stack = [LuaValue::integer(11), LuaValue::integer(22), LuaValue::integer(33)];
        let exit = unsafe {
            jit.resolve_trace_exit(
                chunk_ptr,
                0,
                3,
                stack.as_ptr(),
                0,
                &chunk.constants,
                std::ptr::null(),
            )
        }
        .unwrap();

        assert_eq!(exit.recovery.target.resume_pc, 3);
        assert!(exit
            .recovery
            .register_restores
            .iter()
            .any(|(reg, value)| *reg == 0 && *value == LuaValue::integer(22)));
        assert!(jit
            .side_traces
            .contains_key(&SideTraceKey {
                parent: TraceKey {
                    chunk_addr: chunk_ptr as usize,
                    pc: 0,
                },
                exit_index: 0,
            }));
    }

    #[test]
    fn executable_side_trace_produces_dispatch_plan() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_EXIT_THRESHOLD {
            jit.record_hot_exit(chunk_ptr, 0, 0, 0);
        }

        let dispatch = jit.side_trace_dispatch_for(chunk_ptr, 0, 0);
        assert!(dispatch.is_some());
        assert_eq!(dispatch.unwrap().loop_tail_pc, 1);
    }

    #[test]
    fn native_executable_side_trace_produces_native_dispatch_plan() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_EXIT_THRESHOLD {
            jit.record_hot_exit(chunk_ptr, 0, 0, 0);
        }

        let dispatch = jit.native_side_trace_dispatch_for(chunk_ptr, 0, 0);
        assert!(dispatch.is_some());
        let dispatch = dispatch.unwrap();
        assert_eq!(dispatch.start_pc, 0);
        assert_eq!(dispatch.loop_tail_pc, 1);
        assert!(matches!(
            dispatch.native,
            NativeCompiledTrace::LinearIntForLoop { .. }
                | NativeCompiledTrace::NumericForLoop { .. }
        ));

        let ready_dispatch = jit.ready_side_trace_dispatch_for(chunk_ptr, 0, 0);
        assert!(matches!(
            ready_dispatch,
            Some(ReadySideTraceDispatch::Native(NativeExecutableTraceDispatch {
                start_pc: 0,
                loop_tail_pc: 1,
                native: NativeCompiledTrace::LinearIntForLoop { .. }
                    | NativeCompiledTrace::NumericForLoop { .. },
                ..
            }))
        ));
    }

    #[test]
    fn ready_side_trace_dispatch_is_cached_on_parent_trace() {
        let mut jit = JitState::default();
        let mut chunk = LuaProto::new();
        chunk.code.push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk.code.push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        let chunk_ptr = &chunk as *const LuaProto;

        for _ in 0..HOT_LOOP_THRESHOLD {
            jit.record_loop_backedge(chunk_ptr, 0);
        }
        for _ in 0..HOT_EXIT_THRESHOLD {
            jit.record_hot_exit(chunk_ptr, 0, 0, 0);
        }

        let parent = jit.traces.get(&TraceKey {
            chunk_addr: chunk_ptr as usize,
            pc: 0,
        });
        assert!(matches!(
            parent.and_then(|trace| trace.linked_ready_side_traces.get(&0)),
            Some(ReadySideTraceDispatch::Native(NativeExecutableTraceDispatch {
                start_pc: 0,
                loop_tail_pc: 1,
                native: NativeCompiledTrace::LinearIntForLoop { .. }
                    | NativeCompiledTrace::NumericForLoop { .. },
                ..
            }))
        ));
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
        assert_eq!(snapshot.side_trace_count, 0);
        assert_eq!(snapshot.recorded_count, 0);
        assert_eq!(snapshot.lowered_count, 0);
        assert_eq!(snapshot.executable_count, 1);
        assert_eq!(snapshot.blacklisted_count, 2);
        assert_eq!(snapshot.side_recorded_count, 0);
        assert_eq!(snapshot.side_lowered_count, 0);
        assert_eq!(snapshot.side_executable_count, 0);
        assert_eq!(snapshot.side_blacklisted_count, 0);
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
        assert_eq!(
            jit.trace_status_for(chunk_ptr as usize, 2),
            Some(TraceStatus::Executable {
                instruction_count: 2,
            })
        );
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
        assert!(report.contains("status=Lowered(instr=4)"));
        assert!(report.contains("executor=SummaryOnly"));
        assert!(report.contains("details=load=1,arith=1,call=1,backedge=1"));
        assert!(report.contains("ssa_ti_opt=none"));
        assert!(report.contains("status=Blacklisted(attempts=1, reason=UnsupportedOpcode(TailCall))"));
    }

    #[test]
    fn guarded_numeric_for_trace_is_enterable_and_reported() {
        let mut vm = LuaVM::new(SafeOption::default());
        let results = vm
            .execute(
                r#"
                local function is_non_decreasing(a)
                    for i = 2, #a do
                        if a[i - 1] > a[i] then
                            return false
                        end
                    end
                    return true
                end

                local a = {}
                for i = 1, 64 do
                    a[i] = i
                end

                local checks = 0
                for iter = 1, 200 do
                    if is_non_decreasing(a) then
                        checks = checks + 1
                    end
                end

                return checks
                "#,
            )
            .unwrap();
        assert_eq!(results[0].as_integer(), Some(200));

        let report = vm.jit.trace_report();
        assert!(report.contains("executor=NativeGuardedNumericForLoop"));
        let compiled_trace = vm
            .jit
            .traces
            .values()
            .filter_map(|trace| trace.compiled_trace.as_ref())
            .find(|trace| trace.executor_family() == "NativeGuardedNumericForLoop")
            .unwrap();
        assert!(compiled_trace.is_enterable());
        assert_eq!(compiled_trace.executor_family(), "NativeGuardedNumericForLoop");
        assert!(report.contains("status=Executable(instr=6) executor=NativeGuardedNumericForLoop"));
    }

    #[test]
    fn guarded_numeric_for_trace_exit_starts_side_trace_from_guard_exit_pc() {
        let mut vm = LuaVM::new(SafeOption::default());
        let results = vm
            .execute(
                r#"
                local function is_non_decreasing(a)
                    for i = 2, #a do
                        if a[i - 1] > a[i] then
                            return false
                        end
                    end
                    return true
                end

                local a = {}
                for i = 1, 64 do
                    a[i] = i
                end

                local checks = 0
                for iter = 1, 200 do
                    if is_non_decreasing(a) then
                        checks = checks + 1
                    end
                end

                return checks
                "#,
            )
            .unwrap();
        assert_eq!(results[0].as_integer(), Some(200));

        let (trace_key, guard_exit_pc) = vm
            .jit
            .traces
            .iter()
            .find_map(|(key, trace)| {
                let compiled = trace.compiled_trace.as_ref()?;
                if compiled.executor_family() != "NativeGuardedNumericForLoop" {
                    return None;
                }
                let exit_pc = trace.artifact.as_ref()?.exits.first()?.exit_pc;
                Some((*key, exit_pc))
            })
            .unwrap();

        let deopt = vm
            .jit
            .record_trace_exit(trace_key.chunk_addr as *const LuaProto, trace_key.pc, guard_exit_pc)
            .unwrap();
        assert_eq!(deopt.exit_index, 0);
        assert_eq!(deopt.resume_pc, guard_exit_pc);

        let snapshot = vm.jit.stats_snapshot();
        assert_eq!(snapshot.side_trace_count, 1);

        let side = vm
            .jit
            .side_traces
            .values()
            .find(|trace| trace.start_pc == guard_exit_pc)
            .unwrap();
        assert!(matches!(side.status, SideTraceStatus::Counting { hits: 1 }));

        let report = vm.jit.trace_report();
        assert!(report.contains(&format!("start_pc={} status=Counting(hits=1)", guard_exit_pc)));
    }

    #[test]
    fn head_guard_linear_int_jmp_trace_is_enterable_and_reported() {
        let mut vm = LuaVM::new(SafeOption::default());
        let results = vm
            .execute(
                r#"
                local total = 0
                for outer = 1, 80 do
                    local i = 0
                    local acc = 0
                    while i < 128 do
                        acc = acc + i
                        i = i + 1
                    end
                    total = total + acc
                end
                return total
                "#,
            )
            .unwrap();

        assert_eq!(results[0].as_integer(), Some(650240));

        let report = vm.jit.trace_report();
        assert!(report.contains("LinearIntJmpLoop"));
        assert!(report.contains("status=Executable"));
        assert!(vm.jit.stats_snapshot().counters.trace_enter_hits > 0);
    }

    #[test]
    fn tail_guard_linear_int_jmp_trace_is_enterable_and_reported() {
        let mut vm = LuaVM::new(SafeOption::default());
        let results = vm
            .execute(
                r#"
                local total = 0
                for outer = 1, 80 do
                    local i = 0
                    local acc = 0
                    repeat
                        acc = acc + i
                        i = i + 1
                    until i >= 128
                    total = total + acc
                end
                return total
                "#,
            )
            .unwrap();

        assert_eq!(results[0].as_integer(), Some(650240));

        let report = vm.jit.trace_report();
        assert!(report.contains("LinearIntJmpLoop"));
        assert!(report.contains("status=Executable"));
    }

    #[test]
    fn upvalue_numeric_for_trace_is_enterable_and_reported() {
        let mut vm = LuaVM::new(SafeOption::default());
        let results = vm
            .execute(
                r#"
                local iterations = 200
                local upvalue_var = 0

                local function upvalue_test()
                    for i = 1, iterations do
                        upvalue_var = upvalue_var + 1
                    end
                end

                upvalue_test()
                return upvalue_var
                "#,
            )
            .unwrap();

        assert_eq!(results[0].as_integer(), Some(200));

        let report = vm.jit.trace_report();
        assert!(report.contains("executor=NativeNumericForLoop"));
        assert!(report.contains("details=upget=1,upset=1,arith=1,backedge=1"));

        let compiled_trace = vm
            .jit
            .traces
            .values()
            .filter_map(|trace| trace.compiled_trace.as_ref())
            .find(|trace| trace.executor_family() == "NativeNumericForLoop")
            .unwrap();
        assert!(compiled_trace.is_enterable());
    }

    #[test]
    fn benchmark_mixed_arithmetic_trace_is_native_and_correct() {
        let mut vm = LuaVM::new(SafeOption::default());
        let results = vm
            .execute(
                r#"
                local iterations = 200
                local x, y, z = 0, 0, 0
                for i = 1, iterations do
                    x = i + 5
                    y = x * 2
                    z = y - 3
                end
                return z
                "#,
            )
            .unwrap();

        assert_eq!(results[0].as_integer(), Some(407));

        let report = vm.jit.trace_report();
        assert!(report.contains("executor=NativeNumericForLoop"));
        assert!(report.contains("details=arith=3,backedge=1"));
    }
}

fn apply_native_lowering_profile(counters: &mut JitCounters, profile: NativeLoweringProfile) {
    counters.native_profile_guard_steps = counters
        .native_profile_guard_steps
        .saturating_add(profile.guard_steps);
    counters.native_profile_linear_guards = counters
        .native_profile_linear_guards
        .saturating_add(profile.linear_guard_steps);
    counters.native_profile_numeric_int_compare_guards = counters
        .native_profile_numeric_int_compare_guards
        .saturating_add(profile.numeric_int_compare_guard_steps);
    counters.native_profile_numeric_reg_compare_guards = counters
        .native_profile_numeric_reg_compare_guards
        .saturating_add(profile.numeric_reg_compare_guard_steps);
    counters.native_profile_truthy_guards = counters
        .native_profile_truthy_guards
        .saturating_add(profile.truthy_guard_steps);
    counters.native_profile_arithmetic_helpers = counters
        .native_profile_arithmetic_helpers
        .saturating_add(profile.arithmetic_helper_steps);
    counters.native_profile_table_helpers = counters
        .native_profile_table_helpers
        .saturating_add(profile.table_helper_steps);
    counters.native_profile_upvalue_helpers = counters
        .native_profile_upvalue_helpers
        .saturating_add(profile.upvalue_helper_steps);
    counters.native_profile_shift_helpers = counters
        .native_profile_shift_helpers
        .saturating_add(profile.shift_helper_steps);
}