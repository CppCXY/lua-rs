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
