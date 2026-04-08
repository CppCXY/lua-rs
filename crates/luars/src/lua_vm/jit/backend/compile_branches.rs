fn compile_numeric_ifelse_forloop(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() < 6 {
        return None;
    }

    let loop_backedge = ir.insts.last()?;
    if loop_backedge.opcode != crate::OpCode::ForLoop {
        return None;
    }

    let cond_index = ir.insts.iter().position(|inst| {
        matches!(
            inst.opcode,
            crate::OpCode::EqI
                | crate::OpCode::LtI
                | crate::OpCode::LeI
                | crate::OpCode::GtI
                | crate::OpCode::GeI
                | crate::OpCode::Test
                | crate::OpCode::TestSet
        )
    })?;
    if cond_index + 4 >= ir.insts.len() - 1 {
        return None;
    }

    let else_jump = &ir.insts[cond_index + 1];
    if else_jump.opcode != crate::OpCode::Jmp {
        return None;
    }
    let else_jump_inst = Instruction::from_u32(else_jump.raw_instruction);
    if else_jump_inst.get_sj() <= 0 {
        return None;
    }

    let merge_jump_index = cond_index + 1 + else_jump_inst.get_sj() as usize;
    if merge_jump_index <= cond_index + 1 || merge_jump_index >= ir.insts.len() - 1 {
        return None;
    }
    let merge_jump = &ir.insts[merge_jump_index];
    if merge_jump.opcode != crate::OpCode::Jmp {
        return None;
    }
    let merge_jump_inst = Instruction::from_u32(merge_jump.raw_instruction);
    if merge_jump_inst.get_sj() <= 0 {
        return None;
    }

    let pre_steps = compile_numeric_steps(&ir.insts[..cond_index])?;
    let then_steps = compile_numeric_steps(&ir.insts[cond_index + 2..merge_jump_index])?;
    let else_steps = compile_numeric_steps(&ir.insts[merge_jump_index + 1..ir.insts.len() - 1])?;
    if then_steps.is_empty() || else_steps.is_empty() {
        return None;
    }

    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);
    let (cond, then_on_true, then_preset, else_preset) =
        compile_numeric_ifelse_condition(&ir.insts[cond_index])?;

    Some(CompiledTraceExecutor::NumericIfElseForLoop {
        loop_reg: loop_inst.get_a(),
        pre_steps,
        cond,
        then_preset,
        else_preset,
        then_steps,
        else_steps,
        then_on_true,
    })
}

fn compile_guarded_numeric_ifelse_forloop(
    artifact: &TraceArtifact,
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<CompiledTraceExecutor> {
    if ir.insts.len() < 5 || ir.guards.len() != 1 {
        return None;
    }

    let guard = ir.guards[0];
    if guard.kind != TraceIrGuardKind::SideExit || guard.taken_on_trace {
        return None;
    }
    let lowered_exit = lowered_exit_for_guard(lowered_trace, 0, guard)?;

    let loop_backedge = ir.insts.last()?;
    if loop_backedge.opcode != crate::OpCode::ForLoop {
        return None;
    }

    let cond_index = ir.insts.iter().position(|inst| {
        matches!(
            inst.opcode,
            crate::OpCode::EqI
                | crate::OpCode::LtI
                | crate::OpCode::LeI
                | crate::OpCode::GtI
                | crate::OpCode::GeI
                | crate::OpCode::Test
                | crate::OpCode::TestSet
        )
    })?;
    if cond_index + 3 >= ir.insts.len() {
        return None;
    }

    let branch_inst = &ir.insts[cond_index + 1];
    if branch_inst.opcode != crate::OpCode::Jmp {
        return None;
    }

    let merge_jump_index = ir.insts.len() - 2;
    let merge_jump = &ir.insts[merge_jump_index];
    if merge_jump.opcode != crate::OpCode::Jmp {
        return None;
    }

    let pre_steps = compile_numeric_steps(&ir.insts[..cond_index])?;
    let then_steps = compile_numeric_steps(&ir.insts[cond_index + 2..merge_jump_index])?;
    if then_steps.is_empty() {
        return None;
    }

    let chunk = unsafe { (artifact.seed.root_chunk_addr as *const LuaProto).as_ref() }?;
    let merge_inst = Instruction::from_u32(merge_jump.raw_instruction);
    let merge_target_pc = ((merge_jump.pc + 1) as i64 + merge_inst.get_sj() as i64) as u32;
    if lowered_exit.resume_pc >= merge_target_pc {
        return None;
    }

    let else_steps = compile_numeric_steps_from_chunk(chunk, lowered_exit.resume_pc, merge_target_pc)?;
    if else_steps.is_empty() {
        return None;
    }

    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);
    let (cond, then_on_true, then_preset, else_preset) =
        compile_numeric_ifelse_condition(&ir.insts[cond_index])?;
    Some(CompiledTraceExecutor::NumericIfElseForLoop {
        loop_reg: loop_inst.get_a(),
        pre_steps,
        cond,
        then_preset,
        else_preset,
        then_steps,
        else_steps,
        then_on_true,
    })
}

fn compile_linear_int_jmp_loop(
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<CompiledTraceExecutor> {
    if ir.insts.len() < 3 || ir.guards.len() != 1 {
        return None;
    }

    let backedge = ir.insts.last()?;
    if backedge.opcode != crate::OpCode::Jmp {
        return None;
    }
    let backedge_inst = Instruction::from_u32(backedge.raw_instruction);
    if backedge_inst.get_sj() >= 0 {
        return None;
    }

    let guard = ir.guards[0];
    let lowered_exit = lowered_exit_for_guard(lowered_trace, 0, guard)?;

    if !guard.taken_on_trace {
        if ir.insts.len() < 4 || ir.insts[1].opcode != crate::OpCode::Jmp {
            return None;
        }
        let head = &ir.insts[0];
        let branch = &ir.insts[1];
        let loop_guard = compile_linear_int_guard(head, false, lowered_exit.exit_pc)?;
        let branch_inst = Instruction::from_u32(branch.raw_instruction);
        if branch_inst.get_sj() <= 0 {
            return None;
        }
        let steps = compile_linear_int_steps(&ir.insts[2..ir.insts.len() - 1])?;
        return Some(CompiledTraceExecutor::LinearIntJmpLoop {
            steps,
            guard: loop_guard,
        });
    }

    let head_len = ir.insts.len() - 2;
    let tail_guard_inst = &ir.insts[ir.insts.len() - 2];
    let steps = compile_linear_int_steps(&ir.insts[..head_len])?;
    let loop_guard = compile_linear_int_guard(tail_guard_inst, true, lowered_exit.exit_pc)?;
    Some(CompiledTraceExecutor::LinearIntJmpLoop {
        steps,
        guard: loop_guard,
    })
}


fn compile_numeric_jmp_loop(
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<CompiledTraceExecutor> {
    if ir.insts.len() < 3 || ir.guards.is_empty() {
        return None;
    }

    let backedge = ir.insts.last()?;
    if backedge.opcode != crate::OpCode::Jmp {
        return None;
    }
    let backedge_inst = Instruction::from_u32(backedge.raw_instruction);
    if backedge_inst.get_sj() >= 0 {
        return None;
    }

    if ir.guards.iter().all(|guard| !guard.taken_on_trace) {
        let mut head_blocks = Vec::with_capacity(ir.guards.len());
        let mut guard_index = 0usize;
        let mut segment_start = 0usize;

        while segment_start < ir.insts.len() - 1 {
            let relative_guard_index = ir.insts[segment_start..ir.insts.len() - 1]
                .iter()
                .position(|inst| {
                    matches!(
                        inst.opcode,
                        crate::OpCode::Test
                            | crate::OpCode::TestSet
                            | crate::OpCode::Lt
                            | crate::OpCode::Le
                    )
                });
            let Some(relative_guard_index) = relative_guard_index else {
                break;
            };
            let inst_guard_index = segment_start + relative_guard_index;
            if inst_guard_index + 1 >= ir.insts.len() - 1
                || ir.insts[inst_guard_index + 1].opcode != crate::OpCode::Jmp
            {
                return None;
            }

            let guard = *ir.guards.get(guard_index)?;
            let lowered_exit = lowered_exit_for_guard(lowered_trace, guard_index, guard)?;
            let pre_steps = compile_numeric_steps(&ir.insts[segment_start..inst_guard_index])?;
            let loop_guard =
                compile_numeric_jmp_guard(&ir.insts[inst_guard_index], false, lowered_exit.exit_pc)?;
            head_blocks.push(NumericJmpLoopGuardBlock { pre_steps, guard: loop_guard });
            guard_index += 1;
            segment_start = inst_guard_index + 2;
        }

        if guard_index != ir.guards.len() {
            return None;
        }

        let steps = compile_numeric_steps(&ir.insts[segment_start..ir.insts.len() - 1])?;
        return Some(CompiledTraceExecutor::NumericJmpLoop {
            head_blocks,
            steps,
            tail_blocks: Vec::new(),
        });
    }

    if ir.guards.len() != 1 || !ir.guards[0].taken_on_trace {
        return None;
    }

    let guard = ir.guards[0];
    let lowered_exit = lowered_exit_for_guard(lowered_trace, 0, guard)?;
    let loop_guard = compile_numeric_jmp_guard(&ir.insts[ir.insts.len() - 2], true, lowered_exit.exit_pc)?;
    let steps = compile_numeric_steps(&ir.insts[..ir.insts.len() - 2])?;
    Some(CompiledTraceExecutor::NumericJmpLoop {
        head_blocks: Vec::new(),
        steps,
        tail_blocks: vec![NumericJmpLoopGuardBlock {
            pre_steps: Vec::new(),
            guard: loop_guard,
        }],
    })
}

fn compile_numeric_table_scan_jmp_loop(
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<CompiledTraceExecutor> {
    if ir.insts.len() != 6 || ir.guards.len() != 1 {
        return None;
    }

    let guard = ir.guards[0];
    if guard.taken_on_trace {
        return None;
    }
    let lowered_exit = lowered_exit_for_guard(lowered_trace, 0, guard)?;

    let get_table = &ir.insts[0];
    let compare = &ir.insts[1];
    let exit_jump = &ir.insts[2];
    let addi = &ir.insts[3];
    let mm_bini = &ir.insts[4];
    let backedge = &ir.insts[5];

    if get_table.opcode != crate::OpCode::GetTable
        || exit_jump.opcode != crate::OpCode::Jmp
        || addi.opcode != crate::OpCode::AddI
        || mm_bini.opcode != crate::OpCode::MmBinI
        || backedge.opcode != crate::OpCode::Jmp
    {
        return None;
    }

    let exit_jump_inst = Instruction::from_u32(exit_jump.raw_instruction);
    let backedge_inst = Instruction::from_u32(backedge.raw_instruction);
    if exit_jump_inst.get_sj() <= 0 || backedge_inst.get_sj() >= 0 {
        return None;
    }

    let get_table_inst = Instruction::from_u32(get_table.raw_instruction);
    if get_table_inst.get_k() {
        return None;
    }
    let compare_inst = Instruction::from_u32(compare.raw_instruction);
    let addi_inst = Instruction::from_u32(addi.raw_instruction);
    let mm_bini_inst = Instruction::from_u32(mm_bini.raw_instruction);

    let loaded_reg = get_table_inst.get_a();
    let table_reg = get_table_inst.get_b();
    let index_reg = get_table_inst.get_c();

    let (lhs, rhs) = (compare_inst.get_a(), compare_inst.get_b());
    let (limit_reg, compare_op) = if lhs == loaded_reg {
        let op = match compare.opcode {
            crate::OpCode::Lt => LinearIntGuardOp::Lt,
            crate::OpCode::Le => LinearIntGuardOp::Le,
            _ => return None,
        };
        (rhs, op)
    } else if rhs == loaded_reg {
        let op = match compare.opcode {
            crate::OpCode::Lt => LinearIntGuardOp::Gt,
            crate::OpCode::Le => LinearIntGuardOp::Ge,
            _ => return None,
        };
        (lhs, op)
    } else {
        return None;
    };

    if addi_inst.get_a() != index_reg
        || addi_inst.get_b() != index_reg
        || mm_bini_inst.get_a() != index_reg
        || mm_bini_inst.get_sb() != addi_inst.get_sc().unsigned_abs() as i32
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericTableScanJmpLoop {
        table_reg,
        index_reg,
        limit_reg,
        step_imm: addi_inst.get_sc(),
        compare_op,
        exit_pc: lowered_exit.exit_pc,
    })
}


