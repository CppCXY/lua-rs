fn compile_linear_int_forloop(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() < 2 {
        return None;
    }

    let loop_backedge = ir.insts.last()?;
    if loop_backedge.opcode != crate::OpCode::ForLoop {
        return None;
    }

    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);
    let loop_reg = loop_inst.get_a();
    let steps = compile_linear_int_steps(&ir.insts[..ir.insts.len() - 1])?;

    Some(CompiledTraceExecutor::LinearIntForLoop { loop_reg, steps })
}

fn compile_guarded_numeric_forloop(
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<CompiledTraceExecutor> {
    if ir.insts.len() < 4 || ir.guards.len() != 1 {
        return None;
    }

    let loop_backedge = ir.insts.last()?;
    if loop_backedge.opcode != crate::OpCode::ForLoop {
        return None;
    }

    let guard = ir.guards[0];
    let lowered_exit = lowered_exit_for_guard(lowered_trace, 0, guard)?;
    let guard_index = ir
        .insts
        .iter()
        .position(|inst| {
            matches!(
                inst.opcode,
                crate::OpCode::Test | crate::OpCode::TestSet | crate::OpCode::Lt | crate::OpCode::Le
            )
        })?;

    if guard_index + 2 != ir.insts.len() - 1 {
        return None;
    }
    if ir.insts[guard_index + 1].opcode != crate::OpCode::Jmp {
        return None;
    }

    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);
    let steps = compile_numeric_steps(&ir.insts[..guard_index])?;
    let loop_guard = compile_numeric_jmp_guard(&ir.insts[guard_index], true, lowered_exit.exit_pc)?;

    Some(CompiledTraceExecutor::GuardedNumericForLoop {
        loop_reg: loop_inst.get_a(),
        steps,
        guard: loop_guard,
    })
}

fn compile_numeric_forloop(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() < 2 {
        return None;
    }

    let loop_backedge = ir.insts.last()?;
    if loop_backedge.opcode != crate::OpCode::ForLoop {
        return None;
    }

    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);
    let loop_reg = loop_inst.get_a();
    let steps = compile_numeric_steps(&ir.insts[..ir.insts.len() - 1])?;

    Some(CompiledTraceExecutor::NumericForLoop { loop_reg, steps })
}
