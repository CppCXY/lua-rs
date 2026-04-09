#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TableIntRegion {
    table_value: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TableIntKey {
    region: TableIntRegion,
    index_base_value: u32,
    index_offset: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RegisterAlias {
    root_value: u32,
    offset: i32,
}

#[derive(Clone, Copy, Debug, Default)]
struct TableIntRegionState {
    current_version: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct TableIntKeyState {
    region_version: u32,
    available_value_reg: Option<u32>,
    last_store_output: Option<usize>,
    read_since_last_store: bool,
}

fn fresh_register_value_id(next_value_id: &mut u32) -> u32 {
    let value_id = *next_value_id;
    *next_value_id = next_value_id.saturating_sub(1);
    value_id
}

fn resolve_register_alias(
    register_values: &std::collections::HashMap<u32, u32>,
    register_aliases: &std::collections::HashMap<u32, RegisterAlias>,
    reg: u32,
) -> RegisterAlias {
    if let Some(&alias) = register_aliases.get(&reg) {
        return alias;
    }

    RegisterAlias {
        root_value: register_values.get(&reg).copied().unwrap_or(reg),
        offset: 0,
    }
}

fn reset_register_value(
    register_values: &mut std::collections::HashMap<u32, u32>,
    register_aliases: &mut std::collections::HashMap<u32, RegisterAlias>,
    next_value_id: &mut u32,
    reg: u32,
) {
    register_aliases.remove(&reg);
    register_values.insert(reg, fresh_register_value_id(next_value_id));
}

fn set_register_value(
    register_values: &mut std::collections::HashMap<u32, u32>,
    register_aliases: &mut std::collections::HashMap<u32, RegisterAlias>,
    reg: u32,
    alias: RegisterAlias,
) {
    if alias.offset == 0 {
        register_aliases.remove(&reg);
        register_values.insert(reg, alias.root_value);
    } else {
        register_aliases.insert(reg, alias);
        register_values.remove(&reg);
    }
}

fn find_table_int_alias_reg(
    register_slots: &std::collections::HashMap<u32, TableIntKey>,
    key: TableIntKey,
) -> Option<u32> {
    register_slots
        .iter()
        .find_map(|(&reg, &mapped_key)| (mapped_key == key).then_some(reg))
}

fn clear_table_int_value_register(
    register_slots: &mut std::collections::HashMap<u32, TableIntKey>,
    key_states: &mut std::collections::HashMap<TableIntKey, TableIntKeyState>,
    reg: u32,
) {
    register_slots.remove(&reg);

    for (&key, state) in key_states.iter_mut() {
        if state.available_value_reg == Some(reg) {
            state.available_value_reg = find_table_int_alias_reg(register_slots, key);
        }
    }
}

fn current_table_int_region_version(
    region_states: &mut std::collections::HashMap<TableIntRegion, TableIntRegionState>,
    region: TableIntRegion,
) -> u32 {
    region_states.entry(region).or_default().current_version
}

fn current_table_int_key_state<'a>(
    region_states: &mut std::collections::HashMap<TableIntRegion, TableIntRegionState>,
    key_states: &'a mut std::collections::HashMap<TableIntKey, TableIntKeyState>,
    key: TableIntKey,
) -> &'a mut TableIntKeyState {
    let version = current_table_int_region_version(region_states, key.region);
    let state = key_states.entry(key).or_default();
    if state.region_version != version {
        state.region_version = version;
        state.available_value_reg = None;
        state.last_store_output = None;
        state.read_since_last_store = true;
    }
    state
}

fn set_table_int_key_value_reg(
    register_slots: &mut std::collections::HashMap<u32, TableIntKey>,
    region_states: &mut std::collections::HashMap<TableIntRegion, TableIntRegionState>,
    key_states: &mut std::collections::HashMap<TableIntKey, TableIntKeyState>,
    key: TableIntKey,
    value_reg: u32,
) {
    register_slots.retain(|&reg, mapped_key| *mapped_key != key || reg == value_reg);
    register_slots.insert(value_reg, key);
    current_table_int_key_state(region_states, key_states, key).available_value_reg = Some(value_reg);
}

fn normalized_table_int_key(
    register_values: &std::collections::HashMap<u32, u32>,
    register_aliases: &std::collections::HashMap<u32, RegisterAlias>,
    table: u32,
    index: u32,
) -> TableIntKey {
    let table_alias = resolve_register_alias(register_values, register_aliases, table);
    let index_alias = resolve_register_alias(register_values, register_aliases, index);
    TableIntKey {
        region: TableIntRegion {
            table_value: table_alias.root_value,
        },
        index_base_value: index_alias.root_value,
        index_offset: index_alias.offset,
    }
}

fn numeric_step_reads(step: NumericStep) -> impl Iterator<Item = u32> {
    let regs = match step {
        NumericStep::Move { src, .. } => vec![src],
        NumericStep::LoadBool { .. } | NumericStep::LoadI { .. } | NumericStep::LoadF { .. } => {
            Vec::new()
        }
        NumericStep::GetUpval { .. } => Vec::new(),
        NumericStep::SetUpval { src, .. } => vec![src],
        NumericStep::GetTableInt { table, index, .. } => vec![table, index],
        NumericStep::SetTableInt {
            table,
            index,
            value,
        } => vec![table, index, value],
        NumericStep::Binary { lhs, rhs, .. } => {
            let mut regs = Vec::new();
            if let NumericOperand::Reg(reg) = lhs {
                regs.push(reg);
            }
            if let NumericOperand::Reg(reg) = rhs {
                regs.push(reg);
            }
            regs
        }
    };
    regs.into_iter()
}

fn numeric_step_writes(step: NumericStep) -> impl Iterator<Item = u32> {
    let regs = match step {
        NumericStep::Move { dst, .. }
        | NumericStep::LoadBool { dst, .. }
        | NumericStep::LoadI { dst, .. }
        | NumericStep::LoadF { dst, .. }
        | NumericStep::GetUpval { dst, .. }
        | NumericStep::GetTableInt { dst, .. }
        | NumericStep::Binary { dst, .. } => vec![dst],
        NumericStep::SetUpval { .. } | NumericStep::SetTableInt { .. } => Vec::new(),
    };
    regs.into_iter()
}

fn numeric_step_touches_reg(step: NumericStep, reg: u32) -> bool {
    numeric_step_reads(step).any(|candidate| candidate == reg)
        || numeric_step_writes(step).any(|candidate| candidate == reg)
}

fn resolve_numeric_move_alias(
    move_aliases: &std::collections::HashMap<u32, u32>,
    reg: u32,
) -> u32 {
    let mut current = reg;
    let mut seen = std::collections::HashSet::new();
    while let Some(next) = move_aliases.get(&current).copied() {
        if !seen.insert(current) || next == current {
            break;
        }
        current = next;
    }
    current
}

fn normalize_numeric_operand_alias(
    move_aliases: &std::collections::HashMap<u32, u32>,
    operand: NumericOperand,
) -> NumericOperand {
    match operand {
        NumericOperand::Reg(reg) => NumericOperand::Reg(resolve_numeric_move_alias(move_aliases, reg)),
        NumericOperand::ImmI(_) | NumericOperand::Const(_) => operand,
    }
}

fn prune_dead_pure_numeric_defs(steps: Vec<NumericStep>) -> Vec<NumericStep> {
    let mut live = std::collections::HashSet::<u32>::new();
    let mut killed_by_later_write = std::collections::HashSet::<u32>::new();
    let mut kept = Vec::with_capacity(steps.len());

    for step in steps.into_iter().rev() {
        let reads: Vec<_> = numeric_step_reads(step).collect();
        let writes: Vec<_> = numeric_step_writes(step).collect();
        let keep = match step {
            NumericStep::Move { dst, src } => dst == src || live.contains(&dst),
            NumericStep::LoadI { dst, .. } => live.contains(&dst),
            NumericStep::LoadBool { dst, .. }
            | NumericStep::LoadF { dst, .. } => {
                live.contains(&dst) || !killed_by_later_write.contains(&dst)
            }
            NumericStep::Binary { dst, op, .. } => {
                live.contains(&dst)
                    || !numeric_binary_is_safe_to_drop_when_dead(op)
                    || !killed_by_later_write.contains(&dst)
            }
            NumericStep::GetUpval { .. }
            | NumericStep::SetUpval { .. }
            | NumericStep::GetTableInt { .. }
            | NumericStep::SetTableInt { .. } => true,
        };

        for reg in &reads {
            killed_by_later_write.remove(reg);
        }
        for reg in &writes {
            killed_by_later_write.insert(*reg);
        }

        if keep {
            for reg in &writes {
                live.remove(reg);
            }
            for reg in reads {
                live.insert(reg);
            }
            kept.push(step);
        }
    }

    kept.into_iter().rev().collect()
}

fn can_forward_numeric_binary_across_step(step: NumericStep) -> bool {
    matches!(
        step,
        NumericStep::Move { .. }
            | NumericStep::LoadBool { .. }
            | NumericStep::LoadI { .. }
            | NumericStep::LoadF { .. }
    )
}

fn numeric_binary_is_safe_to_drop_when_dead(op: NumericBinaryOp) -> bool {
    matches!(
        op,
        NumericBinaryOp::Add
            | NumericBinaryOp::Sub
            | NumericBinaryOp::Mul
            | NumericBinaryOp::BAnd
            | NumericBinaryOp::BOr
            | NumericBinaryOp::BXor
            | NumericBinaryOp::Shl
            | NumericBinaryOp::Shr
    )
}

fn numeric_binary_is_single_use_operand(step: NumericStep, current: u32) -> bool {
    match step {
        NumericStep::Binary { lhs, rhs, .. } => {
            matches!(lhs, NumericOperand::Reg(reg) if reg == current)
                || matches!(rhs, NumericOperand::Reg(reg) if reg == current)
        }
        _ => false,
    }
}

fn is_numeric_binary_forward_terminal_consumer(
    step: NumericStep,
    current: u32,
    read_counts: &std::collections::HashMap<u32, usize>,
) -> bool {
    if read_counts.get(&current).copied().unwrap_or(0) != 1 {
        return false;
    }

    match step {
        NumericStep::SetUpval { src, .. } => src == current,
        NumericStep::SetTableInt { value, .. } => value == current,
        NumericStep::Binary { .. } => numeric_binary_is_single_use_operand(step, current),
        _ => false,
    }
}

fn forward_local_numeric_binary_moves(steps: Vec<NumericStep>) -> Vec<NumericStep> {
    let mut read_counts = std::collections::HashMap::<u32, usize>::new();
    for step in &steps {
        for reg in numeric_step_reads(*step) {
            *read_counts.entry(reg).or_insert(0) += 1;
        }
    }

    let mut forwarded = Vec::with_capacity(steps.len());
    let mut index = 0usize;
    'outer: while index < steps.len() {
        if let Some(NumericStep::Binary {
            dst: temp,
            lhs,
            rhs,
            op,
        }) = steps.get(index).copied()
            && read_counts.get(&temp).copied().unwrap_or(0) == 1
        {
            let mut scan = index.saturating_add(1);
            let mut current = temp;
            let mut final_dst = None;
            let mut final_end = None;
            let mut skipped_move_indices = std::collections::HashSet::<usize>::new();
            while let Some(step) = steps.get(scan).copied() {
                match step {
                    NumericStep::Move { dst, src }
                        if src == current
                            && dst != current
                            && read_counts.get(&current).copied().unwrap_or(0) == 1 =>
                    {
                        final_dst = Some(dst);
                        current = dst;
                        skipped_move_indices.insert(scan);
                        scan = scan.saturating_add(1);
                    }
                    _ if current != temp
                        && is_numeric_binary_forward_terminal_consumer(step, current, &read_counts) =>
                    {
                        final_dst = Some(current);
                        final_end = Some(scan.saturating_add(1));
                        break;
                    }
                    _ if can_forward_numeric_binary_across_step(step)
                        && !numeric_step_touches_reg(step, current) =>
                    {
                        scan = scan.saturating_add(1);
                    }
                    _ => break,
                }
            }

            if let Some(dst) = final_dst {
                let end = final_end.unwrap_or(scan);
                forwarded.push(NumericStep::Binary { dst, lhs, rhs, op });
                for (offset, step) in steps[index.saturating_add(1)..end].iter().copied().enumerate() {
                    let step_index = index.saturating_add(1).saturating_add(offset);
                    if !skipped_move_indices.contains(&step_index) {
                        forwarded.push(step);
                    }
                }
                index = end;
                continue 'outer;
            }
        }

        forwarded.push(steps[index]);
        index = index.saturating_add(1);
    }

    forwarded
}

fn run_numeric_forwarding_pass(steps: Vec<NumericStep>) -> Vec<NumericStep> {
    forward_local_numeric_binary_moves(steps)
}

fn run_numeric_normalize_and_prune_pass(steps: Vec<NumericStep>) -> Vec<NumericStep> {
    optimize_numeric_steps(steps)
}

pub(super) fn run_numeric_midend_passes(steps: Vec<NumericStep>) -> Vec<NumericStep> {
    let mut current = steps;
    for _ in 0..4 {
        let next = run_numeric_normalize_and_prune_pass(run_numeric_forwarding_pass(current.clone()));
        if next == current {
            return next;
        }
        current = next;
    }

    current
}

fn optimize_numeric_steps(steps: Vec<NumericStep>) -> Vec<NumericStep> {
    let mut optimized = Vec::with_capacity(steps.len());
    let mut register_values = std::collections::HashMap::<u32, u32>::new();
    let mut register_slots = std::collections::HashMap::<u32, TableIntKey>::new();
    let mut region_states = std::collections::HashMap::<TableIntRegion, TableIntRegionState>::new();
    let mut key_states = std::collections::HashMap::<TableIntKey, TableIntKeyState>::new();
    let mut register_aliases = std::collections::HashMap::<u32, RegisterAlias>::new();
    let mut move_aliases = std::collections::HashMap::<u32, u32>::new();
    let mut next_value_id = u32::MAX;
    let mut read_counts = std::collections::HashMap::<u32, usize>::new();

    for step in &steps {
        for reg in numeric_step_reads(*step) {
            *read_counts.entry(reg).or_insert(0) += 1;
        }
    }

    for step in steps {
        match step {
            NumericStep::Move { dst, src } => {
                let src = resolve_numeric_move_alias(&move_aliases, src);
                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                move_aliases.remove(&dst);
                let resolved = resolve_register_alias(&register_values, &register_aliases, src);
                set_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    dst,
                    resolved,
                );
                move_aliases.insert(dst, src);
                if let Some(key) = register_slots.get(&src).copied() {
                    register_slots.insert(dst, key);
                    current_table_int_key_state(&mut region_states, &mut key_states, key)
                        .available_value_reg = Some(dst);
                }

                if dst != src && read_counts.get(&dst).copied().unwrap_or(0) > 0 {
                    optimized.push(Some(NumericStep::Move { dst, src }));
                }
            }
            NumericStep::LoadBool { dst, value } => {
                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                move_aliases.remove(&dst);
                reset_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    &mut next_value_id,
                    dst,
                );
                optimized.push(Some(NumericStep::LoadBool { dst, value }));
            }
            NumericStep::LoadI { dst, imm } => {
                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                move_aliases.remove(&dst);
                reset_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    &mut next_value_id,
                    dst,
                );
                optimized.push(Some(NumericStep::LoadI { dst, imm }));
            }
            NumericStep::LoadF { dst, imm } => {
                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                move_aliases.remove(&dst);
                reset_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    &mut next_value_id,
                    dst,
                );
                optimized.push(Some(NumericStep::LoadF { dst, imm }));
            }
            NumericStep::GetUpval { dst, upvalue } => {
                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                move_aliases.remove(&dst);
                reset_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    &mut next_value_id,
                    dst,
                );
                optimized.push(Some(NumericStep::GetUpval { dst, upvalue }));
            }
            NumericStep::SetUpval { src, upvalue } => {
                let src = resolve_numeric_move_alias(&move_aliases, src);
                optimized.push(Some(NumericStep::SetUpval { src, upvalue }));
            }
            NumericStep::GetTableInt { dst, table, index } => {
                let table = resolve_numeric_move_alias(&move_aliases, table);
                let index = resolve_numeric_move_alias(&move_aliases, index);
                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                move_aliases.remove(&dst);
                reset_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    &mut next_value_id,
                    dst,
                );

                let key = normalized_table_int_key(&register_values, &register_aliases, table, index);
                let state = current_table_int_key_state(&mut region_states, &mut key_states, key);
                if state.available_value_reg.is_some() || state.last_store_output.is_some() {
                    if state.last_store_output.is_some() {
                        state.read_since_last_store = true;
                    }
                    if let Some(src) = state.available_value_reg {
                        register_slots.insert(dst, key);
                        state.available_value_reg = Some(dst);
                        if src != dst {
                            optimized.push(Some(NumericStep::Move { dst, src }));
                        }
                        continue;
                    }
                }

                optimized.push(Some(NumericStep::GetTableInt { dst, table, index }));
                register_slots.insert(dst, key);
                let state = current_table_int_key_state(&mut region_states, &mut key_states, key);
                state.available_value_reg = Some(dst);
                state.last_store_output = None;
            }
            NumericStep::SetTableInt { table, index, value } => {
                let table = resolve_numeric_move_alias(&move_aliases, table);
                let index = resolve_numeric_move_alias(&move_aliases, index);
                let value = resolve_numeric_move_alias(&move_aliases, value);
                let key = normalized_table_int_key(&register_values, &register_aliases, table, index);
                let existing_state = current_table_int_key_state(&mut region_states, &mut key_states, key);
                if let Some(prev_output) = existing_state.last_store_output
                    && !existing_state.read_since_last_store
                {
                    optimized[prev_output] = None;
                }

                let output_index = optimized.len();
                optimized.push(Some(NumericStep::SetTableInt { table, index, value }));
                set_table_int_key_value_reg(
                    &mut register_slots,
                    &mut region_states,
                    &mut key_states,
                    key,
                    value,
                );
                let state = current_table_int_key_state(&mut region_states, &mut key_states, key);
                state.last_store_output = Some(output_index);
                state.read_since_last_store = false;
            }
            NumericStep::Binary { dst, lhs, rhs, op } => {
                let lhs = normalize_numeric_operand_alias(&move_aliases, lhs);
                let rhs = normalize_numeric_operand_alias(&move_aliases, rhs);
                let affine_alias = if op == NumericBinaryOp::Add {
                    match (lhs, rhs) {
                        (NumericOperand::Reg(src), NumericOperand::ImmI(imm))
                        | (NumericOperand::ImmI(imm), NumericOperand::Reg(src)) => {
                            let resolved = resolve_register_alias(
                                &register_values,
                                &register_aliases,
                                src,
                            );
                            Some(RegisterAlias {
                                root_value: resolved.root_value,
                                offset: resolved.offset.saturating_add(imm),
                            })
                        }
                        _ => None,
                    }
                } else {
                    None
                };

                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                move_aliases.remove(&dst);
                reset_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    &mut next_value_id,
                    dst,
                );

                optimized.push(Some(NumericStep::Binary { dst, lhs, rhs, op }));
                if let Some(alias) = affine_alias {
                    set_register_value(&mut register_values, &mut register_aliases, dst, alias);
                }
            }
        }
    }

    prune_dead_pure_numeric_defs(optimized.into_iter().flatten().collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoopGuardPosition {
    Head,
    Tail,
}

impl LoopGuardPosition {
    fn continue_when(self, branch_k: bool) -> bool {
        match self {
            Self::Head => !branch_k,
            Self::Tail => branch_k,
        }
    }

    fn is_tail(self) -> bool {
        matches!(self, Self::Tail)
    }
}

fn wrap_numeric_jmp_guard(
    position: LoopGuardPosition,
    cond: NumericIfElseCond,
    continue_when: bool,
    continue_preset: Option<NumericStep>,
    exit_preset: Option<NumericStep>,
    exit_pc: u32,
) -> NumericJmpLoopGuard {
    match position {
        LoopGuardPosition::Head => NumericJmpLoopGuard::Head {
            cond,
            continue_when,
            continue_preset,
            exit_preset,
            exit_pc,
        },
        LoopGuardPosition::Tail => NumericJmpLoopGuard::Tail {
            cond,
            continue_when,
            continue_preset,
            exit_preset,
            exit_pc,
        },
    }
}

fn compile_numeric_jmp_guard(
    inst: &TraceIrInst,
    tail: bool,
    exit_pc: u32,
) -> Option<NumericJmpLoopGuard> {
    let raw = Instruction::from_u32(inst.raw_instruction);
    let position = if tail {
        LoopGuardPosition::Tail
    } else {
        LoopGuardPosition::Head
    };
    let (cond, continue_when, continue_preset, exit_preset) = match inst.opcode {
        crate::OpCode::Lt | crate::OpCode::Le => {
            let op = match inst.opcode {
                crate::OpCode::Lt => LinearIntGuardOp::Lt,
                crate::OpCode::Le => LinearIntGuardOp::Le,
                _ => unreachable!(),
            };
            (
                NumericIfElseCond::RegCompare {
                    op,
                    lhs: raw.get_a(),
                    rhs: raw.get_b(),
                },
                position.continue_when(raw.get_k()),
                None,
                None,
            )
        }
        crate::OpCode::Test => (
            NumericIfElseCond::Truthy { reg: raw.get_a() },
            position.continue_when(raw.get_k()),
            None,
            None,
        ),
        crate::OpCode::TestSet => {
            let preset = NumericStep::Move {
                dst: raw.get_a(),
                src: raw.get_b(),
            };
            (
                NumericIfElseCond::Truthy { reg: raw.get_b() },
                position.continue_when(raw.get_k()),
                if position.is_tail() { Some(preset) } else { None },
                if position.is_tail() { None } else { Some(preset) },
            )
        }
        _ => return None,
    };

    Some(wrap_numeric_jmp_guard(
        position,
        cond,
        continue_when,
        continue_preset,
        exit_preset,
        exit_pc,
    ))
}


fn compile_linear_int_steps(insts: &[TraceIrInst], lowered_trace: &LoweredTrace) -> Option<Vec<LinearIntStep>> {
    let mut steps = Vec::with_capacity(insts.len());
    let mut index = 0usize;

    while index < insts.len() {
        let inst = &insts[index];
        let raw = Instruction::from_u32(inst.raw_instruction);
        let step = match inst.opcode {
            crate::OpCode::Move => LinearIntStep::Move {
                dst: raw.get_a(),
                src: raw.get_b(),
            },
            crate::OpCode::LoadI => LinearIntStep::LoadI {
                dst: raw.get_a(),
                imm: raw.get_sbx(),
            },
            crate::OpCode::BNot => LinearIntStep::BNot {
                dst: raw.get_a(),
                src: raw.get_b(),
            },
            crate::OpCode::Add if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::Add {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::AddI => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::AddI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: raw.get_sc(),
                }
            }
            crate::OpCode::AddK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::AddI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: lowered_trace.integer_constant(raw.get_c())?,
                }
            }
            crate::OpCode::Sub if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::Sub {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::SubK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::SubI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: lowered_trace.integer_constant(raw.get_c())?,
                }
            }
            crate::OpCode::Mul if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::Mul {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::MulK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::MulI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: lowered_trace.integer_constant(raw.get_c())?,
                }
            }
            crate::OpCode::IDiv if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::IDiv {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::IDivK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::IDivI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: lowered_trace.integer_constant(raw.get_c())?,
                }
            }
            crate::OpCode::Mod if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::Mod {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::ModK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::ModI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: lowered_trace.integer_constant(raw.get_c())?,
                }
            }
            crate::OpCode::BAnd if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::BAnd {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::BAndK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::BAndI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: lowered_trace.integer_constant(raw.get_c())?,
                }
            }
            crate::OpCode::BOr if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::BOr {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::BOrK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::BOrI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: lowered_trace.integer_constant(raw.get_c())?,
                }
            }
            crate::OpCode::BXor if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::BXor {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::BXorK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::BXorI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: lowered_trace.integer_constant(raw.get_c())?,
                }
            }
            crate::OpCode::Shl if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::Shl {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::ShlI => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::ShlI {
                    dst: raw.get_a(),
                    imm: raw.get_sc(),
                    src: raw.get_b(),
                }
            }
            crate::OpCode::Shr if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::Shr {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::ShrI => {
                consume_fused_arithmetic_companion(insts, &mut index);
                LinearIntStep::ShrI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: raw.get_sc(),
                }
            }
            crate::OpCode::MmBin | crate::OpCode::MmBinI => return None,
            _ => return None,
        };
        steps.push(step);
        index += 1;
    }

    Some(steps)
}

fn consume_fused_arithmetic_companion(insts: &[TraceIrInst], index: &mut usize) {
    if crate::lua_vm::jit::ir::is_fused_arithmetic_metamethod_fallback(
        insts,
        index.saturating_add(1),
    ) {
        *index = index.saturating_add(1);
    }
}

fn ssa_table_int_rewrite_for_pc(
    lowered_trace: &LoweredTrace,
    pc: u32,
) -> Option<SsaTableIntRewrite> {
    lowered_trace
        .ssa_trace
        .instructions
        .iter()
        .find(|instruction| instruction.pc == pc)
        .and_then(|instruction| instruction.table_int_rewrite)
}

fn compile_numeric_steps(insts: &[TraceIrInst], lowered_trace: &LoweredTrace) -> Option<Vec<NumericStep>> {
    let mut steps = Vec::with_capacity(insts.len());
    let mut index = 0usize;

    while index < insts.len() {
        let inst = &insts[index];
        let raw = Instruction::from_u32(inst.raw_instruction);
        let table_int_rewrite = ssa_table_int_rewrite_for_pc(lowered_trace, inst.pc);
        let step = match inst.opcode {
            crate::OpCode::Move => NumericStep::Move {
                dst: raw.get_a(),
                src: raw.get_b(),
            },
            crate::OpCode::GetUpval => NumericStep::GetUpval {
                dst: raw.get_a(),
                upvalue: raw.get_b(),
            },
            crate::OpCode::SetUpval if !raw.get_k() => NumericStep::SetUpval {
                src: raw.get_a(),
                upvalue: raw.get_b(),
            },
            crate::OpCode::GetTable if !raw.get_k() => match table_int_rewrite {
                Some(SsaTableIntRewrite::ForwardFromRegister { reg, .. }) => NumericStep::Move {
                    dst: raw.get_a(),
                    src: reg,
                },
                _ => NumericStep::GetTableInt {
                    dst: raw.get_a(),
                    table: raw.get_b(),
                    index: raw.get_c(),
                },
            },
            crate::OpCode::SetTable if !raw.get_k() => {
                if matches!(table_int_rewrite, Some(SsaTableIntRewrite::DeadStore)) {
                    index += 1;
                    continue;
                }
                NumericStep::SetTableInt {
                    table: raw.get_a(),
                    index: raw.get_b(),
                    value: raw.get_c(),
                }
            }
            crate::OpCode::LoadI => NumericStep::LoadI {
                dst: raw.get_a(),
                imm: raw.get_sbx(),
            },
            crate::OpCode::LoadF => NumericStep::LoadF {
                dst: raw.get_a(),
                imm: raw.get_sbx(),
            },
            crate::OpCode::Add if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Add,
                }
            }
            crate::OpCode::AddI => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::ImmI(raw.get_sc()),
                    op: NumericBinaryOp::Add,
                }
            }
            crate::OpCode::AddK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: numeric_operand_for_constant(lowered_trace, raw.get_c()),
                    op: NumericBinaryOp::Add,
                }
            }
            crate::OpCode::Sub if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Sub,
                }
            }
            crate::OpCode::SubK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: numeric_operand_for_constant(lowered_trace, raw.get_c()),
                    op: NumericBinaryOp::Sub,
                }
            }
            crate::OpCode::Mul if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Mul,
                }
            }
            crate::OpCode::MulK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: numeric_operand_for_constant(lowered_trace, raw.get_c()),
                    op: NumericBinaryOp::Mul,
                }
            }
            crate::OpCode::Div if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Div,
                }
            }
            crate::OpCode::DivK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: numeric_operand_for_constant(lowered_trace, raw.get_c()),
                    op: NumericBinaryOp::Div,
                }
            }
            crate::OpCode::IDiv if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::IDiv,
                }
            }
            crate::OpCode::IDivK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: numeric_operand_for_constant(lowered_trace, raw.get_c()),
                    op: NumericBinaryOp::IDiv,
                }
            }
            crate::OpCode::Mod if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Mod,
                }
            }
            crate::OpCode::ModK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: numeric_operand_for_constant(lowered_trace, raw.get_c()),
                    op: NumericBinaryOp::Mod,
                }
            }
            crate::OpCode::Pow if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Pow,
                }
            }
            crate::OpCode::PowK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: numeric_operand_for_constant(lowered_trace, raw.get_c()),
                    op: NumericBinaryOp::Pow,
                }
            }
            crate::OpCode::BAnd if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::BAnd,
                }
            }
            crate::OpCode::BAndK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: numeric_operand_for_constant(lowered_trace, raw.get_c()),
                    op: NumericBinaryOp::BAnd,
                }
            }
            crate::OpCode::BOr if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::BOr,
                }
            }
            crate::OpCode::BOrK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: numeric_operand_for_constant(lowered_trace, raw.get_c()),
                    op: NumericBinaryOp::BOr,
                }
            }
            crate::OpCode::BXor if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::BXor,
                }
            }
            crate::OpCode::BXorK => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: numeric_operand_for_constant(lowered_trace, raw.get_c()),
                    op: NumericBinaryOp::BXor,
                }
            }
            crate::OpCode::Shl if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Shl,
                }
            }
            crate::OpCode::Shr if !raw.get_k() => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Shr,
                }
            }
            crate::OpCode::ShlI => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::ImmI(raw.get_sc()),
                    rhs: NumericOperand::Reg(raw.get_b()),
                    op: NumericBinaryOp::Shl,
                }
            }
            crate::OpCode::ShrI => {
                consume_fused_arithmetic_companion(insts, &mut index);
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::ImmI(raw.get_sc()),
                    op: NumericBinaryOp::Shr,
                }
            }
            crate::OpCode::MmBin | crate::OpCode::MmBinI | crate::OpCode::MmBinK => return None,
            _ => return None,
        };
        steps.push(step);
        index += 1;
    }

    Some(run_numeric_midend_passes(steps))
}

fn numeric_operand_for_constant(lowered_trace: &LoweredTrace, index: u32) -> NumericOperand {
    lowered_trace
        .integer_constant(index)
        .map(NumericOperand::ImmI)
        .unwrap_or(NumericOperand::Const(index))
}

fn compile_linear_int_guard(
    inst: &TraceIrInst,
    tail: bool,
    exit_pc: u32,
) -> Option<LinearIntLoopGuard> {
    let raw = Instruction::from_u32(inst.raw_instruction);
    let position = if tail {
        LoopGuardPosition::Tail
    } else {
        LoopGuardPosition::Head
    };
    let continue_when = position.continue_when(raw.get_k());

    match inst.opcode {
        crate::OpCode::Lt | crate::OpCode::Le => {
            let op = match inst.opcode {
                crate::OpCode::Lt => LinearIntGuardOp::Lt,
                crate::OpCode::Le => LinearIntGuardOp::Le,
                _ => unreachable!(),
            };
            if position.is_tail() {
                Some(LinearIntLoopGuard::TailRegReg {
                    op,
                    lhs: raw.get_a(),
                    rhs: raw.get_b(),
                    continue_when,
                    exit_pc,
                })
            } else {
                Some(LinearIntLoopGuard::HeadRegReg {
                    op,
                    lhs: raw.get_a(),
                    rhs: raw.get_b(),
                    continue_when,
                    exit_pc,
                })
            }
        }
        crate::OpCode::EqI
        | crate::OpCode::LtI
        | crate::OpCode::LeI
        | crate::OpCode::GtI
        | crate::OpCode::GeI => {
            if raw.get_c() != 0 {
                return None;
            }

            let op = match inst.opcode {
                crate::OpCode::EqI => LinearIntGuardOp::Eq,
                crate::OpCode::LtI => LinearIntGuardOp::Lt,
                crate::OpCode::LeI => LinearIntGuardOp::Le,
                crate::OpCode::GtI => LinearIntGuardOp::Gt,
                crate::OpCode::GeI => LinearIntGuardOp::Ge,
                _ => unreachable!(),
            };

            if position.is_tail() {
                Some(LinearIntLoopGuard::TailRegImm {
                    op,
                    reg: raw.get_a(),
                    imm: raw.get_sb(),
                    continue_when,
                    exit_pc,
                })
            } else {
                Some(LinearIntLoopGuard::HeadRegImm {
                    op,
                    reg: raw.get_a(),
                    imm: raw.get_sb(),
                    continue_when,
                    exit_pc,
                })
            }
        }
        _ => None,
    }
}


