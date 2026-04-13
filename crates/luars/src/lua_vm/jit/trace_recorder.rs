use super::ir::is_fused_arithmetic_metamethod_pair;
use crate::lua_value::LuaProto;
use crate::{Instruction, OpCode};

const MAX_RECORDED_TRACE_LEN: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TraceArtifact {
    pub seed: TraceSeed,
    pub ops: Vec<TraceOp>,
    pub exits: Vec<TraceExit>,
    pub loop_header_pc: u32,
    pub loop_tail_pc: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TraceSeed {
    pub start_pc: u32,
    pub root_chunk_addr: usize,
    pub instruction_budget: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TraceOp {
    pub pc: u32,
    pub instruction: Instruction,
    pub opcode: OpCode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TraceExit {
    pub guard_pc: u32,
    pub branch_pc: u32,
    pub exit_pc: u32,
    pub taken_on_trace: bool,
    pub kind: TraceExitKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraceExitKind {
    GuardExit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraceAbortReason {
    PcOutOfBounds,
    EmptyLoopBody,
    TraceTooLong,
    UnsupportedOpcode(OpCode),
    BackedgeMismatch { target_pc: u32 },
    MissingBranchAfterGuard,
    ForwardJump,
}

pub(crate) struct TraceRecorder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TraceRecordMode {
    Root,
    Side,
}

impl TraceRecorder {
    pub(crate) fn record_root(
        chunk_ptr: *const LuaProto,
        start_pc: u32,
    ) -> Result<TraceArtifact, TraceAbortReason> {
        Self::record_inner(chunk_ptr, start_pc, TraceRecordMode::Root)
    }

    pub(crate) fn record_side(
        chunk_ptr: *const LuaProto,
        start_pc: u32,
    ) -> Result<TraceArtifact, TraceAbortReason> {
        Self::record_inner(chunk_ptr, start_pc, TraceRecordMode::Side)
    }

    fn record_inner(
        chunk_ptr: *const LuaProto,
        start_pc: u32,
        mode: TraceRecordMode,
    ) -> Result<TraceArtifact, TraceAbortReason> {
        let chunk = unsafe { chunk_ptr.as_ref() }.ok_or(TraceAbortReason::PcOutOfBounds)?;

        let start_pc = start_pc as usize;
        if chunk.code.is_empty() {
            return Err(TraceAbortReason::EmptyLoopBody);
        }

        if start_pc >= chunk.code.len() {
            return Err(TraceAbortReason::PcOutOfBounds);
        }

        let seed = TraceSeed {
            start_pc: start_pc as u32,
            root_chunk_addr: chunk_ptr as usize,
            instruction_budget: chunk.code.len().min(u16::MAX as usize) as u16,
        };

        let mut pc = start_pc;
        let mut ops = Vec::new();
        let mut exits = Vec::new();

        loop {
            if ops.len() >= MAX_RECORDED_TRACE_LEN {
                return Err(TraceAbortReason::TraceTooLong);
            }

            let instruction = *chunk.code.get(pc).ok_or(TraceAbortReason::PcOutOfBounds)?;
            let opcode = instruction.get_opcode();
            if !is_supported_trace_opcode(opcode) {
                return Err(TraceAbortReason::UnsupportedOpcode(opcode));
            }

            if should_skip_fused_arithmetic_metamethod_companion(&ops, opcode, instruction) {
                pc += 1;
                continue;
            }

            ops.push(TraceOp {
                pc: pc as u32,
                instruction,
                opcode,
            });

            match opcode {
                OpCode::TForPrep => {
                    let next_pc = pc + 1 + instruction.get_bx() as usize;
                    let _ = chunk
                        .code
                        .get(next_pc)
                        .ok_or(TraceAbortReason::PcOutOfBounds)?;
                    pc = next_pc;
                    continue;
                }
                OpCode::TForLoop => {
                    let bx = instruction.get_bx() as usize;
                    let Some(target_pc) = (pc + 1).checked_sub(bx) else {
                        return Err(TraceAbortReason::PcOutOfBounds);
                    };
                    if target_pc != start_pc {
                        return finish_mismatched_backedge(
                            mode,
                            chunk_ptr,
                            seed,
                            &ops,
                            &exits,
                            target_pc as u32,
                            pc as u32,
                        );
                    }
                    exits.push(TraceExit {
                        guard_pc: pc as u32,
                        branch_pc: pc as u32,
                        exit_pc: (pc + 1) as u32,
                        taken_on_trace: true,
                        kind: TraceExitKind::GuardExit,
                    });
                    return Ok(TraceArtifact {
                        seed,
                        ops,
                        exits,
                        loop_header_pc: seed.start_pc,
                        loop_tail_pc: pc as u32,
                    });
                }
                OpCode::Eq
                | OpCode::Lt
                | OpCode::Le
                | OpCode::EqK
                | OpCode::EqI
                | OpCode::LtI
                | OpCode::LeI
                | OpCode::GtI
                | OpCode::GeI
                | OpCode::Test
                | OpCode::TestSet => {
                    let branch_pc = pc + 1;
                    let branch = *chunk
                        .code
                        .get(branch_pc)
                        .ok_or(TraceAbortReason::MissingBranchAfterGuard)?;
                    if branch.get_opcode() != OpCode::Jmp {
                        return Err(TraceAbortReason::MissingBranchAfterGuard);
                    }

                    let target_pc = ((branch_pc + 1) as isize + branch.get_sj() as isize) as usize;
                    ops.push(TraceOp {
                        pc: branch_pc as u32,
                        instruction: branch,
                        opcode: OpCode::Jmp,
                    });

                    if target_pc > branch_pc + 1 {
                        if let Some((exit_pc, next_pc)) =
                            guard_fallthrough_exit(chunk, branch_pc, target_pc)
                        {
                            exits.push(TraceExit {
                                guard_pc: pc as u32,
                                branch_pc: branch_pc as u32,
                                exit_pc,
                                taken_on_trace: false,
                                kind: TraceExitKind::GuardExit,
                            });
                            pc = next_pc as usize;
                            continue;
                        }

                        if let Some(next_pc) =
                            guard_taken_continue(chunk, start_pc as u32, target_pc as u32)
                        {
                            exits.push(TraceExit {
                                guard_pc: pc as u32,
                                branch_pc: branch_pc as u32,
                                exit_pc: (branch_pc + 1) as u32,
                                taken_on_trace: true,
                                kind: TraceExitKind::GuardExit,
                            });
                            pc = next_pc as usize;
                            continue;
                        }

                        exits.push(TraceExit {
                            guard_pc: pc as u32,
                            branch_pc: branch_pc as u32,
                            exit_pc: target_pc as u32,
                            taken_on_trace: false,
                            kind: TraceExitKind::GuardExit,
                        });
                        pc = branch_pc + 1;
                        continue;
                    }

                    if target_pc == start_pc {
                        exits.push(TraceExit {
                            guard_pc: pc as u32,
                            branch_pc: branch_pc as u32,
                            exit_pc: (branch_pc + 1) as u32,
                            taken_on_trace: true,
                            kind: TraceExitKind::GuardExit,
                        });
                        return Ok(TraceArtifact {
                            seed,
                            ops,
                            exits,
                            loop_header_pc: seed.start_pc,
                            loop_tail_pc: branch_pc as u32,
                        });
                    }

                    if let Some(artifact) = rerecord_from_backedge_target(
                        chunk_ptr,
                        seed,
                        target_pc as u32,
                        branch_pc as u32,
                    ) {
                        return artifact;
                    }

                    return finish_mismatched_backedge(
                        mode,
                        chunk_ptr,
                        seed,
                        &ops,
                        &exits,
                        target_pc as u32,
                        branch_pc as u32,
                    );
                }
                OpCode::Jmp => {
                    let target_pc = ((pc + 1) as isize + instruction.get_sj() as isize) as usize;
                    if target_pc >= pc + 1 {
                        if branch_merge_continue(&exits, pc as u32, target_pc as u32) {
                            pc = target_pc;
                            continue;
                        }
                        return Err(TraceAbortReason::ForwardJump);
                    }
                    if target_pc != start_pc {
                        return finish_mismatched_backedge(
                            mode,
                            chunk_ptr,
                            seed,
                            &ops,
                            &exits,
                            target_pc as u32,
                            pc as u32,
                        );
                    }
                    return Ok(TraceArtifact {
                        seed,
                        ops,
                        exits,
                        loop_header_pc: seed.start_pc,
                        loop_tail_pc: pc as u32,
                    });
                }
                OpCode::ForLoop => {
                    let bx = instruction.get_bx() as usize;
                    let Some(target_pc) = (pc + 1).checked_sub(bx) else {
                        return Err(TraceAbortReason::PcOutOfBounds);
                    };
                    if target_pc != start_pc {
                        return finish_mismatched_backedge(
                            mode,
                            chunk_ptr,
                            seed,
                            &ops,
                            &exits,
                            target_pc as u32,
                            pc as u32,
                        );
                    }
                    return Ok(TraceArtifact {
                        seed,
                        ops,
                        exits,
                        loop_header_pc: seed.start_pc,
                        loop_tail_pc: pc as u32,
                    });
                }
                OpCode::Return | OpCode::Return0 | OpCode::Return1 => {
                    return Ok(TraceArtifact {
                        seed,
                        ops,
                        exits,
                        loop_header_pc: seed.start_pc,
                        loop_tail_pc: pc as u32,
                    });
                }
                _ => {
                    pc += 1;
                }
            }
        }
    }
}

fn should_skip_fused_arithmetic_metamethod_companion(
    ops: &[TraceOp],
    opcode: OpCode,
    instruction: Instruction,
) -> bool {
    let Some(previous) = ops.last() else {
        return false;
    };

    matches!(opcode, OpCode::MmBin | OpCode::MmBinI | OpCode::MmBinK)
        && is_fused_arithmetic_metamethod_pair(
            previous.opcode,
            previous.instruction,
            opcode,
            instruction,
        )
}

fn guard_fallthrough_exit(
    chunk: &LuaProto,
    branch_pc: usize,
    target_pc: usize,
) -> Option<(u32, u32)> {
    let fallthrough_pc = branch_pc + 1;
    let fallthrough = *chunk.code.get(fallthrough_pc)?;
    if fallthrough.get_opcode() != OpCode::Jmp {
        return None;
    }

    let exit_pc = ((fallthrough_pc + 1) as isize + fallthrough.get_sj() as isize) as usize;
    if exit_pc <= fallthrough_pc + 1 {
        return None;
    }

    Some((exit_pc as u32, target_pc as u32))
}

fn guard_taken_continue(chunk: &LuaProto, start_pc: u32, target_pc: u32) -> Option<u32> {
    let target_pc = target_pc as usize;
    let instruction = *chunk.code.get(target_pc)?;
    match instruction.get_opcode() {
        OpCode::ForLoop | OpCode::TForLoop => {
            let loop_target = (target_pc + 1).checked_sub(instruction.get_bx() as usize)? as u32;
            (loop_target == start_pc).then_some(target_pc as u32)
        }
        OpCode::Jmp => {
            let loop_target = ((target_pc + 1) as i64 + instruction.get_sj() as i64) as u32;
            (loop_target == start_pc).then_some(target_pc as u32)
        }
        _ => None,
    }
}

fn finish_mismatched_backedge(
    mode: TraceRecordMode,
    chunk_ptr: *const LuaProto,
    seed: TraceSeed,
    ops: &[TraceOp],
    exits: &[TraceExit],
    target_pc: u32,
    loop_tail_pc: u32,
) -> Result<TraceArtifact, TraceAbortReason> {
    match mode {
        TraceRecordMode::Root => {
            if let Some(artifact) =
                rerecord_from_backedge_target(chunk_ptr, seed, target_pc, loop_tail_pc)
            {
                return artifact;
            }
            if let Some(artifact) = reroot_trace(seed, ops, exits, target_pc, loop_tail_pc) {
                return Ok(artifact);
            }
        }
        TraceRecordMode::Side => {
            if let Some(artifact) = finalize_side_trace(seed, ops, exits, target_pc, loop_tail_pc) {
                return Ok(artifact);
            }
        }
    }

    Err(TraceAbortReason::BackedgeMismatch { target_pc })
}

fn rerecord_from_backedge_target(
    chunk_ptr: *const LuaProto,
    seed: TraceSeed,
    target_pc: u32,
    loop_tail_pc: u32,
) -> Option<Result<TraceArtifact, TraceAbortReason>> {
    if target_pc == seed.start_pc || target_pc >= loop_tail_pc {
        return None;
    }

    Some(TraceRecorder::record_inner(
        chunk_ptr,
        target_pc,
        TraceRecordMode::Root,
    ))
}

fn branch_merge_continue(exits: &[TraceExit], branch_pc: u32, target_pc: u32) -> bool {
    exits.last().is_some_and(|exit| {
        matches!(exit.kind, TraceExitKind::GuardExit)
            && !exit.taken_on_trace
            && exit.exit_pc == branch_pc + 1
            && target_pc > exit.exit_pc
    })
}

fn reroot_trace(
    seed: TraceSeed,
    ops: &[TraceOp],
    exits: &[TraceExit],
    new_start_pc: u32,
    loop_tail_pc: u32,
) -> Option<TraceArtifact> {
    if new_start_pc <= seed.start_pc {
        return None;
    }

    if new_start_pc >= loop_tail_pc {
        return None;
    }

    let start_idx = ops.iter().position(|op| op.pc == new_start_pc)?;
    let rerooted_ops = ops[start_idx..].to_vec();
    let rerooted_exits = exits
        .iter()
        .copied()
        .filter(|exit| exit.guard_pc >= new_start_pc && exit.branch_pc >= new_start_pc)
        .collect();

    Some(TraceArtifact {
        seed: TraceSeed {
            start_pc: new_start_pc,
            ..seed
        },
        ops: rerooted_ops,
        exits: rerooted_exits,
        loop_header_pc: new_start_pc,
        loop_tail_pc,
    })
}

fn finalize_side_trace(
    seed: TraceSeed,
    ops: &[TraceOp],
    exits: &[TraceExit],
    loop_header_pc: u32,
    loop_tail_pc: u32,
) -> Option<TraceArtifact> {
    if loop_header_pc >= seed.start_pc || loop_header_pc >= loop_tail_pc {
        return None;
    }

    Some(TraceArtifact {
        seed,
        ops: ops.to_vec(),
        exits: exits.to_vec(),
        loop_header_pc,
        loop_tail_pc,
    })
}

fn is_supported_trace_opcode(opcode: OpCode) -> bool {
    matches!(
        opcode,
        OpCode::Move
            | OpCode::LoadI
            | OpCode::LoadF
            | OpCode::LoadK
            | OpCode::LoadKX
            | OpCode::LoadFalse
            | OpCode::LFalseSkip
            | OpCode::LoadTrue
            | OpCode::LoadNil
            | OpCode::GetUpval
            | OpCode::SetUpval
            | OpCode::Close
            | OpCode::GetTabUp
            | OpCode::GetTable
            | OpCode::GetI
            | OpCode::GetField
            | OpCode::SetTabUp
            | OpCode::SetTable
            | OpCode::SetI
            | OpCode::SetField
            | OpCode::SetList
            | OpCode::NewTable
            | OpCode::Self_
            | OpCode::AddI
            | OpCode::AddK
            | OpCode::SubK
            | OpCode::MulK
            | OpCode::ModK
            | OpCode::PowK
            | OpCode::DivK
            | OpCode::IDivK
            | OpCode::BAndK
            | OpCode::BOrK
            | OpCode::BXorK
            | OpCode::ShlI
            | OpCode::ShrI
            | OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::Div
            | OpCode::IDiv
            | OpCode::BAnd
            | OpCode::BOr
            | OpCode::BXor
            | OpCode::Shl
            | OpCode::Shr
            | OpCode::Unm
            | OpCode::BNot
            | OpCode::Not
            | OpCode::Len
            | OpCode::Concat
            | OpCode::Closure
            | OpCode::MmBin
            | OpCode::MmBinI
            | OpCode::MmBinK
            | OpCode::Call
            | OpCode::TForCall
            | OpCode::ForPrep
            | OpCode::TForPrep
            | OpCode::TForLoop
            | OpCode::Return
            | OpCode::Return0
            | OpCode::Return1
            | OpCode::ExtraArg
            | OpCode::Eq
            | OpCode::Lt
            | OpCode::Le
            | OpCode::EqK
            | OpCode::EqI
            | OpCode::LtI
            | OpCode::LeI
            | OpCode::GtI
            | OpCode::GeI
            | OpCode::Test
            | OpCode::TestSet
            | OpCode::Jmp
            | OpCode::ForLoop
    )
}

#[cfg(test)]
mod tests {
    use super::{TraceAbortReason, TraceExit, TraceExitKind, TraceRecorder};
    use crate::lua_value::LuaProto;
    use crate::lua_vm::jit::helper_plan::{HelperPlan, HelperPlanStep};
    use crate::lua_vm::jit::ir::TraceIr;
    use crate::lua_vm::jit::lowering::LoweredTrace;
    use crate::{Instruction, OpCode};
    use crate::{LuaVM, SafeOption};

    fn load_bench_quicksort_chunk() -> LuaProto {
        let mut vm = LuaVM::new(SafeOption::default());
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/bench_quicksort.lua"
        ))
        .unwrap();
        vm.compile_with_name(&source, "@bench_quicksort.lua")
            .unwrap()
    }

    fn load_bench_functions_chunk() -> LuaProto {
        let mut vm = LuaVM::new(SafeOption::default());
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/bench_functions.lua"
        ))
        .unwrap();
        vm.compile_with_name(&source, "@bench_functions.lua")
            .unwrap()
    }

    fn find_child_proto(chunk: &LuaProto, linedefined: usize, lastlinedefined: usize) -> &LuaProto {
        chunk
            .child_protos
            .iter()
            .find_map(|proto| {
                let proto = &proto.as_ref().data;
                (proto.linedefined == linedefined && proto.lastlinedefined == lastlinedefined)
                    .then_some(proto)
            })
            .unwrap_or_else(|| {
                let ranges = chunk
                    .child_protos
                    .iter()
                    .map(|proto| {
                        let proto = &proto.as_ref().data;
                        (proto.linedefined, proto.lastlinedefined, proto.code.len())
                    })
                    .collect::<Vec<_>>();
                panic!(
                    "child proto not found for {linedefined}..{lastlinedefined}; child ranges={ranges:?}"
                );
            })
    }

    #[test]
    fn recorder_rejects_null_root() {
        assert_eq!(
            TraceRecorder::record_root(std::ptr::null(), 0),
            Err(TraceAbortReason::PcOutOfBounds)
        );
    }

    #[test]
    fn recorder_rejects_out_of_bounds_pc() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        assert_eq!(
            TraceRecorder::record_root(&chunk as *const LuaProto, 1),
            Err(TraceAbortReason::PcOutOfBounds)
        );
    }

    #[test]
    fn recorder_is_explicitly_stubbed_for_valid_roots() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::ForLoop, 0, 2));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 2);
        assert!(artifact.exits.is_empty());
        assert_eq!(artifact.loop_tail_pc, 1);
    }

    #[test]
    fn recorder_rejects_unsupported_opcode() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abck(OpCode::TailCall, 0, 1, 1, false));

        assert_eq!(
            TraceRecorder::record_root(&chunk as *const LuaProto, 0),
            Err(TraceAbortReason::UnsupportedOpcode(OpCode::TailCall))
        );
    }

    #[test]
    fn recorder_accepts_call_inside_loop() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Call, 0, 2, 2));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -3));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 3);
        assert_eq!(artifact.loop_tail_pc, 2);
    }

    #[test]
    fn recorder_skips_fused_mmbin_inside_loop() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Add, 0, 1, 2, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::MmBin, 1, 2, 0));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -3));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 2);
        assert_eq!(
            artifact.ops.iter().map(|op| op.pc).collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(artifact.loop_tail_pc, 2);
    }

    #[test]
    fn recorder_inspects_quicksort_partition_outer_loop_trace() {
        let mut vm = LuaVM::new(SafeOption::default());
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/bench_quicksort.lua"
        ))
        .unwrap();
        let chunk = vm
            .compile_with_name(&source, "@bench_quicksort.lua")
            .unwrap();

        let partition = chunk
            .child_protos
            .iter()
            .find_map(|proto| {
                let proto = &proto.as_ref().data;
                (proto.linedefined == 50 && proto.lastlinedefined == 75).then_some(proto)
            })
            .unwrap();

        let mut recorded = Vec::new();
        for start_pc in 5..=12 {
            match TraceRecorder::record_root(partition as *const LuaProto, start_pc) {
                Ok(artifact) => recorded.push((
                    start_pc,
                    artifact.seed.start_pc,
                    artifact
                        .ops
                        .iter()
                        .map(|op| (op.pc, op.opcode))
                        .collect::<Vec<_>>(),
                    artifact.exits,
                )),
                Err(err) => println!("partition trace start_pc={start_pc} err={err:?}"),
            }
        }
        println!("partition recorded traces={recorded:?}");
        assert!(!recorded.is_empty());
    }

    #[test]
    fn recorder_rerecords_quicksort_partition_inner_exit_to_outer_header() {
        let mut vm = LuaVM::new(SafeOption::default());
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/bench_quicksort.lua"
        ))
        .unwrap();
        let chunk = vm
            .compile_with_name(&source, "@bench_quicksort.lua")
            .unwrap();

        let partition = chunk
            .child_protos
            .iter()
            .find_map(|proto| {
                let proto = &proto.as_ref().data;
                (proto.linedefined == 50 && proto.lastlinedefined == 75).then_some(proto)
            })
            .unwrap();

        let artifact = TraceRecorder::record_root(partition as *const LuaProto, 21).unwrap();
        assert_eq!(artifact.seed.start_pc, 9);
        assert_eq!(
            artifact.ops.iter().map(|op| op.pc).collect::<Vec<_>>(),
            vec![9, 10, 11, 12, 14]
        );
        assert_eq!(artifact.exits.len(), 1);
        assert_eq!(artifact.exits[0].exit_pc, 15);
        assert!(!artifact.exits[0].taken_on_trace);
        assert_eq!(artifact.loop_tail_pc, 14);
    }

    #[test]
    fn recorder_inspects_quicksort_build_source_array_trace() {
        let mut vm = LuaVM::new(SafeOption::default());
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/bench_quicksort.lua"
        ))
        .unwrap();
        let chunk = vm
            .compile_with_name(&source, "@bench_quicksort.lua")
            .unwrap();

        let proto = chunk
            .child_protos
            .iter()
            .find_map(|proto| {
                let proto = &proto.as_ref().data;
                (proto.linedefined == 20 && proto.lastlinedefined == 26).then_some(proto)
            })
            .unwrap_or_else(|| {
                let ranges = chunk
                    .child_protos
                    .iter()
                    .map(|proto| {
                        let proto = &proto.as_ref().data;
                        (proto.linedefined, proto.lastlinedefined, proto.code.len())
                    })
                    .collect::<Vec<_>>();
                panic!("build_source_array proto not found; child ranges={ranges:?}");
            });

        let mut recorded = Vec::new();
        for start_pc in 0..proto.code.len() as u32 {
            if let Ok(artifact) = TraceRecorder::record_root(proto as *const LuaProto, start_pc) {
                recorded.push((
                    start_pc,
                    artifact.seed.start_pc,
                    artifact
                        .ops
                        .iter()
                        .map(|op| (op.pc, op.opcode))
                        .collect::<Vec<_>>(),
                ));
            }
        }
        println!("build_source_array traces={recorded:?}");
        assert!(!recorded.is_empty());
    }

    #[test]
    fn recorder_inspects_quicksort_copy_array_trace() {
        let mut vm = LuaVM::new(SafeOption::default());
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/bench_quicksort.lua"
        ))
        .unwrap();
        let chunk = vm
            .compile_with_name(&source, "@bench_quicksort.lua")
            .unwrap();

        let proto = chunk
            .child_protos
            .iter()
            .find_map(|proto| {
                let proto = &proto.as_ref().data;
                (proto.linedefined == 30 && proto.lastlinedefined == 36).then_some(proto)
            })
            .unwrap_or_else(|| {
                let ranges = chunk
                    .child_protos
                    .iter()
                    .map(|proto| {
                        let proto = &proto.as_ref().data;
                        (proto.linedefined, proto.lastlinedefined, proto.code.len())
                    })
                    .collect::<Vec<_>>();
                panic!("copy_array proto not found; child ranges={ranges:?}");
            });

        let mut recorded = Vec::new();
        for start_pc in 0..proto.code.len() as u32 {
            if let Ok(artifact) = TraceRecorder::record_root(proto as *const LuaProto, start_pc) {
                recorded.push((
                    start_pc,
                    artifact.seed.start_pc,
                    artifact
                        .ops
                        .iter()
                        .map(|op| (op.pc, op.opcode))
                        .collect::<Vec<_>>(),
                    artifact.exits,
                ));
            }
        }
        println!("copy_array traces={recorded:?}");
        assert!(!recorded.is_empty());
    }

    #[test]
    fn recorder_inspects_quicksort_insertion_sort_trace() {
        let mut vm = LuaVM::new(SafeOption::default());
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/bench_quicksort.lua"
        ))
        .unwrap();
        let chunk = vm
            .compile_with_name(&source, "@bench_quicksort.lua")
            .unwrap();

        let proto = chunk
            .child_protos
            .iter()
            .find_map(|proto| {
                let proto = &proto.as_ref().data;
                (proto.linedefined == 38 && proto.lastlinedefined == 48).then_some(proto)
            })
            .unwrap_or_else(|| {
                let ranges = chunk
                    .child_protos
                    .iter()
                    .map(|proto| {
                        let proto = &proto.as_ref().data;
                        (proto.linedefined, proto.lastlinedefined, proto.code.len())
                    })
                    .collect::<Vec<_>>();
                panic!("insertion_sort proto not found; child ranges={ranges:?}");
            });

        let mut recorded = Vec::new();
        for start_pc in 0..proto.code.len() as u32 {
            if let Ok(artifact) = TraceRecorder::record_root(proto as *const LuaProto, start_pc) {
                recorded.push((
                    start_pc,
                    artifact.seed.start_pc,
                    artifact
                        .ops
                        .iter()
                        .map(|op| (op.pc, op.opcode))
                        .collect::<Vec<_>>(),
                    artifact.exits,
                ));
            }
        }
        let insertion_loop = recorded
            .iter()
            .find(|(start_pc, seed_pc, ops, exits)| {
                *start_pc == 0
                    && *seed_pc == 8
                    && ops.len() == 10
                    && exits.len() == 2
                    && ops.first() == Some(&(8, crate::OpCode::Le))
                    && ops.last() == Some(&(19, crate::OpCode::Jmp))
            })
            .cloned();

        assert_eq!(
            insertion_loop,
            Some((
                0,
                8,
                vec![
                    (8, crate::OpCode::Le),
                    (9, crate::OpCode::Jmp),
                    (10, crate::OpCode::GetTable),
                    (11, crate::OpCode::Lt),
                    (12, crate::OpCode::Jmp),
                    (13, crate::OpCode::AddI),
                    (15, crate::OpCode::GetTable),
                    (16, crate::OpCode::SetTable),
                    (17, crate::OpCode::AddI),
                    (19, crate::OpCode::Jmp),
                ],
                vec![
                    TraceExit {
                        guard_pc: 8,
                        branch_pc: 9,
                        exit_pc: 20,
                        taken_on_trace: false,
                        kind: TraceExitKind::GuardExit,
                    },
                    TraceExit {
                        guard_pc: 11,
                        branch_pc: 12,
                        exit_pc: 20,
                        taken_on_trace: false,
                        kind: TraceExitKind::GuardExit,
                    },
                ],
            ))
        );
    }

    #[test]
    fn recorder_inspects_quicksort_checksum_trace() {
        let mut vm = LuaVM::new(SafeOption::default());
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/bench_quicksort.lua"
        ))
        .unwrap();
        let chunk = vm
            .compile_with_name(&source, "@bench_quicksort.lua")
            .unwrap();

        let proto = chunk
            .child_protos
            .iter()
            .find_map(|proto| {
                let proto = &proto.as_ref().data;
                (proto.linedefined == 100 && proto.lastlinedefined == 106).then_some(proto)
            })
            .unwrap_or_else(|| {
                let ranges = chunk
                    .child_protos
                    .iter()
                    .map(|proto| {
                        let proto = &proto.as_ref().data;
                        (proto.linedefined, proto.lastlinedefined, proto.code.len())
                    })
                    .collect::<Vec<_>>();
                panic!("checksum proto not found; child ranges={ranges:?}");
            });

        let mut recorded = Vec::new();
        for start_pc in 0..proto.code.len() as u32 {
            if let Ok(artifact) = TraceRecorder::record_root(proto as *const LuaProto, start_pc) {
                recorded.push((
                    start_pc,
                    artifact.seed.start_pc,
                    artifact
                        .ops
                        .iter()
                        .map(|op| (op.pc, op.opcode))
                        .collect::<Vec<_>>(),
                    artifact.exits,
                ));
            }
        }
        println!("checksum traces={recorded:?}");
        assert!(!recorded.is_empty());
    }

    #[test]
    fn recorder_inspects_quicksort_is_sorted_trace() {
        let mut vm = LuaVM::new(SafeOption::default());
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benchmarks/bench_quicksort.lua"
        ))
        .unwrap();
        let chunk = vm
            .compile_with_name(&source, "@bench_quicksort.lua")
            .unwrap();

        let proto = chunk
            .child_protos
            .iter()
            .find_map(|proto| {
                let proto = &proto.as_ref().data;
                (proto.linedefined == 108 && proto.lastlinedefined == 115).then_some(proto)
            })
            .unwrap_or_else(|| {
                let ranges = chunk
                    .child_protos
                    .iter()
                    .map(|proto| {
                        let proto = &proto.as_ref().data;
                        (proto.linedefined, proto.lastlinedefined, proto.code.len())
                    })
                    .collect::<Vec<_>>();
                panic!("is_sorted proto not found; child ranges={ranges:?}");
            });

        let mut recorded = Vec::new();
        for start_pc in 0..proto.code.len() as u32 {
            if let Ok(artifact) = TraceRecorder::record_root(proto as *const LuaProto, start_pc) {
                recorded.push((
                    start_pc,
                    artifact.seed.start_pc,
                    artifact
                        .ops
                        .iter()
                        .map(|op| (op.pc, op.opcode))
                        .collect::<Vec<_>>(),
                    artifact.exits,
                ));
            }
        }
        println!("is_sorted traces={recorded:?}");
        assert!(!recorded.is_empty());
    }

    #[test]
    fn recorder_inspects_quicksort_main_loop_trace_blocker() {
        let chunk = load_bench_quicksort_chunk();
        let quicksort = find_child_proto(&chunk, 77, 98);

        let artifact = TraceRecorder::record_root(quicksort as *const LuaProto, 12).unwrap();

        assert_eq!(artifact.seed.start_pc, 0);
        assert_eq!(artifact.loop_tail_pc, 11);
        assert_eq!(
            artifact
                .ops
                .iter()
                .map(|op| (op.pc, op.opcode))
                .collect::<Vec<_>>(),
            vec![
                (0, OpCode::Lt),
                (1, OpCode::Jmp),
                (2, OpCode::Sub),
                (4, OpCode::LeI),
                (5, OpCode::Jmp),
                (6, OpCode::GetUpval),
                (7, OpCode::Move),
                (8, OpCode::Move),
                (9, OpCode::Move),
                (10, OpCode::Call),
                (11, OpCode::Return0),
            ]
        );
        assert_eq!(artifact.exits.len(), 2);
        assert_eq!(artifact.exits[0].guard_pc, 0);
        assert_eq!(artifact.exits[0].branch_pc, 1);
        assert_eq!(artifact.exits[0].exit_pc, 41);
        assert!(!artifact.exits[0].taken_on_trace);
        assert_eq!(artifact.exits[1].guard_pc, 4);
        assert_eq!(artifact.exits[1].branch_pc, 5);
        assert_eq!(artifact.exits[1].exit_pc, 12);
        assert!(!artifact.exits[1].taken_on_trace);
    }

    #[test]
    fn recorder_inspects_quicksort_top_level_trace_blocker() {
        let chunk = load_bench_quicksort_chunk();
        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 34).unwrap();

        assert_eq!(artifact.seed.start_pc, 34);
        assert_eq!(artifact.loop_tail_pc, 57);
        assert_eq!(
            artifact
                .ops
                .iter()
                .map(|op| (op.pc, op.opcode))
                .collect::<Vec<_>>(),
            vec![
                (34, OpCode::Move),
                (35, OpCode::Move),
                (36, OpCode::Call),
                (37, OpCode::Move),
                (38, OpCode::Move),
                (39, OpCode::LoadI),
                (40, OpCode::Len),
                (41, OpCode::Call),
                (42, OpCode::Move),
                (43, OpCode::Move),
                (44, OpCode::Call),
                (45, OpCode::Test),
                (46, OpCode::Jmp),
                (47, OpCode::GetTabUp),
                (48, OpCode::LoadK),
                (49, OpCode::Call),
                (50, OpCode::Move),
                (51, OpCode::Move),
                (52, OpCode::Call),
                (53, OpCode::Add),
                (55, OpCode::ModK),
                (57, OpCode::ForLoop),
            ]
        );
        assert_eq!(artifact.exits.len(), 1);
        assert_eq!(
            artifact.exits[0],
            super::TraceExit {
                guard_pc: 45,
                branch_pc: 46,
                exit_pc: 50,
                taken_on_trace: false,
                kind: TraceExitKind::GuardExit,
            }
        );

        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(helper_plan.root_pc, 34);
        assert_eq!(helper_plan.loop_tail_pc, 57);
        assert_eq!(helper_plan.guard_count, 1);
        assert_eq!(
            helper_plan.dispatch().steps_executed as usize,
            helper_plan.steps.len()
        );
        assert_eq!(helper_plan.dispatch().guards_observed, 1);
        assert_eq!(helper_plan.dispatch().call_steps, 5);
        assert_eq!(
            helper_plan
                .steps
                .iter()
                .filter(|step| matches!(step, HelperPlanStep::Call { .. }))
                .count(),
            5
        );
        assert_eq!(
            helper_plan
                .steps
                .iter()
                .filter(|step| matches!(step, HelperPlanStep::TableAccess { .. }))
                .count(),
            1
        );
        assert_eq!(lowered.ssa_memory_effect_summary().call_count, 5);
    }

    #[test]
    fn recorder_inspects_bench_functions_vararg_outer_loop_trace() {
        let chunk = load_bench_functions_chunk();
        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 70).unwrap();

        let ops = artifact
            .ops
            .iter()
            .map(|op| (op.pc, op.opcode))
            .collect::<Vec<_>>();

        println!("bench_functions outer trace={ops:?} exits={:?}", artifact.exits);

        assert_eq!(artifact.seed.start_pc, 70);
        assert_eq!(artifact.loop_tail_pc, 78);
        assert_eq!(artifact.ops.len(), 9);
        assert_eq!(ops.last(), Some(&(78, OpCode::ForLoop)));
        assert_eq!(ops.iter().filter(|(_, opcode)| *opcode == OpCode::Call).count(), 1);
        assert!(artifact.exits.is_empty());
    }

    #[test]
    fn recorder_reports_varargprep_blocker_for_bench_functions_vararg_child_entry() {
        let chunk = load_bench_functions_chunk();
        let proto = find_child_proto(&chunk, 32, 38);
        assert_eq!(
            TraceRecorder::record_root(proto as *const LuaProto, 0),
            Err(TraceAbortReason::UnsupportedOpcode(OpCode::VarargPrep))
        );
    }

    #[test]
    fn recorder_preserves_quicksort_top_level_side_trace_identity() {
        let chunk = load_bench_quicksort_chunk();
        let artifact = TraceRecorder::record_side(&chunk as *const LuaProto, 50).unwrap();

        assert_eq!(artifact.seed.start_pc, 50);
        assert_eq!(artifact.loop_header_pc, 34);
        assert_eq!(artifact.loop_tail_pc, 57);
        assert_eq!(
            artifact
                .ops
                .iter()
                .map(|op| (op.pc, op.opcode))
                .collect::<Vec<_>>(),
            vec![
                (50, OpCode::Move),
                (51, OpCode::Move),
                (52, OpCode::Call),
                (53, OpCode::Add),
                (55, OpCode::ModK),
                (57, OpCode::ForLoop),
            ]
        );
        assert!(artifact.exits.is_empty());
    }

    #[test]
    fn recorder_rerecords_backward_guard_branch_to_earlier_header() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 1, 2, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 2, 3, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 0, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 3).unwrap();
        assert_eq!(artifact.seed.start_pc, 0);
        assert_eq!(
            artifact.ops.iter().map(|op| op.pc).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(artifact.loop_tail_pc, 4);
    }

    #[test]
    fn recorder_accepts_getupval_and_forprep_inside_loop() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::GetUpval, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::ForPrep, 0, 1));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::ForLoop, 0, 3));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 3);
        assert_eq!(artifact.loop_tail_pc, 2);
    }

    #[test]
    fn recorder_follows_tforprep_jump_target() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abx(OpCode::TForPrep, 0, 1));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::GetUpval, 0, 1, 0));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -4));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 3);
        assert_eq!(artifact.ops[1].pc, 2);
        assert_eq!(artifact.loop_tail_pc, 3);
    }

    #[test]
    fn recorder_accepts_tforcall_inside_loop() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::TForCall, 0, 0, 2));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -2));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 2);
        assert_eq!(artifact.loop_tail_pc, 1);
    }

    #[test]
    fn recorder_accepts_terminal_return0_trace() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return0, 0, 0, 0));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 1);
        assert_eq!(artifact.ops[0].opcode, OpCode::Return0);
        assert!(artifact.exits.is_empty());
        assert_eq!(artifact.loop_tail_pc, 0);
    }

    #[test]
    fn recorder_accepts_terminal_return1_trace() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Return1, 3, 0, 0));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 1);
        assert_eq!(artifact.ops[0].opcode, OpCode::Return1);
        assert!(artifact.exits.is_empty());
        assert_eq!(artifact.loop_tail_pc, 0);
    }

    #[test]
    fn recorder_accepts_terminal_return_trace() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Return, 2, 3, 0, false));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 1);
        assert_eq!(artifact.ops[0].opcode, OpCode::Return);
        assert!(artifact.exits.is_empty());
        assert_eq!(artifact.loop_tail_pc, 0);
    }

    #[test]
    fn recorder_accepts_setupval_and_closure_inside_loop() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abck(OpCode::SetUpval, 0, 1, 0, false));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::Closure, 1, 0));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -3));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 3);
        assert_eq!(artifact.loop_tail_pc, 2);
    }

    #[test]
    fn recorder_records_tforloop_backedge_with_exit() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::TForCall, 0, 0, 2));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::TForLoop, 0, 2));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 2);
        assert_eq!(artifact.exits.len(), 1);
        assert_eq!(artifact.exits[0].guard_pc, 1);
        assert_eq!(artifact.exits[0].branch_pc, 1);
        assert_eq!(artifact.exits[0].exit_pc, 2);
        assert!(artifact.exits[0].taken_on_trace);
        assert_eq!(artifact.loop_tail_pc, 1);
    }

    #[test]
    fn recorder_accepts_setlist_and_close_inside_loop() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_vabck(OpCode::SetList, 0, 2, 1, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Close, 1, 0, 0));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -3));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 3);
        assert_eq!(artifact.loop_tail_pc, 2);
    }

    #[test]
    fn recorder_reroots_nested_forloop_trace() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 1, 2, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 2, 3, 0));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::ForLoop, 0, 2));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.seed.start_pc, 2);
        assert_eq!(artifact.ops.len(), 2);
        assert_eq!(artifact.ops[0].pc, 2);
        assert_eq!(artifact.loop_tail_pc, 3);
    }

    #[test]
    fn recorder_rerecords_to_earlier_backedge_header() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Add, 0, 0, 1, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -3));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 1).unwrap();
        assert_eq!(artifact.seed.start_pc, 0);
        assert_eq!(
            artifact.ops.iter().map(|op| op.pc).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(artifact.loop_tail_pc, 2);
    }

    #[test]
    fn recorder_rerecords_generic_for_body_after_preheader() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Call, 0, 2, 5));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::TForPrep, 0, 2));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Add, 1, 1, 2, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::MmBin, 1, 2, 6));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::TForCall, 0, 2, 0));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::TForLoop, 0, 4));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Close, 0, 0, 0));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::ForLoop, 0, 8));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.seed.start_pc, 3);
        assert_eq!(
            artifact.ops.iter().map(|op| op.pc).collect::<Vec<_>>(),
            vec![3, 5, 6]
        );
        assert_eq!(artifact.exits.len(), 1);
        assert_eq!(artifact.exits[0].guard_pc, 6);
        assert_eq!(artifact.exits[0].exit_pc, 7);
        assert_eq!(artifact.loop_tail_pc, 6);
    }

    #[test]
    fn recorder_follows_guard_continue_target_with_forward_exit_jmp() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abck(OpCode::EqK, 0, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 2));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 3));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 1, 2, 0));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -6));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(
            artifact.ops.iter().map(|op| op.pc).collect::<Vec<_>>(),
            vec![0, 1, 4, 5]
        );
        assert_eq!(artifact.exits.len(), 1);
        assert_eq!(artifact.exits[0].exit_pc, 6);
        assert!(!artifact.exits[0].taken_on_trace);
    }

    #[test]
    fn recorder_follows_guard_fallthrough_arm_through_merge_jump() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abck(OpCode::EqI, 0, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 3));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Add, 1, 1, 2, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::MmBin, 1, 2, 6));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 2));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Sub, 1, 1, 2, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::MmBin, 1, 2, 7));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -8));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(
            artifact.ops.iter().map(|op| op.pc).collect::<Vec<_>>(),
            vec![0, 1, 2, 4, 7]
        );
        assert_eq!(artifact.exits.len(), 1);
        assert_eq!(artifact.exits[0].exit_pc, 5);
        assert!(!artifact.exits[0].taken_on_trace);
        assert_eq!(artifact.loop_tail_pc, 7);
    }

    #[test]
    fn recorder_rejects_mismatched_backedge() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abx(OpCode::ForLoop, 0, 1));

        assert_eq!(
            TraceRecorder::record_root(&chunk as *const LuaProto, 0),
            Err(TraceAbortReason::BackedgeMismatch { target_pc: 1 })
        );
    }

    #[test]
    fn recorder_preserves_quicksort_child_return_side_trace_identity() {
        let chunk = load_bench_quicksort_chunk();
        let quicksort = find_child_proto(&chunk, 77, 98);
        let artifact = TraceRecorder::record_side(quicksort as *const LuaProto, 12).unwrap();

        assert_eq!(artifact.seed.start_pc, 12);
        assert_eq!(artifact.loop_header_pc, 0);
        assert_eq!(artifact.loop_tail_pc, 31);
        assert_eq!(
            artifact
                .ops
                .iter()
                .map(|op| (op.pc, op.opcode))
                .collect::<Vec<_>>(),
            vec![
                (12, OpCode::GetUpval),
                (13, OpCode::Move),
                (14, OpCode::Move),
                (15, OpCode::Move),
                (16, OpCode::Call),
                (17, OpCode::Sub),
                (19, OpCode::Sub),
                (21, OpCode::Lt),
                (22, OpCode::Jmp),
                (23, OpCode::Lt),
                (24, OpCode::Jmp),
                (25, OpCode::GetUpval),
                (26, OpCode::Move),
                (27, OpCode::Move),
                (28, OpCode::Move),
                (29, OpCode::Call),
                (30, OpCode::Move),
                (31, OpCode::Jmp),
            ]
        );
        assert_eq!(
            artifact.exits,
            vec![
                TraceExit {
                    guard_pc: 21,
                    branch_pc: 22,
                    exit_pc: 32,
                    taken_on_trace: false,
                    kind: TraceExitKind::GuardExit,
                },
                TraceExit {
                    guard_pc: 23,
                    branch_pc: 24,
                    exit_pc: 30,
                    taken_on_trace: false,
                    kind: TraceExitKind::GuardExit,
                },
            ]
        );
    }

    #[test]
    fn recorder_records_forward_guard_exit_inside_loop() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 0, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, 1));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Add, 0, 0, 1, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -5));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 4);
        assert_eq!(artifact.exits.len(), 1);
        assert_eq!(artifact.exits[0].guard_pc, 1);
        assert_eq!(artifact.exits[0].branch_pc, 2);
        assert_eq!(artifact.exits[0].exit_pc, 3);
        assert!(artifact.exits[0].taken_on_trace);
        assert_eq!(artifact.exits[0].kind, TraceExitKind::GuardExit);
        assert_eq!(artifact.loop_tail_pc, 4);
    }

    #[test]
    fn recorder_records_repeat_style_backward_guard() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 0, 0, 0, false));
        chunk.code.push(Instruction::create_sj(OpCode::Jmp, -3));

        let artifact = TraceRecorder::record_root(&chunk as *const LuaProto, 0).unwrap();
        assert_eq!(artifact.ops.len(), 3);
        assert_eq!(artifact.exits.len(), 1);
        assert_eq!(artifact.exits[0].guard_pc, 1);
        assert_eq!(artifact.exits[0].branch_pc, 2);
        assert_eq!(artifact.exits[0].exit_pc, 3);
        assert!(artifact.exits[0].taken_on_trace);
        assert_eq!(artifact.loop_tail_pc, 2);
    }

    #[test]
    fn recorder_rejects_guard_without_branch_jmp() {
        let mut chunk = LuaProto::new();
        chunk
            .code
            .push(Instruction::create_abck(OpCode::Test, 0, 0, 0, false));
        chunk
            .code
            .push(Instruction::create_abc(OpCode::Move, 0, 1, 0));

        assert_eq!(
            TraceRecorder::record_root(&chunk as *const LuaProto, 0),
            Err(TraceAbortReason::MissingBranchAfterGuard)
        );
    }
}
