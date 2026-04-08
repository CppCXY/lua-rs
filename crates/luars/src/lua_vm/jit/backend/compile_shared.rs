fn compile_numeric_steps_from_chunk(
    chunk: &LuaProto,
    start_pc: u32,
    end_pc: u32,
) -> Option<Vec<NumericStep>> {
    if start_pc >= end_pc {
        return Some(Vec::new());
    }

    let insts = (start_pc..end_pc)
        .map(|pc| {
            let raw_instruction = chunk.code.get(pc as usize)?.as_u32();
            let opcode = Instruction::from_u32(raw_instruction).get_opcode();
            let kind = match opcode {
                crate::OpCode::MmBin | crate::OpCode::MmBinI | crate::OpCode::MmBinK => {
                    TraceIrInstKind::MetamethodFallback
                }
                crate::OpCode::Jmp => TraceIrInstKind::Branch,
                crate::OpCode::ForLoop | crate::OpCode::TForLoop => TraceIrInstKind::LoopBackedge,
                _ => TraceIrInstKind::Arithmetic,
            };
            Some(TraceIrInst {
                pc,
                opcode,
                raw_instruction,
                kind,
                reads: Vec::new(),
                writes: Vec::new(),
            })
        })
        .collect::<Option<Vec<_>>>()?;

    compile_numeric_steps(&insts)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TableIntSlot {
    table_reg: u32,
    index_base_reg: u32,
    index_offset: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RegisterAlias {
    root_reg: u32,
    offset: i32,
}

#[derive(Clone, Copy, Debug, Default)]
struct TableIntSlotState {
    available_value_reg: Option<u32>,
    last_store_output: Option<usize>,
    read_since_last_store: bool,
}

fn resolve_register_alias(
    register_aliases: &std::collections::HashMap<u32, RegisterAlias>,
    reg: u32,
) -> RegisterAlias {
    let mut current = RegisterAlias {
        root_reg: reg,
        offset: 0,
    };
    while let Some(&next) = register_aliases.get(&current.root_reg) {
        if next.root_reg == current.root_reg {
            break;
        }
        current = RegisterAlias {
            root_reg: next.root_reg,
            offset: current.offset.saturating_add(next.offset),
        };
    }
    current
}

fn invalidate_register_aliases(
    register_aliases: &mut std::collections::HashMap<u32, RegisterAlias>,
    reg: u32,
) {
    register_aliases.remove(&reg);
    let killed_aliases = register_aliases
        .iter()
        .filter_map(|(&alias, &root)| (alias == reg || root.root_reg == reg).then_some(alias))
        .collect::<Vec<_>>();
    for alias in killed_aliases {
        register_aliases.remove(&alias);
    }
}

fn find_table_int_alias_reg(
    register_slots: &std::collections::HashMap<u32, TableIntSlot>,
    slot: TableIntSlot,
) -> Option<u32> {
    register_slots
        .iter()
        .find_map(|(&reg, &mapped_slot)| (mapped_slot == slot).then_some(reg))
}

fn invalidate_table_int_register(
    register_slots: &mut std::collections::HashMap<u32, TableIntSlot>,
    slot_states: &mut std::collections::HashMap<TableIntSlot, TableIntSlotState>,
    reg: u32,
) {
    register_slots.remove(&reg);

    let killed_slots = slot_states
        .keys()
        .copied()
        .filter(|slot| slot.table_reg == reg || slot.index_base_reg == reg)
        .collect::<Vec<_>>();
    for slot in killed_slots {
        slot_states.remove(&slot);
        register_slots.retain(|_, mapped_slot| *mapped_slot != slot);
    }

    for (&slot, state) in slot_states.iter_mut() {
        if state.available_value_reg == Some(reg) {
            state.available_value_reg = find_table_int_alias_reg(register_slots, slot);
        }
    }
}

fn set_table_int_slot_value_reg(
    register_slots: &mut std::collections::HashMap<u32, TableIntSlot>,
    slot_states: &mut std::collections::HashMap<TableIntSlot, TableIntSlotState>,
    slot: TableIntSlot,
    value_reg: u32,
) {
    register_slots.retain(|&reg, mapped_slot| *mapped_slot != slot || reg == value_reg);
    register_slots.insert(value_reg, slot);
    slot_states.entry(slot).or_default().available_value_reg = Some(value_reg);
}

fn optimize_numeric_steps(steps: Vec<NumericStep>) -> Vec<NumericStep> {
    let mut optimized = Vec::with_capacity(steps.len());
    let mut register_slots = std::collections::HashMap::<u32, TableIntSlot>::new();
    let mut slot_states = std::collections::HashMap::<TableIntSlot, TableIntSlotState>::new();
    let mut register_aliases = std::collections::HashMap::<u32, RegisterAlias>::new();

    for step in steps {
        match step {
            NumericStep::Move { dst, src } => {
                invalidate_table_int_register(&mut register_slots, &mut slot_states, dst);
                invalidate_register_aliases(&mut register_aliases, dst);
                optimized.push(Some(NumericStep::Move { dst, src }));
                register_aliases.insert(dst, resolve_register_alias(&register_aliases, src));
                if let Some(slot) = register_slots.get(&src).copied() {
                    register_slots.insert(dst, slot);
                    slot_states.entry(slot).or_default().available_value_reg = Some(dst);
                }
            }
            NumericStep::LoadBool { dst, value } => {
                invalidate_table_int_register(&mut register_slots, &mut slot_states, dst);
                invalidate_register_aliases(&mut register_aliases, dst);
                optimized.push(Some(NumericStep::LoadBool { dst, value }));
            }
            NumericStep::LoadI { dst, imm } => {
                invalidate_table_int_register(&mut register_slots, &mut slot_states, dst);
                invalidate_register_aliases(&mut register_aliases, dst);
                optimized.push(Some(NumericStep::LoadI { dst, imm }));
            }
            NumericStep::LoadF { dst, imm } => {
                invalidate_table_int_register(&mut register_slots, &mut slot_states, dst);
                invalidate_register_aliases(&mut register_aliases, dst);
                optimized.push(Some(NumericStep::LoadF { dst, imm }));
            }
            NumericStep::GetUpval { dst, upvalue } => {
                invalidate_table_int_register(&mut register_slots, &mut slot_states, dst);
                invalidate_register_aliases(&mut register_aliases, dst);
                optimized.push(Some(NumericStep::GetUpval { dst, upvalue }));
            }
            NumericStep::SetUpval { src, upvalue } => {
                optimized.push(Some(NumericStep::SetUpval { src, upvalue }));
            }
            NumericStep::GetTableInt { dst, table, index } => {
                invalidate_table_int_register(&mut register_slots, &mut slot_states, dst);
                invalidate_register_aliases(&mut register_aliases, dst);

                let slot = TableIntSlot {
                    table_reg: resolve_register_alias(&register_aliases, table).root_reg,
                    index_base_reg: resolve_register_alias(&register_aliases, index).root_reg,
                    index_offset: resolve_register_alias(&register_aliases, index).offset,
                };
                if let Some(state) = slot_states.get_mut(&slot) {
                    if state.last_store_output.is_some() {
                        state.read_since_last_store = true;
                    }
                    if let Some(src) = state.available_value_reg {
                        register_slots.insert(dst, slot);
                        state.available_value_reg = Some(dst);
                        if src != dst {
                            optimized.push(Some(NumericStep::Move { dst, src }));
                        }
                        continue;
                    }
                }

                optimized.push(Some(NumericStep::GetTableInt { dst, table, index }));
                register_slots.insert(dst, slot);
                let state = slot_states.entry(slot).or_default();
                state.available_value_reg = Some(dst);
                state.last_store_output = None;
            }
            NumericStep::SetTableInt { table, index, value } => {
                let slot = TableIntSlot {
                    table_reg: resolve_register_alias(&register_aliases, table).root_reg,
                    index_base_reg: resolve_register_alias(&register_aliases, index).root_reg,
                    index_offset: resolve_register_alias(&register_aliases, index).offset,
                };
                if let Some(state) = slot_states.get(&slot)
                    && let Some(prev_output) = state.last_store_output
                    && !state.read_since_last_store
                {
                    optimized[prev_output] = None;
                }

                let output_index = optimized.len();
                optimized.push(Some(NumericStep::SetTableInt { table, index, value }));
                set_table_int_slot_value_reg(&mut register_slots, &mut slot_states, slot, value);
                let state = slot_states.entry(slot).or_default();
                state.last_store_output = Some(output_index);
                state.read_since_last_store = false;
            }
            NumericStep::Binary { dst, lhs, rhs, op } => {
                invalidate_table_int_register(&mut register_slots, &mut slot_states, dst);
                invalidate_register_aliases(&mut register_aliases, dst);
                optimized.push(Some(NumericStep::Binary { dst, lhs, rhs, op }));
                if op == NumericBinaryOp::Add {
                    let alias = match (lhs, rhs) {
                        (NumericOperand::Reg(src), NumericOperand::ImmI(imm))
                        | (NumericOperand::ImmI(imm), NumericOperand::Reg(src)) => {
                            let resolved = resolve_register_alias(&register_aliases, src);
                            Some(RegisterAlias {
                                root_reg: resolved.root_reg,
                                offset: resolved.offset.saturating_add(imm),
                            })
                        }
                        _ => None,
                    };
                    if let Some(alias) = alias
                        && alias.root_reg != dst
                    {
                        register_aliases.insert(dst, alias);
                    }
                }
            }
        }
    }

    optimized.into_iter().flatten().collect()
}

fn lowered_exit_for_guard<'a>(
    lowered_trace: &'a LoweredTrace,
    index: usize,
    guard: TraceIrGuard,
) -> Option<&'a LoweredExit> {
    let exit = lowered_trace.exits.get(index)?;
    if exit.guard_pc != guard.guard_pc || exit.branch_pc != guard.branch_pc || exit.exit_pc != guard.exit_pc {
        return None;
    }
    Some(exit)
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


fn compile_linear_int_steps(insts: &[TraceIrInst]) -> Option<Vec<LinearIntStep>> {
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
            crate::OpCode::Add if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                LinearIntStep::Add {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::AddI => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinI
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_sb() != raw.get_sc().unsigned_abs() as i32 {
                        return None;
                    }
                    index += 1;
                }
                LinearIntStep::AddI {
                    dst: raw.get_a(),
                    src: raw.get_b(),
                    imm: raw.get_sc(),
                }
            }
            crate::OpCode::Sub if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                LinearIntStep::Sub {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
                }
            }
            crate::OpCode::Mul if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                LinearIntStep::Mul {
                    dst: raw.get_a(),
                    lhs: raw.get_b(),
                    rhs: raw.get_c(),
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

fn compile_numeric_steps(insts: &[TraceIrInst]) -> Option<Vec<NumericStep>> {
    let mut steps = Vec::with_capacity(insts.len());
    let mut index = 0usize;

    while index < insts.len() {
        let inst = &insts[index];
        let raw = Instruction::from_u32(inst.raw_instruction);
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
            crate::OpCode::GetTable if !raw.get_k() => NumericStep::GetTableInt {
                dst: raw.get_a(),
                table: raw.get_b(),
                index: raw.get_c(),
            },
            crate::OpCode::SetTable if !raw.get_k() => NumericStep::SetTableInt {
                table: raw.get_a(),
                index: raw.get_b(),
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
            crate::OpCode::Add if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Add,
                }
            }
            crate::OpCode::AddI => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinI
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_sb() != raw.get_sc().unsigned_abs() as i32 {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::ImmI(raw.get_sc()),
                    op: NumericBinaryOp::Add,
                }
            }
            crate::OpCode::AddK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Add,
                }
            }
            crate::OpCode::Sub if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Sub,
                }
            }
            crate::OpCode::SubK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Sub,
                }
            }
            crate::OpCode::Mul if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_a() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Mul,
                }
            }
            crate::OpCode::MulK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Mul,
                }
            }
            crate::OpCode::Div if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Div,
                }
            }
            crate::OpCode::DivK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Div,
                }
            }
            crate::OpCode::IDiv if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::IDiv,
                }
            }
            crate::OpCode::IDivK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::IDiv,
                }
            }
            crate::OpCode::Mod if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Mod,
                }
            }
            crate::OpCode::ModK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Mod,
                }
            }
            crate::OpCode::Pow if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Pow,
                }
            }
            crate::OpCode::PowK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::Pow,
                }
            }
            crate::OpCode::BAnd if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::BAnd,
                }
            }
            crate::OpCode::BAndK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::BAnd,
                }
            }
            crate::OpCode::BOr if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::BOr,
                }
            }
            crate::OpCode::BOrK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::BOr,
                }
            }
            crate::OpCode::BXor if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::BXor,
                }
            }
            crate::OpCode::BXorK => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinK
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Const(raw.get_c()),
                    op: NumericBinaryOp::BXor,
                }
            }
            crate::OpCode::Shl if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Shl,
                }
            }
            crate::OpCode::Shr if !raw.get_k() => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBin
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_b() != raw.get_c() {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::Reg(raw.get_b()),
                    rhs: NumericOperand::Reg(raw.get_c()),
                    op: NumericBinaryOp::Shr,
                }
            }
            crate::OpCode::ShlI => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinI
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_sb() != raw.get_sc().unsigned_abs() as i32 {
                        return None;
                    }
                    index += 1;
                }
                NumericStep::Binary {
                    dst: raw.get_a(),
                    lhs: NumericOperand::ImmI(raw.get_sc()),
                    rhs: NumericOperand::Reg(raw.get_b()),
                    op: NumericBinaryOp::Shl,
                }
            }
            crate::OpCode::ShrI => {
                if let Some(next) = insts.get(index + 1)
                    && next.opcode == crate::OpCode::MmBinI
                {
                    let mm = Instruction::from_u32(next.raw_instruction);
                    if mm.get_a() != raw.get_b() || mm.get_sb() != raw.get_sc().unsigned_abs() as i32 {
                        return None;
                    }
                    index += 1;
                }
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

    Some(optimize_numeric_steps(steps))
}

fn compile_numeric_ifelse_condition(
    inst: &TraceIrInst,
) -> Option<(NumericIfElseCond, bool, Option<NumericStep>, Option<NumericStep>)> {
    let raw = Instruction::from_u32(inst.raw_instruction);
    match inst.opcode {
        crate::OpCode::Lt | crate::OpCode::Le => {
            let op = match inst.opcode {
                crate::OpCode::Lt => LinearIntGuardOp::Lt,
                crate::OpCode::Le => LinearIntGuardOp::Le,
                _ => unreachable!(),
            };

            Some((
                NumericIfElseCond::RegCompare {
                    op,
                    lhs: raw.get_a(),
                    rhs: raw.get_b(),
                },
                !raw.get_k(),
                None,
                None,
            ))
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

            Some((
                NumericIfElseCond::IntCompare {
                    op,
                    reg: raw.get_a(),
                    imm: raw.get_sb(),
                },
                !raw.get_k(),
                None,
                None,
            ))
        }
        crate::OpCode::Test => Some((
            NumericIfElseCond::Truthy { reg: raw.get_a() },
            !raw.get_k(),
            None,
            None,
        )),
        crate::OpCode::TestSet => {
            let preset = NumericStep::Move {
                dst: raw.get_a(),
                src: raw.get_b(),
            };
            let then_on_true = !raw.get_k();
            Some((
                NumericIfElseCond::Truthy { reg: raw.get_b() },
                then_on_true,
                if then_on_true { None } else { Some(preset) },
                if then_on_true { Some(preset) } else { None },
            ))
        }
        _ => None,
    }
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


