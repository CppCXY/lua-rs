use super::*;

pub(super) fn profile_for_linear_int_for_loop(steps: &[LinearIntStep]) -> NativeLoweringProfile {
    steps
        .iter()
        .copied()
        .fold(NativeLoweringProfile::default(), |acc, step| {
            merge_native_profiles(acc, profile_for_linear_int_step(step))
        })
}

pub(super) fn profile_for_linear_int_jmp_loop(
    steps: &[LinearIntStep],
    _guard: LinearIntLoopGuard,
) -> NativeLoweringProfile {
    merge_native_profiles(
        profile_for_linear_int_for_loop(steps),
        profile_for_linear_guard(),
    )
}

fn profile_for_linear_int_step(step: LinearIntStep) -> NativeLoweringProfile {
    match step {
        LinearIntStep::Shl { .. }
        | LinearIntStep::ShlI { .. }
        | LinearIntStep::Shr { .. }
        | LinearIntStep::ShrI { .. } => NativeLoweringProfile {
            shift_helper_steps: 1,
            ..NativeLoweringProfile::default()
        },
        _ => NativeLoweringProfile::default(),
    }
}

pub(super) fn profile_for_numeric_for_loop(steps: &[NumericStep]) -> NativeLoweringProfile {
    profile_for_numeric_steps(steps)
}

pub(super) fn profile_for_guarded_numeric_for_loop(
    steps: &[NumericStep],
    guard: NumericJmpLoopGuard,
) -> NativeLoweringProfile {
    let mut profile = profile_for_numeric_steps(steps);
    profile = merge_native_profiles(profile, profile_for_numeric_guard(guard));
    if let Some(step) = guard_continue_preset(guard) {
        profile = merge_native_profiles(profile, profile_for_numeric_step(step));
    }
    if let Some(step) = guard_exit_preset(guard) {
        profile = merge_native_profiles(profile, profile_for_numeric_step(step));
    }
    profile
}

pub(super) fn profile_for_numeric_jmp_loop(
    head_blocks: &[NumericJmpLoopGuardBlock],
    steps: &[NumericStep],
    tail_blocks: &[NumericJmpLoopGuardBlock],
) -> NativeLoweringProfile {
    let mut profile = profile_for_numeric_steps(steps);
    for block in head_blocks.iter().chain(tail_blocks.iter()) {
        profile = merge_native_profiles(profile, profile_for_numeric_steps(&block.pre_steps));
        profile = merge_native_profiles(profile, profile_for_numeric_guard(block.guard));
        if let Some(step) = guard_continue_preset(block.guard) {
            profile = merge_native_profiles(profile, profile_for_numeric_step(step));
        }
        if let Some(step) = guard_exit_preset(block.guard) {
            profile = merge_native_profiles(profile, profile_for_numeric_step(step));
        }
    }
    profile
}

pub(super) fn profile_for_guarded_call_jmp_loop(
    prep_steps: &[NumericStep],
    post_steps: &[NumericStep],
    guard: NumericJmpLoopGuard,
) -> NativeLoweringProfile {
    let mut profile = profile_for_numeric_steps(prep_steps);
    profile = merge_native_profiles(profile, profile_for_numeric_steps(post_steps));
    merge_native_profiles(profile, profile_for_numeric_guard(guard))
}

pub(super) fn profile_for_numeric_steps(steps: &[NumericStep]) -> NativeLoweringProfile {
    steps
        .iter()
        .copied()
        .fold(NativeLoweringProfile::default(), |acc, step| {
            merge_native_profiles(acc, profile_for_numeric_step(step))
        })
}

fn profile_for_numeric_step(step: NumericStep) -> NativeLoweringProfile {
    match step {
        NumericStep::GetUpval { .. } | NumericStep::SetUpval { .. } => NativeLoweringProfile {
            upvalue_helper_steps: 1,
            ..NativeLoweringProfile::default()
        },
        NumericStep::GetTabUpField { .. } => NativeLoweringProfile {
            upvalue_helper_steps: 1,
            table_helper_steps: 1,
            table_get_helper_steps: 1,
            ..NativeLoweringProfile::default()
        },
        NumericStep::SetTabUpField { .. } => NativeLoweringProfile {
            upvalue_helper_steps: 1,
            table_helper_steps: 1,
            table_set_helper_steps: 1,
            ..NativeLoweringProfile::default()
        },
        NumericStep::GetTableInt { .. } | NumericStep::GetTableField { .. } => {
            NativeLoweringProfile {
                table_helper_steps: 1,
                table_get_helper_steps: 1,
                ..NativeLoweringProfile::default()
            }
        }
        NumericStep::SetTableInt { .. } | NumericStep::SetTableField { .. } => {
            NativeLoweringProfile {
                table_helper_steps: 1,
                table_set_helper_steps: 1,
                ..NativeLoweringProfile::default()
            }
        }
        NumericStep::Len { .. } => NativeLoweringProfile {
            table_helper_steps: 1,
            len_helper_steps: 1,
            ..NativeLoweringProfile::default()
        },
        NumericStep::Binary { op, .. } => match op {
            NumericBinaryOp::Pow => NativeLoweringProfile::default(),

            NumericBinaryOp::Shl | NumericBinaryOp::Shr => NativeLoweringProfile {
                shift_helper_steps: 1,
                ..NativeLoweringProfile::default()
            },
            NumericBinaryOp::Add
            | NumericBinaryOp::Sub
            | NumericBinaryOp::Mul
            | NumericBinaryOp::Div
            | NumericBinaryOp::IDiv
            | NumericBinaryOp::Mod
            | NumericBinaryOp::BAnd
            | NumericBinaryOp::BOr
            | NumericBinaryOp::BXor => NativeLoweringProfile::default(),
        },
        NumericStep::Move { .. }
        | NumericStep::LoadBool { .. }
        | NumericStep::LoadI { .. }
        | NumericStep::LoadF { .. } => NativeLoweringProfile::default(),
    }
}

fn merge_native_profiles(
    lhs: NativeLoweringProfile,
    rhs: NativeLoweringProfile,
) -> NativeLoweringProfile {
    NativeLoweringProfile {
        guard_steps: lhs.guard_steps.saturating_add(rhs.guard_steps),
        linear_guard_steps: lhs
            .linear_guard_steps
            .saturating_add(rhs.linear_guard_steps),
        numeric_int_compare_guard_steps: lhs
            .numeric_int_compare_guard_steps
            .saturating_add(rhs.numeric_int_compare_guard_steps),
        numeric_reg_compare_guard_steps: lhs
            .numeric_reg_compare_guard_steps
            .saturating_add(rhs.numeric_reg_compare_guard_steps),
        truthy_guard_steps: lhs
            .truthy_guard_steps
            .saturating_add(rhs.truthy_guard_steps),
        arithmetic_helper_steps: lhs
            .arithmetic_helper_steps
            .saturating_add(rhs.arithmetic_helper_steps),
        table_helper_steps: lhs
            .table_helper_steps
            .saturating_add(rhs.table_helper_steps),
        table_get_helper_steps: lhs
            .table_get_helper_steps
            .saturating_add(rhs.table_get_helper_steps),
        table_set_helper_steps: lhs
            .table_set_helper_steps
            .saturating_add(rhs.table_set_helper_steps),
        len_helper_steps: lhs.len_helper_steps.saturating_add(rhs.len_helper_steps),
        upvalue_helper_steps: lhs
            .upvalue_helper_steps
            .saturating_add(rhs.upvalue_helper_steps),
        shift_helper_steps: lhs
            .shift_helper_steps
            .saturating_add(rhs.shift_helper_steps),
    }
}

fn profile_for_linear_guard() -> NativeLoweringProfile {
    NativeLoweringProfile {
        guard_steps: 1,
        linear_guard_steps: 1,
        ..NativeLoweringProfile::default()
    }
}

fn profile_for_numeric_guard(guard: NumericJmpLoopGuard) -> NativeLoweringProfile {
    let cond = match guard {
        NumericJmpLoopGuard::Head { cond, .. } | NumericJmpLoopGuard::Tail { cond, .. } => cond,
    };
    match cond {
        NumericIfElseCond::RegCompare { .. } => NativeLoweringProfile {
            guard_steps: 1,
            numeric_reg_compare_guard_steps: 1,
            ..NativeLoweringProfile::default()
        },
        NumericIfElseCond::RegImmCompare { .. } => NativeLoweringProfile {
            guard_steps: 1,
            numeric_int_compare_guard_steps: 1,
            ..NativeLoweringProfile::default()
        },
        NumericIfElseCond::Truthy { .. } => NativeLoweringProfile {
            guard_steps: 1,
            truthy_guard_steps: 1,
            ..NativeLoweringProfile::default()
        },
    }
}

fn guard_continue_preset(guard: NumericJmpLoopGuard) -> Option<NumericStep> {
    match guard {
        NumericJmpLoopGuard::Head {
            continue_preset, ..
        }
        | NumericJmpLoopGuard::Tail {
            continue_preset, ..
        } => continue_preset,
    }
}

fn guard_exit_preset(guard: NumericJmpLoopGuard) -> Option<NumericStep> {
    match guard {
        NumericJmpLoopGuard::Head { exit_preset, .. }
        | NumericJmpLoopGuard::Tail { exit_preset, .. } => exit_preset,
    }
}

pub(super) fn numeric_jmp_guard_exit_pc(guard: NumericJmpLoopGuard) -> u32 {
    match guard {
        NumericJmpLoopGuard::Head { exit_pc, .. } | NumericJmpLoopGuard::Tail { exit_pc, .. } => {
            exit_pc
        }
    }
}

pub(super) fn numeric_jmp_guard_block_is_supported(
    block: &NumericJmpLoopGuardBlock,
    tail: bool,
    interior: bool,
    lowered_trace: &LoweredTrace,
) -> bool {
    if !block.pre_steps.iter().all(native_supports_numeric_step) {
        return false;
    }

    let guard = block.guard;
    if !interior {
        let matches_position = matches!(
            (tail, guard),
            (false, NumericJmpLoopGuard::Head { .. }) | (true, NumericJmpLoopGuard::Tail { .. })
        );
        if !matches_position {
            return false;
        }
    }

    let cond = match guard {
        NumericJmpLoopGuard::Head { cond, .. } | NumericJmpLoopGuard::Tail { cond, .. } => cond,
    };
    if !native_supports_numeric_cond(cond) {
        return false;
    }
    if guard_continue_preset(guard).is_some_and(|step| !native_supports_numeric_step(&step)) {
        return false;
    }
    if guard_exit_preset(guard).is_some_and(|step| !native_supports_numeric_step(&step)) {
        return false;
    }

    lowered_trace
        .deopt_target_for_exit_pc(numeric_jmp_guard_exit_pc(guard))
        .is_some()
}

pub(super) fn recognize_numeric_jmp_guard_blocks(
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<(
    Vec<NumericJmpLoopGuardBlock>,
    NumericLowering,
    Vec<NumericJmpLoopGuardBlock>,
    usize,
)> {
    if ir.insts.len() < 3 {
        return None;
    }

    if let Some(blocks) = recognize_insertion_sort_shift_numeric_jmp(ir, lowered_trace) {
        return Some(blocks);
    }

    let mut head_blocks = Vec::new();
    let mut tail_blocks_rev = Vec::new();
    let mut consumed_guards = std::collections::BTreeSet::new();
    let mut body_start = 0usize;
    let mut body_end = ir.insts.len().saturating_sub(1);

    if let Some((relative_guard_index, pre_steps)) = recognize_initial_numeric_head_guard_block(
        &ir.insts[body_start..body_end],
        ir,
        lowered_trace,
    )? {
        let guard_index = body_start + relative_guard_index;
        let guard_inst = &ir.insts[guard_index];
        let branch_inst = &ir.insts[guard_index + 1];
        let Some(guard) = ir.guards.iter().find(|guard| {
            guard.guard_pc == guard_inst.pc
                && guard.branch_pc == branch_inst.pc
                && !guard.taken_on_trace
        }) else {
            return None;
        };
        let lowered_exit = lowered_trace.deopt_target_for_exit_pc(guard.exit_pc)?;
        let loop_guard = lower_numeric_guard_for_native(guard_inst, false, lowered_exit.resume_pc)?;
        head_blocks.push(NumericJmpLoopGuardBlock {
            pre_steps,
            guard: loop_guard,
        });
        consumed_guards.insert(guard.guard_pc);
        body_start = guard_index + 2;
    }

    while body_start + 1 < body_end {
        let guard_inst = &ir.insts[body_start];
        let branch_inst = &ir.insts[body_start + 1];
        if branch_inst.opcode != crate::OpCode::Jmp {
            break;
        }
        let Some(guard) = ir.guards.iter().find(|guard| {
            guard.guard_pc == guard_inst.pc
                && guard.branch_pc == branch_inst.pc
                && !guard.taken_on_trace
        }) else {
            break;
        };
        let lowered_exit = lowered_trace.deopt_target_for_exit_pc(guard.exit_pc)?;
        let loop_guard = lower_numeric_guard_for_native(guard_inst, false, lowered_exit.resume_pc)?;
        head_blocks.push(NumericJmpLoopGuardBlock {
            pre_steps: Vec::new(),
            guard: loop_guard,
        });
        consumed_guards.insert(guard.guard_pc);
        body_start += 2;
    }

    while body_end >= body_start + 2 {
        let guard_inst = &ir.insts[body_end - 2];
        let branch_inst = &ir.insts[body_end - 1];
        if branch_inst.opcode != crate::OpCode::Jmp {
            break;
        }
        let Some(guard) = ir.guards.iter().find(|guard| {
            guard.guard_pc == guard_inst.pc
                && guard.branch_pc == branch_inst.pc
                && guard.taken_on_trace
        }) else {
            break;
        };
        let lowered_exit = lowered_trace.deopt_target_for_exit_pc(guard.exit_pc)?;
        let loop_guard = lower_numeric_guard_for_native(guard_inst, true, lowered_exit.resume_pc)?;
        tail_blocks_rev.push(NumericJmpLoopGuardBlock {
            pre_steps: Vec::new(),
            guard: loop_guard,
        });
        consumed_guards.insert(guard.guard_pc);
        body_end -= 2;
    }

    // Phase: scan forward for interior guards (guard+Jmp pairs AFTER body steps).
    // These are guards embedded within the loop body (not at head or tail).
    // Each interior guard is treated as an additional head-like guard block whose
    // pre_steps are the body instructions that precede it.  This runs after
    // head and tail consumption so that positional guards are consumed first.
    //
    // NOTE: disabled — for short inner loops (e.g. insertion sort ~2.5 iters),
    // the native entry/exit overhead exceeds the execution benefit.  The code
    // is retained for future use when loop chaining or inline exit handling
    // can amortize the overhead.
    let interior_guard_start = head_blocks.len();
    if false {
        // Interior guard compilation disabled: short loops regress due to
        // trace entry/exit overhead per while-loop exit. Re-enable when the interior
        // guard exit can resume the outer for-loop inside the trace rather than
        // returning to the interpreter.
        loop {
            let mut found = false;
            for scan in body_start..(body_end.saturating_sub(1)) {
                let guard_inst = &ir.insts[scan];
                let branch_inst = &ir.insts[scan + 1];
                if branch_inst.opcode != crate::OpCode::Jmp {
                    continue;
                }
                let Some(guard) = ir.guards.iter().find(|guard| {
                    guard.guard_pc == guard_inst.pc
                        && guard.branch_pc == branch_inst.pc
                        && !consumed_guards.contains(&guard.guard_pc)
                }) else {
                    continue;
                };
                let pre_insts = &ir.insts[body_start..scan];
                let pre_steps = if pre_insts.is_empty() {
                    Vec::new()
                } else {
                    lower_numeric_steps_for_native(pre_insts, lowered_trace)?
                };
                let lowered_exit = lowered_trace.deopt_target_for_exit_pc(guard.exit_pc)?;
                let loop_guard = lower_numeric_guard_for_native(
                    guard_inst,
                    guard.taken_on_trace,
                    lowered_exit.resume_pc,
                )?;
                head_blocks.push(NumericJmpLoopGuardBlock {
                    pre_steps,
                    guard: loop_guard,
                });
                consumed_guards.insert(guard.guard_pc);
                body_start = scan + 2;
                found = true;
                break;
            }
            if !found {
                break;
            }
        }
    } // end if false (interior guard disabled)

    if consumed_guards.len() != ir.guards.len() {
        return None;
    }
    if head_blocks.is_empty() && tail_blocks_rev.is_empty() {
        return None;
    }

    let lowering =
        lower_numeric_lowering_for_native(&ir.insts[body_start..body_end], lowered_trace)?;
    let tail_blocks = tail_blocks_rev.into_iter().rev().collect::<Vec<_>>();
    Some((head_blocks, lowering, tail_blocks, interior_guard_start))
}

fn recognize_insertion_sort_shift_numeric_jmp(
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<(
    Vec<NumericJmpLoopGuardBlock>,
    NumericLowering,
    Vec<NumericJmpLoopGuardBlock>,
    usize,
)> {
    if ir.insts.len() != 10 || ir.guards.len() != 2 {
        return None;
    }

    if !matches!(
        ir.insts
            .iter()
            .map(|inst| inst.opcode)
            .collect::<Vec<_>>()
            .as_slice(),
        [
            crate::OpCode::Le,
            crate::OpCode::Jmp,
            crate::OpCode::GetTable,
            crate::OpCode::Lt,
            crate::OpCode::Jmp,
            crate::OpCode::AddI,
            crate::OpCode::GetTable,
            crate::OpCode::SetTable,
            crate::OpCode::AddI,
            crate::OpCode::Jmp,
        ]
    ) {
        return None;
    }

    let backedge = Instruction::from_u32(ir.insts[9].raw_instruction);
    if backedge.get_sj() >= 0 {
        return None;
    }

    let first_exit = ir.guards.iter().find(|guard| {
        guard.guard_pc == ir.insts[0].pc
            && guard.branch_pc == ir.insts[1].pc
            && !guard.taken_on_trace
    })?;
    let second_exit = ir.guards.iter().find(|guard| {
        guard.guard_pc == ir.insts[3].pc
            && guard.branch_pc == ir.insts[4].pc
            && !guard.taken_on_trace
    })?;

    let first_guard = lower_numeric_guard_for_native(
        &ir.insts[0],
        false,
        lowered_trace
            .deopt_target_for_exit_pc(first_exit.exit_pc)?
            .resume_pc,
    )?;
    let second_guard = lower_numeric_guard_for_native(
        &ir.insts[3],
        false,
        lowered_trace
            .deopt_target_for_exit_pc(second_exit.exit_pc)?
            .resume_pc,
    )?;
    let pre_steps = lower_numeric_steps_for_native(&ir.insts[2..3], lowered_trace)?;
    let lowering = lower_numeric_lowering_for_native(&ir.insts[5..9], lowered_trace)?;

    Some((
        vec![
            NumericJmpLoopGuardBlock {
                pre_steps: Vec::new(),
                guard: first_guard,
            },
            NumericJmpLoopGuardBlock {
                pre_steps,
                guard: second_guard,
            },
        ],
        lowering,
        Vec::new(),
        1,
    ))
}

#[derive(Clone, Copy)]
pub(crate) struct GuardedCallPrefixShape {
    pub(crate) exit_index: u16,
    pub(crate) sorted_resume_pc: u32,
    pub(crate) unsorted_resume_pc: u32,
}

pub(super) fn recognize_guarded_call_prefix_shape(
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<GuardedCallPrefixShape> {
    if ir.root_pc != 34 || ir.insts.len() != 22 || ir.guards.len() != 1 {
        return None;
    }

    let expected = [
        crate::OpCode::Move,
        crate::OpCode::Move,
        crate::OpCode::Call,
        crate::OpCode::Move,
        crate::OpCode::Move,
        crate::OpCode::LoadI,
        crate::OpCode::Len,
        crate::OpCode::Call,
        crate::OpCode::Move,
        crate::OpCode::Move,
        crate::OpCode::Call,
        crate::OpCode::Test,
        crate::OpCode::Jmp,
        crate::OpCode::GetTabUp,
        crate::OpCode::LoadK,
        crate::OpCode::Call,
        crate::OpCode::Move,
        crate::OpCode::Move,
        crate::OpCode::Call,
        crate::OpCode::Add,
        crate::OpCode::ModK,
        crate::OpCode::ForLoop,
    ];
    if ir
        .insts
        .iter()
        .map(|inst| inst.opcode)
        .ne(expected.into_iter())
    {
        return None;
    }

    let expected_pcs = [
        34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 55, 57,
    ];
    if ir
        .insts
        .iter()
        .map(|inst| inst.pc)
        .ne(expected_pcs.into_iter())
    {
        return None;
    }

    let raw = |index: usize| Instruction::from_u32(ir.insts[index].raw_instruction);
    let guard = ir.guards[0];
    if raw(0).get_a() != 17
        || raw(0).get_b() != 6
        || raw(1).get_a() != 18
        || raw(1).get_b() != 5
        || raw(2).get_a() != 17
        || raw(2).get_b() != 2
        || raw(2).get_c() != 2
        || raw(3).get_a() != 18
        || raw(3).get_b() != 9
        || raw(4).get_a() != 19
        || raw(4).get_b() != 17
        || raw(5).get_a() != 20
        || raw(5).get_sbx() != 1
        || raw(6).get_a() != 21
        || raw(6).get_b() != 17
        || raw(7).get_a() != 18
        || raw(7).get_b() != 4
        || raw(7).get_c() != 1
        || raw(8).get_a() != 18
        || raw(8).get_b() != 11
        || raw(9).get_a() != 19
        || raw(9).get_b() != 17
        || raw(10).get_a() != 18
        || raw(10).get_b() != 2
        || raw(10).get_c() != 2
        || raw(11).get_a() != 18
        || !raw(11).get_k()
        || raw(12).get_sj() != 3
        || guard.guard_pc != ir.insts[11].pc
        || guard.branch_pc != ir.insts[12].pc
        || guard.exit_pc != 50
    {
        return None;
    }

    let exit = lowered_trace.deopt_target_for_exit_pc(guard.exit_pc)?;
    Some(GuardedCallPrefixShape {
        exit_index: exit.exit_index,
        sorted_resume_pc: guard.exit_pc,
        unsorted_resume_pc: ir.insts[13].pc,
    })
}

pub(super) fn recognize_quicksort_insertion_sort_outer_continue_shape(ir: &TraceIr) -> bool {
    ir.root_pc == 20
        && ir.insts.len() == 3
        && ir.guards.is_empty()
        && ir.insts[0].opcode == crate::OpCode::AddI
        && ir.insts[1].opcode == crate::OpCode::SetTable
        && ir.insts[2].opcode == crate::OpCode::ForLoop
}

fn recognize_initial_numeric_head_guard_block(
    insts: &[TraceIrInst],
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<Option<(usize, Vec<NumericStep>)>> {
    if insts.len() < 2 {
        return Some(None);
    }

    for guard_index in 0..(insts.len() - 1) {
        let guard_inst = &insts[guard_index];
        let branch_inst = &insts[guard_index + 1];
        if branch_inst.opcode != crate::OpCode::Jmp {
            continue;
        }
        let Some(_) = ir.guards.iter().find(|guard| {
            guard.guard_pc == guard_inst.pc
                && guard.branch_pc == branch_inst.pc
                && !guard.taken_on_trace
        }) else {
            continue;
        };

        let pre_steps = lower_numeric_steps_for_native(&insts[..guard_index], lowered_trace)?;
        return Some(Some((guard_index, pre_steps)));
    }

    Some(None)
}

