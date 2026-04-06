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

fn compile_numeric_for_table_mul_add_mod(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 8 {
        return None;
    }

    let get_table = &ir.insts[0];
    let mul = &ir.insts[1];
    let mul_mm = &ir.insts[2];
    let add = &ir.insts[3];
    let add_mm = &ir.insts[4];
    let modk = &ir.insts[5];
    let modk_mm = &ir.insts[6];
    let loop_backedge = &ir.insts[7];

    if get_table.opcode != crate::OpCode::GetTable
        || mul.opcode != crate::OpCode::Mul
        || mul_mm.opcode != crate::OpCode::MmBin
        || add.opcode != crate::OpCode::Add
        || add_mm.opcode != crate::OpCode::MmBin
        || modk.opcode != crate::OpCode::ModK
        || modk_mm.opcode != crate::OpCode::MmBinK
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let get_table_inst = Instruction::from_u32(get_table.raw_instruction);
    let mul_inst = Instruction::from_u32(mul.raw_instruction);
    let mul_mm_inst = Instruction::from_u32(mul_mm.raw_instruction);
    let add_inst = Instruction::from_u32(add.raw_instruction);
    let add_mm_inst = Instruction::from_u32(add_mm.raw_instruction);
    let modk_inst = Instruction::from_u32(modk.raw_instruction);
    let modk_mm_inst = Instruction::from_u32(modk_mm.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    if get_table_inst.get_k() || mul_inst.get_k() || add_inst.get_k() {
        return None;
    }

    let value_reg = get_table_inst.get_a();
    let table_reg = get_table_inst.get_b();
    let index_reg = get_table_inst.get_c();
    let acc_reg = modk_inst.get_a();

    if mul_inst.get_a() != value_reg
        || mul_inst.get_b() != value_reg
        || mul_inst.get_c() != index_reg
        || mul_mm_inst.get_a() != value_reg
        || mul_mm_inst.get_b() != index_reg
        || add_inst.get_a() != value_reg
        || add_inst.get_b() != acc_reg
        || add_inst.get_c() != value_reg
        || add_mm_inst.get_a() != acc_reg
        || add_mm_inst.get_b() != value_reg
        || modk_inst.get_b() != value_reg
        || modk_mm_inst.get_a() != value_reg
        || modk_mm_inst.get_b() != modk_inst.get_c()
        || loop_inst.get_a() + 2 != index_reg
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForTableMulAddMod {
        loop_reg: loop_inst.get_a(),
        table_reg,
        index_reg,
        acc_reg,
        modulo_const: modk_inst.get_c(),
    })
}

fn compile_numeric_for_table_copy(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 3 {
        return None;
    }

    let get_table = &ir.insts[0];
    let set_table = &ir.insts[1];
    let loop_backedge = &ir.insts[2];

    if get_table.opcode != crate::OpCode::GetTable
        || set_table.opcode != crate::OpCode::SetTable
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let get_table_inst = Instruction::from_u32(get_table.raw_instruction);
    let set_table_inst = Instruction::from_u32(set_table.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    if get_table_inst.get_k() || set_table_inst.get_k() {
        return None;
    }

    let value_reg = get_table_inst.get_a();
    let src_table_reg = get_table_inst.get_b();
    let index_reg = get_table_inst.get_c();
    let dst_table_reg = set_table_inst.get_a();

    if set_table_inst.get_b() != index_reg
        || set_table_inst.get_c() != value_reg
        || loop_inst.get_a() + 2 != index_reg
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForTableCopy {
        loop_reg: loop_inst.get_a(),
        src_table_reg,
        dst_table_reg,
        index_reg,
    })
}

fn compile_numeric_for_table_is_sorted(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if ir.insts.len() != 7 || ir.guards.len() != 1 {
        return None;
    }

    let guard = ir.guards[0];
    if !matches!(
        guard.kind,
        crate::lua_vm::jit::ir::TraceIrGuardKind::SideExit
            | crate::lua_vm::jit::ir::TraceIrGuardKind::LoopBackedgeGuard
    ) {
        return None;
    }

    let prev_index = &ir.insts[0];
    let prev_index_mm = &ir.insts[1];
    let prev_value = &ir.insts[2];
    let current_value = &ir.insts[3];
    let compare = &ir.insts[4];
    let compare_jmp = &ir.insts[5];
    let loop_backedge = &ir.insts[6];

    if prev_index.opcode != crate::OpCode::AddI
        || prev_index_mm.opcode != crate::OpCode::MmBinI
        || prev_value.opcode != crate::OpCode::GetTable
        || current_value.opcode != crate::OpCode::GetTable
        || compare.opcode != crate::OpCode::Lt
        || compare_jmp.opcode != crate::OpCode::Jmp
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let prev_index_inst = Instruction::from_u32(prev_index.raw_instruction);
    let prev_value_inst = Instruction::from_u32(prev_value.raw_instruction);
    let current_value_inst = Instruction::from_u32(current_value.raw_instruction);
    let compare_inst = Instruction::from_u32(compare.raw_instruction);
    let compare_jmp_inst = Instruction::from_u32(compare_jmp.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    if prev_index_inst.get_sc() != -1
        || prev_value_inst.get_k()
        || current_value_inst.get_k()
        || compare_inst.get_k()
        || compare_jmp_inst.get_sj() <= 0
    {
        return None;
    }

    let table_reg = prev_value_inst.get_b();
    let index_reg = current_value_inst.get_c();

    if prev_index_inst.get_b() != index_reg
        || prev_value_inst.get_c() != prev_index_inst.get_a()
        || current_value_inst.get_b() != table_reg
        || compare_inst.get_a() != current_value_inst.get_a()
        || compare_inst.get_b() != prev_value_inst.get_a()
        || loop_inst.get_a() + 2 != index_reg
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForTableIsSorted {
        loop_reg: loop_inst.get_a(),
        table_reg,
        index_reg,
        false_exit_pc: compare_jmp.pc + 1,
    })
}

fn compile_numeric_for_sort_check_sum_loop(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if ir.guards.len() != 1 || ir.insts.len() != 24 {
        return None;
    }

    let copy_func = &ir.insts[0];
    let copy_arg = &ir.insts[1];
    let copy_call = &ir.insts[2];
    let sort_func = &ir.insts[3];
    let sort_arg = &ir.insts[4];
    let sort_left = &ir.insts[5];
    let sort_right = &ir.insts[6];
    let sort_call = &ir.insts[7];
    let check_func = &ir.insts[8];
    let check_arg = &ir.insts[9];
    let check_call = &ir.insts[10];
    let check_test = &ir.insts[11];
    let check_jmp = &ir.insts[12];
    let error_func = &ir.insts[13];
    let error_msg = &ir.insts[14];
    let error_call = &ir.insts[15];
    let checksum_func = &ir.insts[16];
    let checksum_arg = &ir.insts[17];
    let checksum_call = &ir.insts[18];
    let sum_add = &ir.insts[19];
    let sum_add_mm = &ir.insts[20];
    let sum_mod = &ir.insts[21];
    let sum_mod_mm = &ir.insts[22];
    let loop_backedge = &ir.insts[23];

    if copy_func.opcode != crate::OpCode::Move
        || copy_arg.opcode != crate::OpCode::Move
        || copy_call.opcode != crate::OpCode::Call
        || sort_func.opcode != crate::OpCode::Move
        || sort_arg.opcode != crate::OpCode::Move
        || sort_left.opcode != crate::OpCode::LoadI
        || sort_right.opcode != crate::OpCode::Len
        || sort_call.opcode != crate::OpCode::Call
        || check_func.opcode != crate::OpCode::Move
        || check_arg.opcode != crate::OpCode::Move
        || check_call.opcode != crate::OpCode::Call
        || check_test.opcode != crate::OpCode::Test
        || check_jmp.opcode != crate::OpCode::Jmp
        || error_func.opcode != crate::OpCode::GetTabUp
        || error_msg.opcode != crate::OpCode::LoadK
        || error_call.opcode != crate::OpCode::Call
        || checksum_func.opcode != crate::OpCode::Move
        || checksum_arg.opcode != crate::OpCode::Move
        || checksum_call.opcode != crate::OpCode::Call
        || sum_add.opcode != crate::OpCode::Add
        || sum_add_mm.opcode != crate::OpCode::MmBin
        || sum_mod.opcode != crate::OpCode::ModK
        || sum_mod_mm.opcode != crate::OpCode::MmBinK
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let copy_func_inst = Instruction::from_u32(copy_func.raw_instruction);
    let copy_arg_inst = Instruction::from_u32(copy_arg.raw_instruction);
    let copy_call_inst = Instruction::from_u32(copy_call.raw_instruction);
    let sort_func_inst = Instruction::from_u32(sort_func.raw_instruction);
    let sort_arg_inst = Instruction::from_u32(sort_arg.raw_instruction);
    let sort_left_inst = Instruction::from_u32(sort_left.raw_instruction);
    let sort_right_inst = Instruction::from_u32(sort_right.raw_instruction);
    let sort_call_inst = Instruction::from_u32(sort_call.raw_instruction);
    let check_func_inst = Instruction::from_u32(check_func.raw_instruction);
    let check_arg_inst = Instruction::from_u32(check_arg.raw_instruction);
    let check_call_inst = Instruction::from_u32(check_call.raw_instruction);
    let check_test_inst = Instruction::from_u32(check_test.raw_instruction);
    let error_func_inst = Instruction::from_u32(error_func.raw_instruction);
    let error_msg_inst = Instruction::from_u32(error_msg.raw_instruction);
    let error_call_inst = Instruction::from_u32(error_call.raw_instruction);
    let checksum_func_inst = Instruction::from_u32(checksum_func.raw_instruction);
    let checksum_arg_inst = Instruction::from_u32(checksum_arg.raw_instruction);
    let checksum_call_inst = Instruction::from_u32(checksum_call.raw_instruction);
    let sum_add_inst = Instruction::from_u32(sum_add.raw_instruction);
    let sum_add_mm_inst = Instruction::from_u32(sum_add_mm.raw_instruction);
    let sum_mod_inst = Instruction::from_u32(sum_mod.raw_instruction);
    let sum_mod_mm_inst = Instruction::from_u32(sum_mod_mm.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    let work_reg = copy_call_inst.get_a();
    let call_base = sort_call_inst.get_a();
    let loop_reg = loop_inst.get_a();

    if copy_func_inst.get_a() != work_reg
        || copy_arg_inst.get_a() != work_reg + 1
        || copy_call_inst.get_b() != 2
        || copy_call_inst.get_c() != 2
        || sort_func_inst.get_a() != call_base
        || sort_arg_inst.get_a() != call_base + 1
        || sort_left_inst.get_a() != call_base + 2
        || sort_left_inst.get_sbx() != 1
        || sort_right_inst.get_a() != call_base + 3
        || sort_right_inst.get_b() != work_reg
        || sort_call_inst.get_b() != 4
        || sort_call_inst.get_c() != 1
        || check_func_inst.get_a() != call_base
        || check_arg_inst.get_a() != call_base + 1
        || check_call_inst.get_a() != call_base
        || check_call_inst.get_b() != 2
        || check_call_inst.get_c() != 2
        || check_test_inst.get_a() != call_base
        || error_func_inst.get_a() != call_base
        || error_msg_inst.get_a() != call_base + 1
        || error_call_inst.get_a() != call_base
        || error_call_inst.get_b() != 2
        || error_call_inst.get_c() != 1
        || checksum_func_inst.get_a() != call_base
        || checksum_arg_inst.get_a() != call_base + 1
        || checksum_call_inst.get_a() != call_base
        || checksum_call_inst.get_b() != 2
        || checksum_call_inst.get_c() != 2
        || sum_add_inst.get_a() != call_base
        || sum_add_inst.get_b() != sum_mod_inst.get_a()
        || sum_add_inst.get_c() != call_base
        || sum_add_inst.get_k()
        || sum_add_mm_inst.get_a() != sum_mod_inst.get_a()
        || sum_add_mm_inst.get_b() != call_base
        || sum_mod_inst.get_b() != call_base
        || sum_mod_mm_inst.get_a() != call_base
        || sum_mod_mm_inst.get_b() != sum_mod_inst.get_c()
        || loop_reg + 2 != loop_reg + 2
    {
        return None;
    }

    if copy_func_inst.get_b() == sort_func_inst.get_b()
        || copy_func_inst.get_b() == check_func_inst.get_b()
        || copy_func_inst.get_b() == checksum_func_inst.get_b()
    {
        return None;
    }

    if sort_arg_inst.get_b() != work_reg
        || check_arg_inst.get_b() != work_reg
        || checksum_arg_inst.get_b() != work_reg
    {
        return None;
    }

    if sum_mod_inst.get_a() != sum_add_inst.get_b() {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForArraySortValidateChecksumLoop {
        loop_reg,
        source_reg: copy_arg_inst.get_b(),
        work_reg,
        sum_reg: sum_mod_inst.get_a(),
        copy_func_reg: copy_func_inst.get_b(),
        sort_func_reg: sort_func_inst.get_b(),
        check_func_reg: check_func_inst.get_b(),
        checksum_func_reg: checksum_func_inst.get_b(),
        modulo_const: sum_mod_inst.get_c(),
    })
}


fn compile_numeric_for_upvalue_addi(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 5 {
        return None;
    }

    let get_upval = &ir.insts[0];
    let add = &ir.insts[1];
    let mm_bin = &ir.insts[2];
    let set_upval = &ir.insts[3];
    let loop_backedge = &ir.insts[4];

    if get_upval.opcode != crate::OpCode::GetUpval
        || add.opcode != crate::OpCode::AddI
        || mm_bin.opcode != crate::OpCode::MmBinI
        || set_upval.opcode != crate::OpCode::SetUpval
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let get_inst = Instruction::from_u32(get_upval.raw_instruction);
    let add_inst = Instruction::from_u32(add.raw_instruction);
    let mm_inst = Instruction::from_u32(mm_bin.raw_instruction);
    let set_inst = Instruction::from_u32(set_upval.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    let value_reg = get_inst.get_a();
    if add_inst.get_a() != value_reg
        || add_inst.get_b() != value_reg
        || mm_inst.get_a() != value_reg
        || mm_inst.get_sb() != add_inst.get_sc().unsigned_abs() as i32
        || set_inst.get_a() != value_reg
        || set_inst.get_b() != get_inst.get_b()
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForUpvalueAddI {
        loop_reg: loop_inst.get_a(),
        upvalue: get_inst.get_b(),
        value_reg,
        imm: add_inst.get_sc(),
    })
}

fn compile_numeric_for_field_addi(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 5 {
        return None;
    }

    let get_field = &ir.insts[0];
    let add = &ir.insts[1];
    let mm_bin = &ir.insts[2];
    let set_field = &ir.insts[3];
    let loop_backedge = &ir.insts[4];

    if get_field.opcode != crate::OpCode::GetField
        || add.opcode != crate::OpCode::AddI
        || mm_bin.opcode != crate::OpCode::MmBinI
        || set_field.opcode != crate::OpCode::SetField
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let get_inst = Instruction::from_u32(get_field.raw_instruction);
    let add_inst = Instruction::from_u32(add.raw_instruction);
    let mm_inst = Instruction::from_u32(mm_bin.raw_instruction);
    let set_inst = Instruction::from_u32(set_field.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    let value_reg = get_inst.get_a();
    let table_reg = get_inst.get_b();
    let key_const = get_inst.get_c();
    if add_inst.get_a() != value_reg
        || add_inst.get_b() != value_reg
        || mm_inst.get_a() != value_reg
        || mm_inst.get_sb() != add_inst.get_sc().unsigned_abs() as i32
        || set_inst.get_a() != table_reg
        || set_inst.get_b() != key_const
        || set_inst.get_k()
        || set_inst.get_c() != value_reg
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForFieldAddI {
        loop_reg: loop_inst.get_a(),
        table_reg,
        value_reg,
        key_const,
        imm: add_inst.get_sc(),
    })
}

fn compile_numeric_for_tabup_addi(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 5 {
        return None;
    }

    let get_tabup = &ir.insts[0];
    let add = &ir.insts[1];
    let mm_bin = &ir.insts[2];
    let set_tabup = &ir.insts[3];
    let loop_backedge = &ir.insts[4];

    if get_tabup.opcode != crate::OpCode::GetTabUp
        || add.opcode != crate::OpCode::AddI
        || mm_bin.opcode != crate::OpCode::MmBinI
        || set_tabup.opcode != crate::OpCode::SetTabUp
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let get_inst = Instruction::from_u32(get_tabup.raw_instruction);
    let add_inst = Instruction::from_u32(add.raw_instruction);
    let mm_inst = Instruction::from_u32(mm_bin.raw_instruction);
    let set_inst = Instruction::from_u32(set_tabup.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    let value_reg = get_inst.get_a();
    let env_upvalue = get_inst.get_b();
    let key_const = get_inst.get_c();
    if add_inst.get_a() != value_reg
        || add_inst.get_b() != value_reg
        || mm_inst.get_a() != value_reg
        || mm_inst.get_sb() != add_inst.get_sc().unsigned_abs() as i32
        || set_inst.get_a() != env_upvalue
        || set_inst.get_b() != key_const
        || set_inst.get_k()
        || set_inst.get_c() != value_reg
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForTabUpAddI {
        loop_reg: loop_inst.get_a(),
        env_upvalue,
        value_reg,
        key_const,
        imm: add_inst.get_sc(),
    })
}

fn compile_numeric_for_tabup_field_addi(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 7 {
        return None;
    }

    let get_table = &ir.insts[0];
    let get_value_table = &ir.insts[1];
    let get_field = &ir.insts[2];
    let add = &ir.insts[3];
    let mm_bin = &ir.insts[4];
    let set_field = &ir.insts[5];
    let loop_backedge = &ir.insts[6];

    if get_table.opcode != crate::OpCode::GetTabUp
        || get_value_table.opcode != crate::OpCode::GetTabUp
        || get_field.opcode != crate::OpCode::GetField
        || add.opcode != crate::OpCode::AddI
        || mm_bin.opcode != crate::OpCode::MmBinI
        || set_field.opcode != crate::OpCode::SetField
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let get_table_inst = Instruction::from_u32(get_table.raw_instruction);
    let get_value_table_inst = Instruction::from_u32(get_value_table.raw_instruction);
    let get_field_inst = Instruction::from_u32(get_field.raw_instruction);
    let add_inst = Instruction::from_u32(add.raw_instruction);
    let mm_inst = Instruction::from_u32(mm_bin.raw_instruction);
    let set_inst = Instruction::from_u32(set_field.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    let env_upvalue = get_table_inst.get_b();
    let table_key_const = get_table_inst.get_c();
    let table_reg = get_table_inst.get_a();
    let value_reg = get_field_inst.get_a();
    let field_key_const = get_field_inst.get_c();
    if get_value_table_inst.get_b() != env_upvalue
        || get_value_table_inst.get_c() != table_key_const
        || get_field_inst.get_b() != get_value_table_inst.get_a()
        || add_inst.get_a() != value_reg
        || add_inst.get_b() != value_reg
        || mm_inst.get_a() != value_reg
        || mm_inst.get_sb() != add_inst.get_sc().unsigned_abs() as i32
        || set_inst.get_a() != table_reg
        || set_inst.get_b() != field_key_const
        || set_inst.get_k()
        || set_inst.get_c() != value_reg
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForTabUpFieldAddI {
        loop_reg: loop_inst.get_a(),
        env_upvalue,
        table_reg,
        value_reg,
        table_key_const,
        field_key_const,
        imm: add_inst.get_sc(),
    })
}

fn compile_numeric_for_tabup_field_load(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 3 {
        return None;
    }

    let get_table = &ir.insts[0];
    let get_field = &ir.insts[1];
    let loop_backedge = &ir.insts[2];

    if get_table.opcode != crate::OpCode::GetTabUp
        || get_field.opcode != crate::OpCode::GetField
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let get_table_inst = Instruction::from_u32(get_table.raw_instruction);
    let get_field_inst = Instruction::from_u32(get_field.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    let value_reg = get_field_inst.get_a();
    if get_field_inst.get_b() != value_reg || get_table_inst.get_a() != value_reg {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForTabUpFieldLoad {
        loop_reg: loop_inst.get_a(),
        env_upvalue: get_table_inst.get_b(),
        value_reg,
        table_key_const: get_table_inst.get_c(),
        field_key_const: get_field_inst.get_c(),
    })
}

fn compile_numeric_for_builtin_unary_const_call(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 4 {
        return None;
    }

    let move_inst = &ir.insts[0];
    let loadk_inst = &ir.insts[1];
    let call_inst = &ir.insts[2];
    let loop_backedge = &ir.insts[3];

    if move_inst.opcode != crate::OpCode::Move
        || loadk_inst.opcode != crate::OpCode::LoadK
        || call_inst.opcode != crate::OpCode::Call
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let move_raw = Instruction::from_u32(move_inst.raw_instruction);
    let loadk_raw = Instruction::from_u32(loadk_inst.raw_instruction);
    let call_raw = Instruction::from_u32(call_inst.raw_instruction);
    let loop_raw = Instruction::from_u32(loop_backedge.raw_instruction);

    let result_reg = call_raw.get_a();
    if move_raw.get_a() != result_reg
        || loadk_raw.get_a() != result_reg + 1
        || call_raw.get_b() != 2
        || call_raw.get_c() != 2
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForBuiltinUnaryConstCall {
        loop_reg: loop_raw.get_a(),
        func_reg: move_raw.get_b(),
        result_reg,
        arg_const: loadk_raw.get_bx(),
    })
}

fn compile_numeric_for_tabup_field_string_unary_call(
    ir: &TraceIr,
) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 5 {
        return None;
    }

    let get_table = &ir.insts[0];
    let get_field = &ir.insts[1];
    let move_arg = &ir.insts[2];
    let call_inst = &ir.insts[3];
    let loop_backedge = &ir.insts[4];

    if get_table.opcode != crate::OpCode::GetTabUp
        || get_field.opcode != crate::OpCode::GetField
        || move_arg.opcode != crate::OpCode::Move
        || call_inst.opcode != crate::OpCode::Call
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let get_table_raw = Instruction::from_u32(get_table.raw_instruction);
    let get_field_raw = Instruction::from_u32(get_field.raw_instruction);
    let move_arg_raw = Instruction::from_u32(move_arg.raw_instruction);
    let call_raw = Instruction::from_u32(call_inst.raw_instruction);
    let loop_raw = Instruction::from_u32(loop_backedge.raw_instruction);

    let result_reg = call_raw.get_a();
    if get_table_raw.get_a() != result_reg
        || get_field_raw.get_a() != result_reg
        || get_field_raw.get_b() != result_reg
        || move_arg_raw.get_a() != result_reg + 1
        || call_raw.get_b() != 2
        || call_raw.get_c() != 2
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForTabUpFieldStringUnaryCall {
        loop_reg: loop_raw.get_a(),
        env_upvalue: get_table_raw.get_b(),
        result_reg,
        arg_reg: move_arg_raw.get_b(),
        table_key_const: get_table_raw.get_c(),
        field_key_const: get_field_raw.get_c(),
    })
}

fn compile_numeric_for_lua_closure_addi(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 6 {
        return None;
    }

    let move_func = &ir.insts[0];
    let move_arg = &ir.insts[1];
    let loadi = &ir.insts[2];
    let call = &ir.insts[3];
    let move_dst = &ir.insts[4];
    let loop_backedge = &ir.insts[5];

    if move_func.opcode != crate::OpCode::Move
        || move_arg.opcode != crate::OpCode::Move
        || loadi.opcode != crate::OpCode::LoadI
        || call.opcode != crate::OpCode::Call
        || move_dst.opcode != crate::OpCode::Move
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let move_func_inst = Instruction::from_u32(move_func.raw_instruction);
    let move_arg_inst = Instruction::from_u32(move_arg.raw_instruction);
    let loadi_inst = Instruction::from_u32(loadi.raw_instruction);
    let call_inst = Instruction::from_u32(call.raw_instruction);
    let move_dst_inst = Instruction::from_u32(move_dst.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    let call_base = call_inst.get_a();
    if move_func_inst.get_a() != call_base
        || move_arg_inst.get_a() != call_base + 1
        || loadi_inst.get_a() != call_base + 2
        || call_inst.get_b() != 3
        || call_inst.get_c() != 2
        || move_dst_inst.get_b() != call_base
    {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForLuaClosureAddI {
        loop_reg: loop_inst.get_a(),
        func_reg: move_func_inst.get_b(),
        arg_reg: move_arg_inst.get_b(),
        dst_reg: move_dst_inst.get_a(),
        imm: loadi_inst.get_sbx(),
    })
}


fn compile_numeric_for_gettable_add(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() || ir.insts.len() != 4 {
        return None;
    }

    let get_table = &ir.insts[0];
    let add = &ir.insts[1];
    let mm_bin = &ir.insts[2];
    let loop_backedge = &ir.insts[3];

    if get_table.opcode != crate::OpCode::GetTable
        || add.opcode != crate::OpCode::Add
        || mm_bin.opcode != crate::OpCode::MmBin
        || loop_backedge.opcode != crate::OpCode::ForLoop
    {
        return None;
    }

    let get_inst = Instruction::from_u32(get_table.raw_instruction);
    let add_inst = Instruction::from_u32(add.raw_instruction);
    let loop_inst = Instruction::from_u32(loop_backedge.raw_instruction);

    let value_reg = get_inst.get_a();
    let table_reg = get_inst.get_b();
    let index_reg = get_inst.get_c();
    let acc_reg = add_inst.get_a();

    if add_inst.get_b() != acc_reg || add_inst.get_c() != value_reg || add_inst.get_k() {
        return None;
    }

    let loop_reg = loop_inst.get_a();
    if index_reg != loop_reg + 2 {
        return None;
    }

    Some(CompiledTraceExecutor::NumericForGetTableAdd {
        loop_reg,
        table_reg,
        index_reg,
        value_reg,
        acc_reg,
    })
}


