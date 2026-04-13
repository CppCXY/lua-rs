use crate::Instruction;

use super::model::{
    LinearIntGuardOp, LinearIntLoopGuard, LinearIntStep, NumericBinaryOp, NumericIfElseCond,
    NumericJmpLoopGuard, NumericLowering, NumericOperand, NumericSelfUpdateValueFlow,
    NumericSelfUpdateValueKind, NumericStep, NumericValueFlowRhs, NumericValueState,
};
use crate::lua_vm::jit::ir::TraceIrInst;
use crate::lua_vm::jit::lowering::{LoweredTrace, SsaTableIntRewrite, TraceValueKind};


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

/// Key for tracking string-keyed table field values in the mid-end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TableFieldSource {
    /// Field loaded via upvalue table (`GetTabUpField`/`SetTabUpField`).
    TabUp(u32),
    /// Field loaded via register-held table (`GetTableField`/`SetTableField`).
    /// The `u32` is the register root_value from alias resolution.
    Register(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TableFieldKey {
    source: TableFieldSource,
    /// Constant pool index for the short-string key.
    key: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct TableFieldKeyState {
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

/// When a register `reg` is overwritten, clear any table-field forwarding state
/// that referenced it as the available value.
fn clear_table_field_value_register(
    field_states: &mut std::collections::HashMap<TableFieldKey, TableFieldKeyState>,
    reg: u32,
) {
    for state in field_states.values_mut() {
        if state.available_value_reg == Some(reg) {
            state.available_value_reg = None;
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
        NumericStep::Len { src, .. } => vec![src],
        NumericStep::GetUpval { .. } | NumericStep::GetTabUpField { .. } => Vec::new(),
        NumericStep::SetUpval { src, .. } => vec![src],
        NumericStep::SetTabUpField { value, .. } => vec![value],
        NumericStep::GetTableInt { table, index, .. } => vec![table, index],
        NumericStep::SetTableInt {
            table,
            index,
            value,
        } => vec![table, index, value],
        NumericStep::GetTableField { table, .. } => vec![table],
        NumericStep::SetTableField { table, value, .. } => vec![table, value],
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
        | NumericStep::Len { dst, .. }
        | NumericStep::GetUpval { dst, .. }
        | NumericStep::GetTabUpField { dst, .. }
        | NumericStep::GetTableInt { dst, .. }
        | NumericStep::GetTableField { dst, .. }
        | NumericStep::Binary { dst, .. } => vec![dst],
        NumericStep::SetUpval { .. }
        | NumericStep::SetTabUpField { .. }
        | NumericStep::SetTableInt { .. }
        | NumericStep::SetTableField { .. } => Vec::new(),
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

fn prune_dead_pure_numeric_defs_with_live_seed(
    steps: Vec<NumericStep>,
    live_seed: &[u32],
) -> Vec<NumericStep> {
    let mut live = live_seed.iter().copied().collect::<std::collections::HashSet<u32>>();
    let mut killed_by_later_write = std::collections::HashSet::<u32>::new();
    let mut kept = Vec::with_capacity(steps.len());

    for step in steps.into_iter().rev() {
        let reads: Vec<_> = numeric_step_reads(step).collect();
        let writes: Vec<_> = numeric_step_writes(step).collect();
        let keep = match step {
            NumericStep::Move { dst, src } => dst == src || live.contains(&dst),
            NumericStep::LoadI { dst, .. } => live.contains(&dst),
            NumericStep::LoadBool { dst, .. }
            | NumericStep::LoadF { dst, .. }
            | NumericStep::Len { dst, .. } => {
                live.contains(&dst) || !killed_by_later_write.contains(&dst)
            }
            NumericStep::Binary { dst, op, .. } => {
                live.contains(&dst)
                    || !numeric_binary_is_safe_to_drop_when_dead(op)
                    || !killed_by_later_write.contains(&dst)
            }
            NumericStep::GetUpval { .. }
            | NumericStep::SetUpval { .. }
            | NumericStep::GetTabUpField { .. }
            | NumericStep::SetTabUpField { .. }
            | NumericStep::GetTableInt { .. }
            | NumericStep::SetTableInt { .. }
            | NumericStep::GetTableField { .. }
            | NumericStep::SetTableField { .. } => true,
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
            | NumericStep::Len { .. }
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
        NumericStep::SetTabUpField { value, .. } => value == current,
        NumericStep::SetTableInt { value, .. } => value == current,
        NumericStep::SetTableField { value, .. } => value == current,
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


pub(super) fn run_numeric_midend_passes_with_live_out(
    steps: Vec<NumericStep>,
    live_out: &[u32],
) -> Vec<NumericStep> {
    let mut current = steps;
    for _ in 0..4 {
        let next = optimize_numeric_steps_with_live_seed(
            run_numeric_forwarding_pass(current.clone()),
            live_out,
        );
        if next == current {
            return next;
        }
        current = next;
    }

    current
}

fn optimize_numeric_steps_with_live_seed(steps: Vec<NumericStep>, live_seed: &[u32]) -> Vec<NumericStep> {
    let mut optimized = Vec::with_capacity(steps.len());
    let mut register_values = std::collections::HashMap::<u32, u32>::new();
    let mut register_slots = std::collections::HashMap::<u32, TableIntKey>::new();
    let mut region_states = std::collections::HashMap::<TableIntRegion, TableIntRegionState>::new();
    let mut key_states = std::collections::HashMap::<TableIntKey, TableIntKeyState>::new();
    let mut register_aliases = std::collections::HashMap::<u32, RegisterAlias>::new();
    let mut move_aliases = std::collections::HashMap::<u32, u32>::new();
    let mut field_states = std::collections::HashMap::<TableFieldKey, TableFieldKeyState>::new();
    let mut next_value_id = u32::MAX;
    let mut read_counts = std::collections::HashMap::<u32, usize>::new();
    let live_seed_set = live_seed.iter().copied().collect::<std::collections::HashSet<u32>>();

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
                clear_table_field_value_register(&mut field_states, dst);
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

                if dst != src
                    && (live_seed_set.contains(&dst)
                        || read_counts.get(&dst).copied().unwrap_or(0) > 0)
                {
                    optimized.push(Some(NumericStep::Move { dst, src }));
                }
            }
            NumericStep::LoadBool { dst, value } => {
                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                clear_table_field_value_register(&mut field_states, dst);
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
                clear_table_field_value_register(&mut field_states, dst);
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
                clear_table_field_value_register(&mut field_states, dst);
                move_aliases.remove(&dst);
                reset_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    &mut next_value_id,
                    dst,
                );
                optimized.push(Some(NumericStep::LoadF { dst, imm }));
            }
            NumericStep::Len { dst, src } => {
                let src = resolve_numeric_move_alias(&move_aliases, src);
                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                clear_table_field_value_register(&mut field_states, dst);
                move_aliases.remove(&dst);
                reset_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    &mut next_value_id,
                    dst,
                );
                optimized.push(Some(NumericStep::Len { dst, src }));
            }
            NumericStep::GetUpval { dst, upvalue } => {
                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                clear_table_field_value_register(&mut field_states, dst);
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
                // Invalidate all TabUp field entries for this upvalue since the
                // upvalue itself is being overwritten.
                field_states.retain(|k, _| {
                    !matches!(k.source, TableFieldSource::TabUp(u) if u == upvalue)
                });
                optimized.push(Some(NumericStep::SetUpval { src, upvalue }));
            }
            NumericStep::GetTabUpField { dst, upvalue, key } => {
                let field_key = TableFieldKey {
                    source: TableFieldSource::TabUp(upvalue),
                    key,
                };
                // Check forwarding BEFORE clearing dst (dst may hold the
                // previous available value for this same key).
                let forward_src = field_states
                    .get(&field_key)
                    .and_then(|s| s.available_value_reg);

                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                clear_table_field_value_register(&mut field_states, dst);
                move_aliases.remove(&dst);
                reset_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    &mut next_value_id,
                    dst,
                );

                if let Some(src) = forward_src {
                    let state = field_states.entry(field_key).or_default();
                    state.available_value_reg = Some(dst);
                    if state.last_store_output.is_some() {
                        state.read_since_last_store = true;
                    }
                    if src != dst {
                        optimized.push(Some(NumericStep::Move { dst, src }));
                    }
                    continue;
                }

                optimized.push(Some(NumericStep::GetTabUpField { dst, upvalue, key }));
                let state = field_states.entry(field_key).or_default();
                state.available_value_reg = Some(dst);
            }
            NumericStep::SetTabUpField { upvalue, key, value } => {
                let value = resolve_numeric_move_alias(&move_aliases, value);
                let field_key = TableFieldKey {
                    source: TableFieldSource::TabUp(upvalue),
                    key,
                };
                // DSE: if previous store to same key with no intervening read.
                if let Some(state) = field_states.get(&field_key) {
                    if let Some(prev_output) = state.last_store_output
                        && !state.read_since_last_store
                    {
                        optimized[prev_output] = None;
                    }
                }
                let output_index = optimized.len();
                optimized.push(Some(NumericStep::SetTabUpField { upvalue, key, value }));
                let state = field_states.entry(field_key).or_default();
                state.available_value_reg = Some(value);
                state.last_store_output = Some(output_index);
                state.read_since_last_store = false;
            }
            NumericStep::GetTableInt { dst, table, index } => {
                let table = resolve_numeric_move_alias(&move_aliases, table);
                let index = resolve_numeric_move_alias(&move_aliases, index);
                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                clear_table_field_value_register(&mut field_states, dst);
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
            NumericStep::GetTableField { dst, table, key } => {
                let table = resolve_numeric_move_alias(&move_aliases, table);
                let table_value = resolve_register_alias(
                    &register_values, &register_aliases, table,
                ).root_value;
                let field_key = TableFieldKey {
                    source: TableFieldSource::Register(table_value),
                    key,
                };
                // Check forwarding BEFORE clearing dst.
                let forward_src = field_states
                    .get(&field_key)
                    .and_then(|s| s.available_value_reg);

                clear_table_int_value_register(&mut register_slots, &mut key_states, dst);
                clear_table_field_value_register(&mut field_states, dst);
                move_aliases.remove(&dst);
                reset_register_value(
                    &mut register_values,
                    &mut register_aliases,
                    &mut next_value_id,
                    dst,
                );

                if let Some(src) = forward_src {
                    let state = field_states.entry(field_key).or_default();
                    state.available_value_reg = Some(dst);
                    if state.last_store_output.is_some() {
                        state.read_since_last_store = true;
                    }
                    if src != dst {
                        optimized.push(Some(NumericStep::Move { dst, src }));
                    }
                    continue;
                }

                optimized.push(Some(NumericStep::GetTableField { dst, table, key }));
                let state = field_states.entry(field_key).or_default();
                state.available_value_reg = Some(dst);
            }
            NumericStep::SetTableField { table, key, value } => {
                let table = resolve_numeric_move_alias(&move_aliases, table);
                let value = resolve_numeric_move_alias(&move_aliases, value);
                let table_value = resolve_register_alias(
                    &register_values, &register_aliases, table,
                ).root_value;
                let field_key = TableFieldKey {
                    source: TableFieldSource::Register(table_value),
                    key,
                };
                // DSE: if previous store to same key with no intervening read.
                if let Some(state) = field_states.get(&field_key) {
                    if let Some(prev_output) = state.last_store_output
                        && !state.read_since_last_store
                    {
                        optimized[prev_output] = None;
                    }
                }
                let output_index = optimized.len();
                optimized.push(Some(NumericStep::SetTableField { table, key, value }));
                let state = field_states.entry(field_key).or_default();
                state.available_value_reg = Some(value);
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
                clear_table_field_value_register(&mut field_states, dst);
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

    prune_dead_pure_numeric_defs_with_live_seed(optimized.into_iter().flatten().collect(), live_seed)
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

            (
                NumericIfElseCond::RegImmCompare {
                    op,
                    reg: raw.get_a(),
                    imm: raw.get_sb(),
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

fn numeric_steps_write_reg_outside_span(
    steps: &[NumericStep],
    reg: u32,
    span_start: usize,
    span_len: usize,
) -> bool {
    steps.iter().enumerate().any(|(index, step)| {
        (index < span_start || index >= span_start.saturating_add(span_len))
            && numeric_step_writes(*step).any(|written| written == reg)
    })
}

fn numeric_steps_touch_reg_outside_span(
    steps: &[NumericStep],
    reg: u32,
    span_start: usize,
    span_len: usize,
) -> bool {
    steps.iter().enumerate().any(|(index, step)| {
        (index < span_start || index >= span_start.saturating_add(span_len))
            && numeric_step_touches_reg(*step, reg)
    })
}

fn detect_integer_self_update_value_flow(
    steps: &[NumericStep],
    lowered_trace: &LoweredTrace,
) -> Option<NumericSelfUpdateValueFlow> {
    let mut detected = None;

    for index in 0..steps.len() {
        let mut candidate = None;
        let mut span_len = 1usize;
        let mut alias_reg = None;

        match steps[index] {
            NumericStep::Binary { dst, lhs, rhs, op } => {
                candidate = Some((dst, lhs, rhs, op));
            }
            NumericStep::Move {
                dst: alias_dst,
                src: alias_src,
            } if index + 1 < steps.len() => {
                if let NumericStep::Binary { dst, lhs, rhs, op } = steps[index + 1] {
                    if matches!(rhs, NumericOperand::Reg(reg) if reg == alias_dst)
                        && alias_dst != dst
                        && alias_src != dst
                    {
                        candidate = Some((dst, lhs, NumericOperand::Reg(alias_src), op));
                        span_len = 2;
                        alias_reg = Some(alias_dst);
                    }
                }
            }
            _ => {}
        }

        let Some((dst, lhs, rhs, op)) = candidate else {
            continue;
        };

        let NumericOperand::Reg(lhs_reg) = lhs else {
            continue;
        };
        if dst != lhs_reg {
            continue;
        }

        let dst_entry_kind = lowered_trace.entry_register_value_kind(dst);
        let dst_stable_kind = lowered_trace.entry_stable_register_value_kind(dst);
        // Reject types that are definitely not integer.  Unknown (None) and
        // Numeric are accepted because the tag guard at the native entry block
        // will verify the actual type at runtime before any loop iteration.
        let reject_integer = matches!(
            dst_entry_kind,
            Some(TraceValueKind::Float | TraceValueKind::Table | TraceValueKind::Boolean | TraceValueKind::Closure)
        ) || matches!(
            dst_stable_kind,
            Some(TraceValueKind::Float | TraceValueKind::Table | TraceValueKind::Boolean | TraceValueKind::Closure)
        );
        if reject_integer {
            continue;
        }

        let rhs = match rhs {
            NumericOperand::ImmI(imm) => NumericValueFlowRhs::ImmI(imm),
            NumericOperand::Reg(rhs_reg) => {
                if rhs_reg == dst {
                    continue;
                }
                let Some(kind) = lowered_trace.entry_stable_register_value_kind(rhs_reg) else {
                    continue;
                };
                if !matches!(kind, TraceValueKind::Integer) {
                    continue;
                }
                NumericValueFlowRhs::StableReg { reg: rhs_reg, kind }
            }
            NumericOperand::Const(_) => continue,
        };

        if !matches!(op, NumericBinaryOp::Add | NumericBinaryOp::Sub | NumericBinaryOp::Mul) {
            continue;
        }

        if numeric_steps_write_reg_outside_span(steps, dst, index, span_len) {
            continue;
        }

        if let NumericValueFlowRhs::StableReg { reg, .. } = rhs {
            if numeric_steps_write_reg_outside_span(steps, reg, index, span_len) {
                continue;
            }
        }

        if let Some(alias_reg) = alias_reg {
            if numeric_steps_touch_reg_outside_span(steps, alias_reg, index, span_len) {
                continue;
            }
        }

        let flow = NumericSelfUpdateValueFlow {
            reg: dst,
            op,
            kind: NumericSelfUpdateValueKind::Integer,
            rhs,
        };

        if detected.replace(flow).is_some() {
            return None;
        }
    }

    detected
}

fn detect_float_self_update_value_flow(
    steps: &[NumericStep],
    lowered_trace: &LoweredTrace,
) -> Option<NumericSelfUpdateValueFlow> {
    let mut detected = None;

    for index in 0..steps.len() {
        let mut candidate = None;
        let mut span_len = 1usize;
        let mut alias_reg = None;

        match steps[index] {
            NumericStep::Binary { dst, lhs, rhs, op } => {
                candidate = Some((dst, lhs, rhs, op));
            }
            NumericStep::Move {
                dst: alias_dst,
                src: alias_src,
            } if index + 1 < steps.len() => {
                if let NumericStep::Binary { dst, lhs, rhs, op } = steps[index + 1] {
                    if matches!(rhs, NumericOperand::Reg(reg) if reg == alias_dst)
                        && alias_dst != dst
                        && alias_src != dst
                    {
                        candidate = Some((dst, lhs, NumericOperand::Reg(alias_src), op));
                        span_len = 2;
                        alias_reg = Some(alias_dst);
                    }
                }
            }
            _ => {}
        }

        let Some((dst, lhs, rhs, op)) = candidate else {
            continue;
        };

        let NumericOperand::Reg(lhs_reg) = lhs else {
            continue;
        };
        if dst != lhs_reg {
            continue;
        }

        let dst_entry_kind = lowered_trace.entry_register_value_kind(dst);
        let dst_stable_kind = lowered_trace.entry_stable_register_value_kind(dst);
        if matches!(
            dst_entry_kind,
            Some(
                TraceValueKind::Integer
                    | TraceValueKind::Table
                    | TraceValueKind::Boolean
                    | TraceValueKind::Closure
            )
        ) || matches!(
            dst_stable_kind,
            Some(
                TraceValueKind::Integer
                    | TraceValueKind::Table
                    | TraceValueKind::Boolean
                    | TraceValueKind::Closure
            )
        ) {
            continue;
        }

        let rhs = match rhs {
            NumericOperand::ImmI(imm) => NumericValueFlowRhs::ImmI(imm),
            NumericOperand::Const(index) => NumericValueFlowRhs::Const(index),
            NumericOperand::Reg(rhs_reg) => {
                if rhs_reg == dst {
                    continue;
                }
                let Some(kind) = lowered_trace.entry_stable_register_value_kind(rhs_reg) else {
                    continue;
                };
                match kind {
                    TraceValueKind::Integer | TraceValueKind::Float => {
                        NumericValueFlowRhs::StableReg { reg: rhs_reg, kind }
                    }
                    TraceValueKind::Unknown
                    | TraceValueKind::Numeric
                    | TraceValueKind::Boolean
                    | TraceValueKind::Table
                    | TraceValueKind::Closure => continue,
                }
            }
        };

        if !matches!(
            op,
            NumericBinaryOp::Add | NumericBinaryOp::Sub | NumericBinaryOp::Mul | NumericBinaryOp::Div
        ) {
            continue;
        }

        if numeric_steps_write_reg_outside_span(steps, dst, index, span_len) {
            continue;
        }

        if let NumericValueFlowRhs::StableReg { reg, .. } = rhs {
            if numeric_steps_write_reg_outside_span(steps, reg, index, span_len) {
                continue;
            }
        }

        if let Some(alias_reg) = alias_reg {
            if numeric_steps_touch_reg_outside_span(steps, alias_reg, index, span_len) {
                continue;
            }
        }

        let flow = NumericSelfUpdateValueFlow {
            reg: dst,
            op,
            kind: NumericSelfUpdateValueKind::Float,
            rhs,
        };

        if detected.replace(flow).is_some() {
            return None;
        }
    }

    detected
}

fn detect_numeric_self_update_value_flow(
    steps: &[NumericStep],
    lowered_trace: &LoweredTrace,
) -> Option<NumericSelfUpdateValueFlow> {
    if let Some(flow) = detect_integer_self_update_value_flow(steps, lowered_trace) {
        return Some(flow);
    }

    if let Some(flow) = detect_float_self_update_value_flow(steps, lowered_trace) {
        return Some(flow);
    }

    let (dst, lhs, rhs, op) = match steps {
        [NumericStep::Binary { dst, lhs, rhs, op }] => (*dst, *lhs, *rhs, *op),
        [
            NumericStep::Move {
                dst: alias_dst,
                src: alias_src,
            },
            NumericStep::Binary { dst, lhs, rhs, op },
        ] if matches!(rhs, NumericOperand::Reg(reg) if *reg == *alias_dst)
            && *alias_dst != *dst
            && *alias_src != *dst => {
                (*dst, *lhs, NumericOperand::Reg(*alias_src), *op)
            }
        _ => return None,
    };

    let NumericOperand::Reg(lhs_reg) = lhs else {
        return None;
    };
    if dst != lhs_reg {
        return None;
    }

    let dst_entry_kind = lowered_trace.entry_register_value_kind(dst);
    let dst_stable_kind = lowered_trace.entry_stable_register_value_kind(dst);
    if matches!(
        dst_entry_kind,
        Some(
            TraceValueKind::Table
                | TraceValueKind::Boolean
                | TraceValueKind::Closure
        )
    ) || matches!(
        dst_stable_kind,
        Some(
            TraceValueKind::Table
                | TraceValueKind::Boolean
                | TraceValueKind::Closure
        )
    ) {
        return None;
    }

    let exact_integer_dst = matches!(dst_entry_kind, Some(TraceValueKind::Integer))
        || matches!(dst_stable_kind, Some(TraceValueKind::Integer));

    let rhs = match rhs {
        NumericOperand::ImmI(imm) => NumericValueFlowRhs::ImmI(imm),
        NumericOperand::Const(index) => NumericValueFlowRhs::Const(index),
        NumericOperand::Reg(rhs_reg) => {
            if rhs_reg == dst {
                return None;
            }
            let kind = lowered_trace.entry_stable_register_value_kind(rhs_reg)?;
            match kind {
                TraceValueKind::Integer | TraceValueKind::Float => {
                    NumericValueFlowRhs::StableReg { reg: rhs_reg, kind }
                }
                TraceValueKind::Unknown
                | TraceValueKind::Numeric
                | TraceValueKind::Boolean
                | TraceValueKind::Table
                | TraceValueKind::Closure => return None,
            }
        }
    };

    let integer_rhs = matches!(rhs, NumericValueFlowRhs::ImmI(_))
        || matches!(
            rhs,
            NumericValueFlowRhs::StableReg {
                kind: TraceValueKind::Integer,
                ..
            }
        );

    if exact_integer_dst
        && integer_rhs
        && matches!(
            op,
            NumericBinaryOp::Add | NumericBinaryOp::Sub | NumericBinaryOp::Mul
        )
    {
        return Some(NumericSelfUpdateValueFlow {
            reg: dst,
            op,
            kind: NumericSelfUpdateValueKind::Integer,
            rhs,
        });
    }

    if !matches!(
        dst_entry_kind,
        Some(TraceValueKind::Integer)
            | Some(TraceValueKind::Table)
            | Some(TraceValueKind::Boolean)
            | Some(TraceValueKind::Closure)
    ) && !matches!(
        dst_stable_kind,
        Some(TraceValueKind::Integer)
            | Some(TraceValueKind::Table)
            | Some(TraceValueKind::Boolean)
            | Some(TraceValueKind::Closure)
    ) && matches!(
        op,
        NumericBinaryOp::Add | NumericBinaryOp::Sub | NumericBinaryOp::Mul | NumericBinaryOp::Div
    ) && matches!(
        rhs,
        NumericValueFlowRhs::ImmI(_)
            | NumericValueFlowRhs::Const(_)
            | NumericValueFlowRhs::StableReg {
                kind: TraceValueKind::Integer,
                ..
            }
            | NumericValueFlowRhs::StableReg {
                kind: TraceValueKind::Float,
                ..
            }
    ) {
        return Some(NumericSelfUpdateValueFlow {
            reg: dst,
            op,
            kind: NumericSelfUpdateValueKind::Float,
            rhs,
        });
    }

    None
}

pub(super) fn build_numeric_value_state(steps: &[NumericStep], lowered_trace: &LoweredTrace) -> NumericValueState {
    NumericValueState {
        self_update: detect_numeric_self_update_value_flow(steps, lowered_trace),
    }
}

/// Extract loop-invariant steps from the loop body into a prologue that runs once
/// before the loop, and detect cross-iteration table field forwarding opportunities.
///
/// Returns `(prologue_steps, body_steps)`.
///
/// Phase 1 – LICM (Loop-Invariant Code Motion):
///   A `GetUpval(dst, upvalue)` is hoisted if:
///   1. No `SetUpval` for `upvalue` exists in the body.
///   2. No other step writes to `dst`.
///
///   A `GetTabUpField(dst, upvalue, key)` is hoisted if:
///   1. No `SetUpval` for `upvalue` exists in the body.
///   2. No `SetTabUpField` for the same `upvalue` exists in the body.
///   3. No other step writes to `dst`.
///
///   After hoisting those roots, a `GetTableField(dst, table, key)` is hoisted
///   if `table` is loop-invariant (because it is entry-stable `Table` or was
///   itself hoisted into the prologue), no `SetTableField(table, key, ..)`
///   exists in the body, and no other step writes to `dst`.
///
///   A `GetTableInt(dst, table, index)` is hoisted if both `table` and `index`
///   are loop-invariant (entry-stable or already hoisted) and no
///   `SetTableInt(table, index, ..)` exists in the body, and no other step
///   writes to `dst`.
///
/// Phase 2 – Cross-iteration carried-read forwarding:
///   If the body ends with `SetUpval(src, upvalue)` and begins with
///   `GetUpval(dst, upvalue)` where `dst == src`, the `GetUpval` can be moved
///   into the prologue because the carried value from the previous iteration's
///   `SetUpval` already resides in `dst`.
///
///   If the body ends with `SetTabUpField(upvalue, key, value)` and begins with
///   `GetTabUpField(dst, upvalue, key)` where `dst == value`, the
///   `GetTabUpField` can be moved into the prologue because the carried value
///   from the previous iteration's `SetTabUpField` already resides in `dst`.
///
///   If the body ends with `SetTableField(table, key, value)` and begins with
///   `GetTableField(dst, table, key)` where `dst == value`, the `GetTableField`
///   can be moved into the prologue because the carried value from the previous
///   iteration's `SetTableField` already resides in `dst`.
///
///   If the body ends with `SetTableInt(table, index, value)` and begins with
///   `GetTableInt(dst, table, index)` where `dst == value`, the `GetTableInt`
///   can be moved into the prologue when both `table` and `index` are
///   loop-invariant, because the carried value from the previous iteration's
///   `SetTableInt` already resides in `dst`.
pub(super) fn extract_loop_prologue(
    steps: &[NumericStep],
    lowered_trace: &LoweredTrace,
) -> (Vec<NumericStep>, Vec<NumericStep>) {
    let mut prologue = Vec::new();
    let mut body: Vec<NumericStep> = steps.to_vec();

    // Phase 1: LICM – hoist invariant GetUpval/GetTabUpField steps.
    let mut hoist_indices = Vec::new();
    for (i, step) in body.iter().enumerate() {
        match *step {
            NumericStep::GetUpval { dst, upvalue } => {
                let has_set_upval = body.iter().any(|s| {
                    matches!(s, NumericStep::SetUpval { upvalue: up, .. } if *up == upvalue)
                });
                if has_set_upval {
                    continue;
                }
                let other_writes_dst = body.iter().enumerate().any(|(j, s)| {
                    j != i && numeric_step_writes(*s).any(|w| w == dst)
                });
                if other_writes_dst {
                    continue;
                }
                hoist_indices.push(i);
            }
            NumericStep::GetTabUpField {
                dst,
                upvalue,
                key: _,
            } => {
                let has_set_upval = body.iter().any(|s| {
                    matches!(s, NumericStep::SetUpval { upvalue: up, .. } if *up == upvalue)
                });
                if has_set_upval {
                    continue;
                }
                let has_set_tab_up = body.iter().any(|s| {
                    matches!(s, NumericStep::SetTabUpField { upvalue: up, .. } if *up == upvalue)
                });
                if has_set_tab_up {
                    continue;
                }
                let other_writes_dst = body.iter().enumerate().any(|(j, s)| {
                    j != i && numeric_step_writes(*s).any(|w| w == dst)
                });
                if other_writes_dst {
                    continue;
                }
                hoist_indices.push(i);
            }
            _ => {}
        }
    }
    for &i in hoist_indices.iter().rev() {
        prologue.push(body.remove(i));
    }
    prologue.reverse();

    let mut hoist_indices = Vec::new();
    for (i, step) in body.iter().enumerate() {
        match *step {
            NumericStep::GetTableField { dst, table, key } => {
                let table_is_loop_invariant = prologue.iter().any(|step| {
                    matches!(
                        step,
                        NumericStep::GetUpval { dst, .. } if *dst == table
                    ) || matches!(
                        step,
                        NumericStep::GetTabUpField { dst, .. } if *dst == table
                    ) || matches!(
                        step,
                        NumericStep::GetTableField { dst, .. } if *dst == table
                    )
                }) || matches!(
                    lowered_trace.entry_stable_register_value_kind(table),
                    Some(TraceValueKind::Table)
                );
                if !table_is_loop_invariant {
                    continue;
                }
                let has_conflicting_store = body.iter().any(|s| {
                    matches!(
                        s,
                        NumericStep::SetTableField {
                            table: set_table,
                            key: set_key,
                            ..
                        } if *set_table == table && *set_key == key
                    )
                });
                if has_conflicting_store {
                    continue;
                }
                let other_writes_dst = body.iter().enumerate().any(|(j, s)| {
                    j != i && numeric_step_writes(*s).any(|w| w == dst)
                });
                if other_writes_dst {
                    continue;
                }
                hoist_indices.push(i);
            }
            NumericStep::GetTableInt {
                dst,
                table,
                index,
            } => {
                let table_is_loop_invariant = prologue.iter().any(|step| {
                    matches!(
                        step,
                        NumericStep::GetUpval { dst, .. } if *dst == table
                    ) || matches!(
                        step,
                        NumericStep::GetTabUpField { dst, .. } if *dst == table
                    ) || matches!(
                        step,
                        NumericStep::GetTableField { dst, .. } if *dst == table
                    ) || matches!(
                        step,
                        NumericStep::GetTableInt { dst, .. } if *dst == table
                    )
                }) || matches!(
                    lowered_trace.entry_stable_register_value_kind(table),
                    Some(TraceValueKind::Table)
                );
                let index_is_loop_invariant = prologue.iter().any(|step| {
                    matches!(
                        step,
                        NumericStep::GetUpval { dst, .. } if *dst == index
                    ) || matches!(
                        step,
                        NumericStep::GetTableInt { dst, .. } if *dst == index
                    ) || matches!(
                        step,
                        NumericStep::GetTableField { dst, .. } if *dst == index
                    )
                }) || matches!(
                    lowered_trace.entry_stable_register_value_kind(index),
                    Some(TraceValueKind::Integer)
                );
                if !table_is_loop_invariant || !index_is_loop_invariant {
                    continue;
                }
                let has_conflicting_store = body.iter().any(|s| {
                    matches!(
                        s,
                        NumericStep::SetTableInt {
                            table: set_table,
                            index: set_index,
                            ..
                        } if *set_table == table && *set_index == index
                    )
                });
                if has_conflicting_store {
                    continue;
                }
                let other_writes_dst = body.iter().enumerate().any(|(j, s)| {
                    j != i && numeric_step_writes(*s).any(|w| w == dst)
                });
                if other_writes_dst {
                    continue;
                }
                hoist_indices.push(i);
            }
            _ => {}
        }
    }
    for &i in hoist_indices.iter().rev() {
        prologue.push(body.remove(i));
    }

    // Phase 2a: cross-iteration upvalue forwarding.
    if body.len() >= 2 {
        let first = body[0];
        let last = body[body.len() - 1];
        if let (
            NumericStep::GetUpval {
                dst: get_dst,
                upvalue: get_upvalue,
            },
            NumericStep::SetUpval {
                src: set_src,
                upvalue: set_upvalue,
            },
        ) = (first, last)
        {
            if get_upvalue == set_upvalue
                && get_dst == set_src
                && !body[..body.len() - 1].iter().any(|step| {
                    matches!(
                        step,
                        NumericStep::SetUpval { upvalue, .. } if *upvalue == get_upvalue
                    )
                })
            {
                let get_step = body.remove(0);
                prologue.push(get_step);
            }
        }
    }

    // Phase 2b: cross-iteration tabup field forwarding.
    if body.len() >= 2 {
        let first = body[0];
        let last = body[body.len() - 1];
        if let (
            NumericStep::GetTabUpField {
                dst: get_dst,
                upvalue: get_upvalue,
                key: get_key,
            },
            NumericStep::SetTabUpField {
                upvalue: set_upvalue,
                key: set_key,
                value: set_value,
            },
        ) = (first, last)
        {
            if get_upvalue == set_upvalue
                && get_key == set_key
                && get_dst == set_value
                && !body[..body.len() - 1].iter().any(|step| {
                    matches!(
                        step,
                        NumericStep::SetUpval { upvalue, .. } if *upvalue == get_upvalue
                    ) || matches!(
                        step,
                        NumericStep::SetTabUpField { upvalue, key, .. }
                            if *upvalue == get_upvalue && *key == get_key
                    )
                })
            {
                let get_step = body.remove(0);
                prologue.push(get_step);
            }
        }
    }

    // Phase 2c: cross-iteration table field forwarding.
    // This is safe when the table register is loop-invariant, either because a
    // GetTabUpField feeding it was hoisted into the prologue or because the
    // register is entry-stable and known to hold a table.
    if body.len() >= 2 {
        let first = body[0];
        let last = body[body.len() - 1];
        if let (
            NumericStep::GetTableField {
                dst: get_dst,
                table: get_table,
                key: get_key,
            },
            NumericStep::SetTableField {
                table: set_table,
                key: set_key,
                value: set_value,
            },
        ) = (first, last)
        {
            let table_is_loop_invariant = prologue.iter().any(|step| {
                matches!(
                    step,
                    NumericStep::GetUpval { dst, .. } if *dst == get_table
                ) || matches!(
                    step,
                    NumericStep::GetTabUpField { dst, .. } if *dst == get_table
                )
            }) || matches!(
                lowered_trace.entry_stable_register_value_kind(get_table),
                Some(TraceValueKind::Table)
            );
            if get_table == set_table
                && get_key == set_key
                && get_dst == set_value
                && table_is_loop_invariant
                && !body
                    .iter()
                    .any(|s| numeric_step_writes(*s).any(|w| w == get_table))
            {
                let get_step = body.remove(0);
                prologue.push(get_step);
            }
        }
    }

    // Phase 2d: cross-iteration table-int forwarding.
    if body.len() >= 2 {
        let first = body[0];
        let last = body[body.len() - 1];
        if let (
            NumericStep::GetTableInt {
                dst: get_dst,
                table: get_table,
                index: get_index,
            },
            NumericStep::SetTableInt {
                table: set_table,
                index: set_index,
                value: set_value,
            },
        ) = (first, last)
        {
            let table_is_loop_invariant = prologue.iter().any(|step| {
                matches!(
                    step,
                    NumericStep::GetUpval { dst, .. } if *dst == get_table
                ) || matches!(
                    step,
                    NumericStep::GetTabUpField { dst, .. } if *dst == get_table
                )
            }) || matches!(
                lowered_trace.entry_stable_register_value_kind(get_table),
                Some(TraceValueKind::Table)
            );
            let index_is_loop_invariant = matches!(
                lowered_trace.entry_stable_register_value_kind(get_index),
                Some(TraceValueKind::Integer)
            );
            if get_table == set_table
                && get_index == set_index
                && get_dst == set_value
                && table_is_loop_invariant
                && index_is_loop_invariant
                && !body
                    .iter()
                    .any(|s| numeric_step_writes(*s).any(|w| w == get_table || w == get_index))
            {
                let get_step = body.remove(0);
                prologue.push(get_step);
            }
        }
    }

    (prologue, body)
}

fn compile_numeric_lowering(
    insts: &[TraceIrInst],
    lowered_trace: &LoweredTrace,
) -> Option<NumericLowering> {
    compile_numeric_lowering_with_live_out(insts, lowered_trace, &[])
}

fn compile_numeric_lowering_with_live_out(
    insts: &[TraceIrInst],
    lowered_trace: &LoweredTrace,
    live_out: &[u32],
) -> Option<NumericLowering> {
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
            crate::OpCode::GetField => NumericStep::GetTableField {
                dst: raw.get_a(),
                table: raw.get_b(),
                key: raw.get_c(),
            },
            crate::OpCode::GetTabUp => NumericStep::GetTabUpField {
                dst: raw.get_a(),
                upvalue: raw.get_b(),
                key: raw.get_c(),
            },
            crate::OpCode::SetField if !raw.get_k() => NumericStep::SetTableField {
                table: raw.get_a(),
                key: raw.get_b(),
                value: raw.get_c(),
            },
            crate::OpCode::SetTabUp if !raw.get_k() => NumericStep::SetTabUpField {
                upvalue: raw.get_a(),
                key: raw.get_b(),
                value: raw.get_c(),
            },
            crate::OpCode::LoadI => NumericStep::LoadI {
                dst: raw.get_a(),
                imm: raw.get_sbx(),
            },
            crate::OpCode::LoadF => NumericStep::LoadF {
                dst: raw.get_a(),
                imm: raw.get_sbx(),
            },
            crate::OpCode::Len => NumericStep::Len {
                dst: raw.get_a(),
                src: raw.get_b(),
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

    let steps = run_numeric_midend_passes_with_live_out(steps, live_out);
    let value_state = build_numeric_value_state(&steps, lowered_trace);
    Some(NumericLowering { steps, value_state })
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


pub(super) fn lower_linear_int_steps_for_native(
    insts: &[TraceIrInst],
    lowered_trace: &LoweredTrace,
) -> Option<Vec<LinearIntStep>> {
    compile_linear_int_steps(insts, lowered_trace)
}

pub(super) fn lower_linear_int_guard_for_native(
    inst: &TraceIrInst,
    tail: bool,
    exit_pc: u32,
) -> Option<LinearIntLoopGuard> {
    compile_linear_int_guard(inst, tail, exit_pc)
}

pub(super) fn lower_numeric_steps_for_native(
    insts: &[TraceIrInst],
    lowered_trace: &LoweredTrace,
) -> Option<Vec<NumericStep>> {
    compile_numeric_lowering(insts, lowered_trace).map(|lowering| lowering.steps)
}

pub(super) fn lower_numeric_steps_for_native_with_live_out(
    insts: &[TraceIrInst],
    lowered_trace: &LoweredTrace,
    live_out: &[u32],
) -> Option<Vec<NumericStep>> {
    compile_numeric_lowering_with_live_out(insts, lowered_trace, live_out)
        .map(|lowering| lowering.steps)
}

pub(super) fn lower_numeric_lowering_for_native(
    insts: &[TraceIrInst],
    lowered_trace: &LoweredTrace,
) -> Option<NumericLowering> {
    compile_numeric_lowering(insts, lowered_trace)
}

pub(super) fn lower_numeric_guard_for_native(
    inst: &TraceIrInst,
    tail: bool,
    exit_pc: u32,
) -> Option<NumericJmpLoopGuard> {
    compile_numeric_jmp_guard(inst, tail, exit_pc)
}
