fn compile_generic_for_builtin_add(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
	if ir.insts.len() != 4 || ir.guards.len() != 1 {
		return None;
	}

	let add = &ir.insts[0];
	let mm_bin = &ir.insts[1];
	let tfor_call = &ir.insts[2];
	let tfor_loop = &ir.insts[3];
	let guard = ir.guards[0];

	if add.opcode != crate::OpCode::Add
		|| mm_bin.opcode != crate::OpCode::MmBin
		|| tfor_call.opcode != crate::OpCode::TForCall
		|| tfor_loop.opcode != crate::OpCode::TForLoop
		|| guard.kind != TraceIrGuardKind::LoopBackedgeGuard
		|| guard.guard_pc != tfor_loop.pc
		|| !guard.taken_on_trace
	{
		return None;
	}

	let add_inst = Instruction::from_u32(add.raw_instruction);
	let mm_bin_inst = Instruction::from_u32(mm_bin.raw_instruction);
	let tfor_call_inst = Instruction::from_u32(tfor_call.raw_instruction);
	let tfor_loop_inst = Instruction::from_u32(tfor_loop.raw_instruction);

	let acc_reg = add_inst.get_a();
	if add_inst.get_b() != acc_reg || add_inst.get_k() {
		return None;
	}

	let tfor_reg = tfor_call_inst.get_a();
	if tfor_call_inst.get_c() != 2 || tfor_loop_inst.get_a() != tfor_reg {
		return None;
	}

	let value_reg = tfor_reg + 4;
	if add_inst.get_c() != value_reg
		|| mm_bin_inst.get_a() != acc_reg
		|| mm_bin_inst.get_b() != value_reg
	{
		return None;
	}

	Some(CompiledTraceExecutor::GenericForBuiltinAdd {
		tfor_reg,
		value_reg,
		acc_reg,
	})
}

fn compile_next_while_builtin_add(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
	if ir.insts.len() != 11 || ir.guards.len() != 1 {
		return None;
	}

	let test = &ir.insts[0];
	let exit_jmp = &ir.insts[1];
	let add = &ir.insts[2];
	let mm_bin = &ir.insts[3];
	let get_tabup = &ir.insts[4];
	let move_state = &ir.insts[5];
	let move_key_arg = &ir.insts[6];
	let call = &ir.insts[7];
	let move_value = &ir.insts[8];
	let move_key = &ir.insts[9];
	let backedge_jmp = &ir.insts[10];
	let guard = ir.guards[0];

	if test.opcode != crate::OpCode::Test
		|| exit_jmp.opcode != crate::OpCode::Jmp
		|| add.opcode != crate::OpCode::Add
		|| mm_bin.opcode != crate::OpCode::MmBin
		|| get_tabup.opcode != crate::OpCode::GetTabUp
		|| move_state.opcode != crate::OpCode::Move
		|| move_key_arg.opcode != crate::OpCode::Move
		|| call.opcode != crate::OpCode::Call
		|| move_value.opcode != crate::OpCode::Move
		|| move_key.opcode != crate::OpCode::Move
		|| backedge_jmp.opcode != crate::OpCode::Jmp
		|| guard.kind != TraceIrGuardKind::SideExit
		|| guard.guard_pc != test.pc
		|| guard.taken_on_trace
	{
		return None;
	}

	let test_inst = Instruction::from_u32(test.raw_instruction);
	let add_inst = Instruction::from_u32(add.raw_instruction);
	let mm_bin_inst = Instruction::from_u32(mm_bin.raw_instruction);
	let get_tabup_inst = Instruction::from_u32(get_tabup.raw_instruction);
	let move_state_inst = Instruction::from_u32(move_state.raw_instruction);
	let move_key_arg_inst = Instruction::from_u32(move_key_arg.raw_instruction);
	let call_inst = Instruction::from_u32(call.raw_instruction);
	let move_value_inst = Instruction::from_u32(move_value.raw_instruction);
	let move_key_inst = Instruction::from_u32(move_key.raw_instruction);

	let key_reg = test_inst.get_a();
	let acc_reg = add_inst.get_a();
	let value_reg = add_inst.get_c();
	let call_base = get_tabup_inst.get_a();

	if add_inst.get_b() != acc_reg
		|| add_inst.get_k()
		|| mm_bin_inst.get_a() != acc_reg
		|| mm_bin_inst.get_b() != value_reg
		|| move_state_inst.get_a() != call_base + 1
		|| move_key_arg_inst.get_a() != call_base + 2
		|| move_key_arg_inst.get_b() != key_reg
		|| call_inst.get_a() != call_base
		|| call_inst.get_b() != 3
		|| call_inst.get_c() != 3
		|| move_value_inst.get_a() != value_reg
		|| move_value_inst.get_b() != call_base + 1
		|| move_key_inst.get_a() != key_reg
		|| move_key_inst.get_b() != call_base
	{
		return None;
	}

	Some(CompiledTraceExecutor::NextWhileBuiltinAdd {
		key_reg,
		value_reg,
		acc_reg,
		table_reg: move_state_inst.get_b(),
		env_upvalue: get_tabup_inst.get_b(),
		key_const: get_tabup_inst.get_c(),
	})
}

