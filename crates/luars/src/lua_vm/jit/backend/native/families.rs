use super::*;
use super::emit::*;
use super::profile::*;

impl NativeTraceBackend {
    pub(super) fn compile_native_generic_trace(
        &mut self,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        if let Some((execution, profile)) = self.compile_native_generic_return(ir) {
            return Some((execution, profile));
        }

        if let Some((execution, profile)) =
            self.compile_native_generic_linear_int_for(ir, lowered_trace)
        {
            return Some((execution, profile));
        }

        if let Some((execution, profile)) =
            self.compile_native_generic_numeric_for(ir, lowered_trace)
        {
            return Some((execution, profile));
        }

        if let Some((execution, profile)) =
            self.compile_native_generic_guarded_call_return(ir, lowered_trace)
        {
            return Some((execution, profile));
        }

        if let Some((execution, profile)) =
            self.compile_native_generic_guarded_call_prefix(ir, lowered_trace)
        {
            return Some((execution, profile));
        }

        if let Some((execution, profile)) = self.compile_native_generic_call_for(ir, lowered_trace)
        {
            return Some((execution, profile));
        }

        if let Some((execution, profile)) =
            self.compile_native_generic_guarded_call_jmp(ir, lowered_trace)
        {
            return Some((execution, profile));
        }

        if let Some((execution, profile)) = self.compile_native_generic_tfor(ir, lowered_trace) {
            return Some((execution, profile));
        }

        if let Some((execution, profile)) =
            self.compile_native_generic_linear_int_jmp(ir, lowered_trace)
        {
            return Some((execution, profile));
        }

        if let Some((execution, profile)) =
            self.compile_native_generic_numeric_jmp(ir, lowered_trace)
        {
            return Some((execution, profile));
        }

        None
    }

    fn compile_native_generic_return(
        &mut self,
        ir: &TraceIr,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        if !ir.guards.is_empty() {
            return None;
        }

        let [inst] = ir.insts.as_slice() else {
            return None;
        };

        let raw = Instruction::from_u32(inst.raw_instruction);
        let native = match inst.opcode {
            crate::OpCode::Return0 => self.compile_native_return0(),
            crate::OpCode::Return1 => self.compile_native_return1(raw.get_a()),
            crate::OpCode::Return if !raw.get_k() => match raw.get_b() {
                1 => self.compile_native_return0(),
                2 => self.compile_native_return1(raw.get_a()),
                b if b > 2 => self.compile_native_return(raw.get_a(), b.saturating_sub(1) as u8),
                _ => None,
            },
            _ => return None,
        }?;

        Some((
            CompiledTraceExecution::Native(native),
            Some(NativeLoweringProfile::default()),
        ))
    }

    fn compile_native_generic_linear_int_for(
        &mut self,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        let loop_backedge = ir.insts.last()?;
        if loop_backedge.opcode != crate::OpCode::ForLoop || !ir.guards.is_empty() {
            return None;
        }

        let loop_reg = Instruction::from_u32(loop_backedge.raw_instruction).get_a();
        let steps =
            lower_linear_int_steps_for_native(&ir.insts[..ir.insts.len() - 1], lowered_trace)?;
        if steps.is_empty() {
            return None;
        }
        let native = self.compile_native_linear_int_for_loop(loop_reg, &steps, lowered_trace)?;
        Some((
            CompiledTraceExecution::Native(native),
            Some(profile_for_linear_int_for_loop(&steps)),
        ))
    }

    fn compile_native_generic_numeric_for(
        &mut self,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        if recognize_quicksort_insertion_sort_outer_continue_shape(ir) {
            return None;
        }

        let loop_backedge = ir.insts.last()?;
        if loop_backedge.opcode != crate::OpCode::ForLoop {
            return None;
        }

        let loop_reg = Instruction::from_u32(loop_backedge.raw_instruction).get_a();

        if ir.guards.is_empty() {
            let lowering =
                lower_numeric_lowering_for_native(&ir.insts[..ir.insts.len() - 1], lowered_trace)?;
            if lowering.steps.is_empty() {
                return None;
            }
            let native =
                self.compile_native_numeric_for_loop(loop_reg, &lowering, lowered_trace)?;
            return Some((
                CompiledTraceExecution::Native(native),
                Some(profile_for_numeric_for_loop(&lowering.steps)),
            ));
        }

        if ir.guards.len() != 1 || ir.insts.len() < 4 {
            return None;
        }

        let guard = ir.guards[0];
        let lowered_exit = lowered_trace.deopt_target_for_exit_pc(guard.exit_pc)?;
        let guard_index = ir.insts.iter().position(|inst| {
            matches!(
                inst.opcode,
                crate::OpCode::Test
                    | crate::OpCode::TestSet
                    | crate::OpCode::Lt
                    | crate::OpCode::Le
            )
        })?;
        if guard_index + 2 != ir.insts.len() - 1 {
            return None;
        }
        if ir.insts[guard_index + 1].opcode != crate::OpCode::Jmp {
            return None;
        }

        let lowering = lower_numeric_lowering_for_native(&ir.insts[..guard_index], lowered_trace)?;
        if lowering.steps.is_empty() {
            return None;
        }
        let loop_guard =
            lower_numeric_guard_for_native(&ir.insts[guard_index], true, lowered_exit.resume_pc)?;
        let native = self.compile_native_guarded_numeric_for_loop(
            loop_reg,
            &lowering,
            loop_guard,
            lowered_trace,
        )?;
        let profile = profile_for_guarded_numeric_for_loop(&lowering.steps, loop_guard);
        Some((CompiledTraceExecution::Native(native), Some(profile)))
    }

    fn compile_native_generic_linear_int_jmp(
        &mut self,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        let backedge = ir.insts.last()?;
        if backedge.opcode != crate::OpCode::Jmp {
            return None;
        }
        if Instruction::from_u32(backedge.raw_instruction).get_sj() >= 0 {
            return None;
        }
        if ir.guards.len() != 1 {
            return None;
        }

        let guard = ir.guards[0];
        let lowered_exit = lowered_trace.deopt_target_for_exit_pc(guard.exit_pc)?;

        let (steps, loop_guard) = if guard.taken_on_trace {
            if ir.insts.len() < 3 {
                return None;
            }

            let guard_inst = &ir.insts[ir.insts.len() - 2];
            let loop_guard =
                lower_linear_int_guard_for_native(guard_inst, true, lowered_exit.resume_pc)?;
            let steps =
                lower_linear_int_steps_for_native(&ir.insts[..ir.insts.len() - 2], lowered_trace)?;
            (steps, loop_guard)
        } else {
            if ir.insts.len() < 4 || ir.insts[1].opcode != crate::OpCode::Jmp {
                return None;
            }

            let loop_guard =
                lower_linear_int_guard_for_native(&ir.insts[0], false, lowered_exit.resume_pc)?;
            let steps =
                lower_linear_int_steps_for_native(&ir.insts[2..ir.insts.len() - 1], lowered_trace)?;
            (steps, loop_guard)
        };

        let native = self.compile_native_linear_int_jmp_loop(&steps, loop_guard, lowered_trace)?;
        let profile = profile_for_linear_int_jmp_loop(&steps, loop_guard);
        Some((CompiledTraceExecution::Native(native), Some(profile)))
    }

    fn compile_native_generic_tfor(
        &mut self,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        let loop_backedge = ir.insts.last()?;
        if loop_backedge.opcode != crate::OpCode::TForLoop {
            return None;
        }
        if ir.insts.len() < 2 || ir.guards.len() != 1 {
            return None;
        }

        let call_inst = &ir.insts[ir.insts.len() - 2];
        if call_inst.opcode != crate::OpCode::TForCall {
            return None;
        }

        let call_raw = Instruction::from_u32(call_inst.raw_instruction);
        let loop_raw = Instruction::from_u32(loop_backedge.raw_instruction);
        if call_raw.get_a() != loop_raw.get_a() {
            return None;
        }

        let exit = lowered_trace.deopt_target_for_exit_pc(ir.guards[0].exit_pc)?;
        let steps = lower_numeric_steps_for_native(&ir.insts[..ir.insts.len() - 2], lowered_trace)?;
        let native = self.compile_native_tfor_loop(
            call_raw.get_a(),
            call_raw.get_c(),
            call_inst.pc,
            ir.guards[0].exit_pc,
            exit.exit_index,
            &steps,
            lowered_trace,
        )?;
        let profile = profile_for_numeric_steps(&steps);
        Some((CompiledTraceExecution::Native(native), Some(profile)))
    }

    fn compile_native_generic_call_for(
        &mut self,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        fn call_for_supports_conservative_prep_step(step: &NumericStep) -> bool {
            matches!(
                step,
                NumericStep::Move { .. }
                    | NumericStep::LoadBool { .. }
                    | NumericStep::LoadI { .. }
                    | NumericStep::LoadF { .. }
                    | NumericStep::Binary { .. }
            )
        }

        let loop_backedge = ir.insts.last()?;
        if loop_backedge.opcode != crate::OpCode::ForLoop || !ir.guards.is_empty() {
            return None;
        }
        if ir.insts.len() < 2 {
            return None;
        }

        let call_positions = ir.insts[..ir.insts.len() - 1]
            .iter()
            .enumerate()
            .filter_map(|(index, inst)| (inst.opcode == crate::OpCode::Call).then_some(index))
            .collect::<Vec<_>>();
        if call_positions.len() != 1 {
            return None;
        }
        let call_index = call_positions[0];
        let call_inst = &ir.insts[call_index];

        let call_raw = Instruction::from_u32(call_inst.raw_instruction);
        if call_raw.get_b() == 0 || call_raw.get_c() == 0 {
            return None;
        }

        let loop_reg = Instruction::from_u32(loop_backedge.raw_instruction).get_a();
        let prep_insts = &ir.insts[..call_index];
        let post_insts = &ir.insts[call_index + 1..ir.insts.len() - 1];
        let call_live_out = (call_raw.get_a()..call_raw.get_a() + call_raw.get_b())
            .collect::<Vec<_>>();
        let post_live_out = post_insts
            .iter()
            .flat_map(|inst| {
                let raw = Instruction::from_u32(inst.raw_instruction);
                match inst.opcode {
                    crate::OpCode::Move
                    | crate::OpCode::LoadFalse
                    | crate::OpCode::LFalseSkip
                    | crate::OpCode::LoadTrue
                    | crate::OpCode::LoadK
                    | crate::OpCode::LoadKX
                    | crate::OpCode::LoadI
                    | crate::OpCode::LoadF
                    | crate::OpCode::GetUpval
                    | crate::OpCode::GetTable
                    | crate::OpCode::GetField
                    | crate::OpCode::GetI
                    | crate::OpCode::GetTabUp
                    | crate::OpCode::Len
                    | crate::OpCode::Add
                    | crate::OpCode::AddK
                    | crate::OpCode::Sub
                    | crate::OpCode::SubK
                    | crate::OpCode::Mul
                    | crate::OpCode::MulK
                    | crate::OpCode::Mod
                    | crate::OpCode::ModK
                    | crate::OpCode::Pow
                    | crate::OpCode::PowK
                    | crate::OpCode::Div
                    | crate::OpCode::DivK
                    | crate::OpCode::IDiv
                    | crate::OpCode::IDivK
                    | crate::OpCode::BAnd
                    | crate::OpCode::BAndK
                    | crate::OpCode::BOr
                    | crate::OpCode::BOrK
                    | crate::OpCode::BXor
                    | crate::OpCode::BXorK
                    | crate::OpCode::Shl
                    | crate::OpCode::Shr
                    | crate::OpCode::ShlI
                    | crate::OpCode::ShrI => vec![raw.get_a()],
                    _ => Vec::new(),
                }
            })
            .collect::<Vec<_>>();
        let prep_steps = lower_numeric_steps_for_native_with_live_out(
            prep_insts,
            lowered_trace,
            &call_live_out,
        )?;
        let post_steps = lower_numeric_steps_for_native_with_live_out(
            post_insts,
            lowered_trace,
            &post_live_out,
        )?;
        if !prep_steps.iter().all(native_supports_numeric_step) {
            return None;
        }
        if !post_steps.iter().all(native_supports_numeric_step) {
            return None;
        }
        if !prep_steps
            .iter()
            .all(call_for_supports_conservative_prep_step)
        {
            return None;
        }
        if !post_steps
            .iter()
            .all(call_for_supports_conservative_prep_step)
        {
            return None;
        }

        let native = self.compile_native_call_for_loop(
            loop_reg,
            &prep_steps,
            &post_steps,
            call_raw.get_a(),
            call_raw.get_b(),
            call_raw.get_c(),
            call_inst.pc,
            lowered_trace,
        )?;
        let profile = profile_for_numeric_steps(&prep_steps);
        Some((CompiledTraceExecution::Native(native), Some(profile)))
    }

    fn compile_native_generic_guarded_call_jmp(
        &mut self,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        let loop_backedge = ir.insts.last()?;
        if loop_backedge.opcode != crate::OpCode::Jmp {
            return None;
        }
        if Instruction::from_u32(loop_backedge.raw_instruction).get_sj() >= 0 {
            return None;
        }
        if ir.guards.len() != 1 || ir.insts.len() < 6 {
            return None;
        }
        if ir.insts[1].opcode != crate::OpCode::Jmp {
            return None;
        }

        let guard_meta = ir.guards[0];
        if guard_meta.guard_pc != ir.insts[0].pc
            || guard_meta.branch_pc != ir.insts[1].pc
            || guard_meta.taken_on_trace
        {
            return None;
        }

        let call_positions = ir.insts[2..ir.insts.len() - 1]
            .iter()
            .enumerate()
            .filter_map(|(index, inst)| (inst.opcode == crate::OpCode::Call).then_some(index + 2))
            .collect::<Vec<_>>();
        if call_positions.len() != 1 {
            return None;
        }
        let call_index = call_positions[0];
        let call_inst = &ir.insts[call_index];
        let call_raw = Instruction::from_u32(call_inst.raw_instruction);
        if call_raw.get_b() == 0 || call_raw.get_c() == 0 {
            return None;
        }

        let lowered_exit = lowered_trace.deopt_target_for_exit_pc(guard_meta.exit_pc)?;
        let guard = lower_numeric_guard_for_native(&ir.insts[0], false, lowered_exit.resume_pc)?;
        if !matches!(
            guard,
            NumericJmpLoopGuard::Head {
                cond: NumericIfElseCond::Truthy { .. },
                ..
            }
        ) {
            return None;
        }

        let prep_insts = &ir.insts[2..call_index];
        let post_insts = &ir.insts[call_index + 1..ir.insts.len() - 1];
        let call_live_out = (call_raw.get_a()..call_raw.get_a() + call_raw.get_b())
            .collect::<Vec<_>>();
        let post_live_out = post_insts
            .iter()
            .flat_map(|inst| {
                let raw = Instruction::from_u32(inst.raw_instruction);
                match inst.opcode {
                    crate::OpCode::Move
                    | crate::OpCode::LoadFalse
                    | crate::OpCode::LFalseSkip
                    | crate::OpCode::LoadTrue
                    | crate::OpCode::LoadK
                    | crate::OpCode::LoadKX
                    | crate::OpCode::LoadI
                    | crate::OpCode::LoadF
                    | crate::OpCode::GetUpval
                    | crate::OpCode::GetTable
                    | crate::OpCode::GetField
                    | crate::OpCode::GetI
                    | crate::OpCode::GetTabUp
                    | crate::OpCode::Len
                    | crate::OpCode::Add
                    | crate::OpCode::AddK
                    | crate::OpCode::Sub
                    | crate::OpCode::SubK
                    | crate::OpCode::Mul
                    | crate::OpCode::MulK
                    | crate::OpCode::Mod
                    | crate::OpCode::ModK
                    | crate::OpCode::Pow
                    | crate::OpCode::PowK
                    | crate::OpCode::Div
                    | crate::OpCode::DivK
                    | crate::OpCode::IDiv
                    | crate::OpCode::IDivK
                    | crate::OpCode::BAnd
                    | crate::OpCode::BAndK
                    | crate::OpCode::BOr
                    | crate::OpCode::BOrK
                    | crate::OpCode::BXor
                    | crate::OpCode::BXorK
                    | crate::OpCode::Shl
                    | crate::OpCode::Shr
                    | crate::OpCode::ShlI
                    | crate::OpCode::ShrI => vec![raw.get_a()],
                    _ => Vec::new(),
                }
            })
            .collect::<Vec<_>>();
        let prep_steps = lower_numeric_steps_for_native_with_live_out(
            prep_insts,
            lowered_trace,
            &call_live_out,
        )?;
        let post_steps = lower_numeric_steps_for_native_with_live_out(
            post_insts,
            lowered_trace,
            &post_live_out,
        )?;
        if !prep_steps.iter().all(native_supports_numeric_step) {
            return None;
        }
        if !post_steps.iter().all(native_supports_numeric_step) {
            return None;
        }
        if !post_steps
            .iter()
            .all(|step| matches!(step, NumericStep::Move { .. }))
        {
            return None;
        }

        let native = self.compile_native_guarded_call_jmp_loop(
            &prep_steps,
            &post_steps,
            guard,
            call_raw.get_a(),
            call_raw.get_b(),
            call_raw.get_c(),
            call_inst.pc,
            lowered_trace,
        )?;
        let profile = profile_for_guarded_call_jmp_loop(&prep_steps, &post_steps, guard);
        Some((CompiledTraceExecution::Native(native), Some(profile)))
    }

    fn compile_native_generic_guarded_call_prefix(
        &mut self,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        let shape = recognize_guarded_call_prefix_shape(ir, lowered_trace)?;
        let native = self.compile_native_guarded_call_prefix_loop(shape, lowered_trace)?;
        Some((
            CompiledTraceExecution::Native(native),
            Some(NativeLoweringProfile {
                guard_steps: 1,
                truthy_guard_steps: 1,
                len_helper_steps: 1,
                arithmetic_helper_steps: 2,
                ..NativeLoweringProfile::default()
            }),
        ))
    }

    fn compile_native_generic_guarded_call_return(
        &mut self,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        let expected = [
            crate::OpCode::Lt,
            crate::OpCode::Jmp,
            crate::OpCode::Sub,
            crate::OpCode::LeI,
            crate::OpCode::Jmp,
            crate::OpCode::GetUpval,
            crate::OpCode::Move,
            crate::OpCode::Move,
            crate::OpCode::Move,
            crate::OpCode::Call,
            crate::OpCode::Return0,
        ];
        if ir.root_pc != 0 || ir.insts.len() != expected.len() || ir.guards.len() != 2 {
            return None;
        }
        if ir.insts.iter().map(|inst| inst.opcode).ne(expected) {
            return None;
        }

        let first_guard_meta = ir.guards.iter().find(|guard| {
            guard.guard_pc == ir.insts[0].pc
                && guard.branch_pc == ir.insts[1].pc
                && !guard.taken_on_trace
        })?;
        let second_guard_meta = ir.guards.iter().find(|guard| {
            guard.guard_pc == ir.insts[3].pc
                && guard.branch_pc == ir.insts[4].pc
                && !guard.taken_on_trace
        })?;

        let first_exit = lowered_trace.deopt_target_for_exit_pc(first_guard_meta.exit_pc)?;
        let second_exit = lowered_trace.deopt_target_for_exit_pc(second_guard_meta.exit_pc)?;
        let first_guard =
            lower_numeric_guard_for_native(&ir.insts[0], false, first_exit.resume_pc)?;
        let second_guard =
            lower_numeric_guard_for_native(&ir.insts[3], false, second_exit.resume_pc)?;
        let middle_steps = lower_numeric_steps_for_native(&ir.insts[2..3], lowered_trace)?;

        let call_inst = &ir.insts[9];
        let call_raw = Instruction::from_u32(call_inst.raw_instruction);
        if call_raw.get_b() == 0 || call_raw.get_c() == 0 {
            return None;
        }
        let call_live_out = (call_raw.get_a()..call_raw.get_a() + call_raw.get_b())
            .collect::<Vec<_>>();
        let prep_steps = lower_numeric_steps_for_native_with_live_out(
            &ir.insts[5..9],
            lowered_trace,
            &call_live_out,
        )?;
        if !middle_steps.iter().all(native_supports_numeric_step) {
            return None;
        }
        if !prep_steps.iter().all(native_supports_numeric_step) {
            return None;
        }

        let native = self.compile_native_guarded_call_return_loop(
            first_guard,
            first_exit.exit_index,
            &middle_steps,
            second_guard,
            second_exit.exit_index,
            &prep_steps,
            call_raw.get_a(),
            call_raw.get_b(),
            call_raw.get_c(),
            call_inst.pc,
            lowered_trace,
        )?;
        Some((
            CompiledTraceExecution::Native(native),
            Some(NativeLoweringProfile {
                guard_steps: 2,
                arithmetic_helper_steps: 1,
                ..NativeLoweringProfile::default()
            }),
        ))
    }

    fn compile_native_guarded_call_return_loop(
        &mut self,
        first_guard: NumericJmpLoopGuard,
        first_exit_index: u16,
        middle_steps: &[NumericStep],
        second_guard: NumericJmpLoopGuard,
        second_exit_index: u16,
        prep_steps: &[NumericStep],
        call_a: u32,
        call_b: u32,
        call_c: u32,
        call_pc: u32,
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        let func_name = self.allocate_function_name("jit_native_guarded_call_return");
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let native_helpers = declare_native_helpers(
            &mut module,
            &mut context.func,
            pointer_ty,
            target_config.default_call_conv,
        )
        .ok()?;
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();
        let mut current_numeric_values = Vec::new();

        let entry_block = builder.create_block();
        let after_first_guard_block = builder.create_block();
        let after_second_guard_block = builder.create_block();
        let call_continue_block = builder.create_block();
        let first_side_exit_block = builder.create_block();
        let second_side_exit_block = builder.create_block();
        let fallback_block = builder.create_block();

        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        builder.ins().jump(entry_block, &[]);

        builder.switch_to_block(entry_block);
        let current_hits = builder.use_var(hits_var);
        emit_numeric_guard_flow(
            &mut builder,
            &abi,
            &native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            match first_guard {
                NumericJmpLoopGuard::Head {
                    cond,
                    continue_when: _,
                    continue_preset,
                    exit_preset,
                    ..
                }
                | NumericJmpLoopGuard::Tail {
                    cond,
                    continue_when: _,
                    continue_preset,
                    exit_preset,
                    ..
                } => {
                    debug_assert!(continue_preset.is_none());
                    debug_assert!(exit_preset.is_none());
                    cond
                }
            },
            match first_guard {
                NumericJmpLoopGuard::Head { continue_when, .. }
                | NumericJmpLoopGuard::Tail { continue_when, .. } => continue_when,
            },
            None,
            None,
            after_first_guard_block,
            first_side_exit_block,
            &mut known_value_kinds,
            &mut current_numeric_values,
            None,
            HoistedNumericGuardValues::default(),
        )?;

        builder.switch_to_block(after_first_guard_block);
        builder.seal_block(after_first_guard_block);
        for &step in middle_steps {
            emit_numeric_step(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_block,
                step,
                &mut known_value_kinds,
                &mut current_numeric_values,
                None,
                HoistedNumericGuardValues::default(),
            )?;
        }
        emit_numeric_guard_flow(
            &mut builder,
            &abi,
            &native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            match second_guard {
                NumericJmpLoopGuard::Head {
                    cond,
                    continue_when: _,
                    continue_preset,
                    exit_preset,
                    ..
                }
                | NumericJmpLoopGuard::Tail {
                    cond,
                    continue_when: _,
                    continue_preset,
                    exit_preset,
                    ..
                } => {
                    debug_assert!(continue_preset.is_none());
                    debug_assert!(exit_preset.is_none());
                    cond
                }
            },
            match second_guard {
                NumericJmpLoopGuard::Head { continue_when, .. }
                | NumericJmpLoopGuard::Tail { continue_when, .. } => continue_when,
            },
            None,
            None,
            after_second_guard_block,
            second_side_exit_block,
            &mut known_value_kinds,
            &mut current_numeric_values,
            None,
            HoistedNumericGuardValues::default(),
        )?;

        builder.switch_to_block(after_second_guard_block);
        builder.seal_block(after_second_guard_block);
        for &step in prep_steps {
            emit_numeric_step(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_block,
                step,
                &mut known_value_kinds,
                &mut current_numeric_values,
                None,
                HoistedNumericGuardValues::default(),
            )?;
        }

        let call_a_value = builder.ins().iconst(types::I32, i64::from(call_a));
        let call_b_value = builder.ins().iconst(types::I32, i64::from(call_b));
        let call_c_value = builder.ins().iconst(types::I32, i64::from(call_c));
        let call_pc_value = builder.ins().iconst(types::I32, i64::from(call_pc));
        let helper_call = builder.ins().call(
            native_helpers.call,
            &[
                abi.lua_state_ptr,
                abi.base_ptr,
                abi.base_slots,
                call_a_value,
                call_b_value,
                call_c_value,
                call_pc_value,
            ],
        );
        let helper_status = builder.inst_results(helper_call)[0];
        let helper_continue = builder
            .ins()
            .icmp_imm(IntCC::Equal, helper_status, i64::from(NATIVE_CALL_CONTINUE));
        builder.ins().brif(
            helper_continue,
            call_continue_block,
            &[],
            fallback_block,
            &[],
        );

        builder.switch_to_block(call_continue_block);
        builder.seal_block(call_continue_block);
        emit_native_return_result(&mut builder, abi.result_ptr, 0, 0);

        builder.switch_to_block(first_side_exit_block);
        let first_hits = builder.ins().iconst(types::I64, 1);
        emit_store_native_result(
            &mut builder,
            abi.result_ptr,
            NativeTraceStatus::SideExit,
            first_hits,
            0,
            first_exit_index,
        );
        builder.ins().return_(&[]);
        builder.seal_block(first_side_exit_block);

        builder.switch_to_block(second_side_exit_block);
        let second_hits = builder.ins().iconst(types::I64, 1);
        emit_store_native_result(
            &mut builder,
            abi.result_ptr,
            NativeTraceStatus::SideExit,
            second_hits,
            0,
            second_exit_index,
        );
        builder.ins().return_(&[]);
        builder.seal_block(second_side_exit_block);

        emit_native_terminal_result(
            &mut builder,
            fallback_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Fallback,
            None,
            None,
        );

        builder.seal_block(entry_block);
        builder.finalize();
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::GuardedCallPrefix {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    fn compile_native_generic_numeric_jmp(
        &mut self,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
    ) -> Option<(CompiledTraceExecution, Option<NativeLoweringProfile>)> {
        let backedge = ir.insts.last()?;
        if backedge.opcode != crate::OpCode::Jmp {
            return None;
        }
        if Instruction::from_u32(backedge.raw_instruction).get_sj() >= 0 {
            return None;
        }
        if ir.guards.is_empty() {
            return None;
        }

        let (head_blocks, lowering, tail_blocks, interior_guard_start) =
            recognize_numeric_jmp_guard_blocks(ir, lowered_trace)?;
        let native = self.compile_native_numeric_jmp_loop(
            &head_blocks,
            &lowering,
            &tail_blocks,
            interior_guard_start,
            lowered_trace,
        )?;
        let profile = profile_for_numeric_jmp_loop(&head_blocks, &lowering.steps, &tail_blocks);
        Some((CompiledTraceExecution::Native(native), Some(profile)))
    }

    fn compile_native_return0(&mut self) -> Option<NativeCompiledTrace> {
        self.compile_native_return_trace("jit_native_return0", 0, 0, NativeReturnKind::Return0)
    }

    fn compile_native_return1(&mut self, src_reg: u32) -> Option<NativeCompiledTrace> {
        self.compile_native_return_trace(
            "jit_native_return1",
            src_reg,
            1,
            NativeReturnKind::Return1,
        )
    }

    fn compile_native_return(
        &mut self,
        start_reg: u32,
        result_count: u8,
    ) -> Option<NativeCompiledTrace> {
        self.compile_native_return_trace(
            "jit_native_return",
            start_reg,
            u32::from(result_count),
            NativeReturnKind::Return,
        )
    }

    fn build_module() -> Result<JITModule, String> {
        let mut flags = settings::builder();
        let _ = flags.set("opt_level", "speed");
        let isa = cranelift_native::builder()
            .map_err(|err| err.to_string())?
            .finish(settings::Flags::new(flags))
            .map_err(|err| err.to_string())?;
        let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
        builder.symbol(
            NATIVE_HELPER_NUMERIC_GET_UPVAL_SYMBOL,
            jit_native_helper_numeric_get_upval as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_SET_UPVAL_SYMBOL,
            jit_native_helper_numeric_set_upval as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_GET_TABUP_FIELD_SYMBOL,
            jit_native_helper_numeric_get_tabup_field as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_SET_TABUP_FIELD_SYMBOL,
            jit_native_helper_numeric_set_tabup_field as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_GET_TABLE_INT_SYMBOL,
            jit_native_helper_numeric_get_table_int as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_SET_TABLE_INT_SYMBOL,
            jit_native_helper_numeric_set_table_int as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_GET_TABLE_FIELD_SYMBOL,
            jit_native_helper_numeric_get_table_field as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_SET_TABLE_FIELD_SYMBOL,
            jit_native_helper_numeric_set_table_field as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_LEN_SYMBOL,
            jit_native_helper_numeric_len as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_BINARY_SYMBOL,
            jit_native_helper_numeric_binary as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_POW_SYMBOL,
            jit_native_helper_numeric_pow as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_SHIFT_LEFT_SYMBOL,
            jit_native_helper_shift_left as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_SHIFT_RIGHT_SYMBOL,
            jit_native_helper_shift_right as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_CALL_SYMBOL,
            jit_native_helper_call as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_TFOR_CALL_SYMBOL,
            jit_native_helper_tfor_call as *const u8,
        );
        Ok(JITModule::new(builder))
    }

    fn compile_native_return_trace(
        &mut self,
        prefix: &str,
        start_reg: u32,
        result_count: u32,
        kind: NativeReturnKind,
    ) -> Option<NativeCompiledTrace> {
        let func_name = self.allocate_function_name(prefix);
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let abi = init_native_entry(&mut builder, pointer_ty);
        emit_native_return_result(&mut builder, abi.result_ptr, start_reg, result_count);
        builder.finalize();
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(match kind {
            NativeReturnKind::Return => NativeCompiledTrace::Return {
                entry: unsafe { std::mem::transmute(entry) },
            },
            NativeReturnKind::Return0 => NativeCompiledTrace::Return0 {
                entry: unsafe { std::mem::transmute(entry) },
            },
            NativeReturnKind::Return1 => NativeCompiledTrace::Return1 {
                entry: unsafe { std::mem::transmute(entry) },
            },
        })
    }

    fn compile_native_linear_int_for_loop(
        &mut self,
        loop_reg: u32,
        steps: &[LinearIntStep],
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        let func_name = self.allocate_function_name("jit_native_linear_int_for_loop");
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let native_helpers = declare_native_helpers(
            &mut module,
            &mut context.func,
            pointer_ty,
            target_config.default_call_conv,
        )
        .ok()?;
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);
        let carried_remaining_var = builder.declare_var(types::I64);
        let carried_index_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);

        let loop_state_is_invariant = linear_int_loop_state_is_invariant(loop_reg, steps);

        let loop_block = builder.create_block();
        if loop_state_is_invariant {
            builder.append_block_param(loop_block, types::I64);
            builder.append_block_param(loop_block, types::I64);
        }
        let fallback_terminal_block = builder.create_block();
        let loop_exit_terminal_block = builder.create_block();
        let fallback_block = if loop_state_is_invariant {
            builder.create_block()
        } else {
            fallback_terminal_block
        };
        let loop_exit_block = if loop_state_is_invariant {
            builder.create_block()
        } else {
            loop_exit_terminal_block
        };
        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        let mut known_integer_regs = lowered_trace
            .entry_ssa_register_hints()
            .into_iter()
            .filter_map(|hint| matches!(hint.kind, TraceValueKind::Integer).then_some(hint.reg))
            .collect::<Vec<_>>();
        if loop_state_is_invariant {
            known_integer_regs.push(loop_reg);
            known_integer_regs.push(loop_reg.saturating_add(1));
            known_integer_regs.push(loop_reg.saturating_add(2));
        }
        let hoisted_step_value = if loop_state_is_invariant {
            let loop_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg);
            let step_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(1));
            let index_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(2));
            emit_integer_guard(&mut builder, loop_ptr, hits_var, zero_hits, fallback_block);
            emit_integer_guard(&mut builder, step_ptr, hits_var, zero_hits, fallback_block);
            emit_integer_guard(&mut builder, index_ptr, hits_var, zero_hits, fallback_block);
            Some(builder.ins().load(
                types::I64,
                MemFlags::new(),
                step_ptr,
                LUA_VALUE_VALUE_OFFSET,
            ))
        } else {
            None
        };
        let initial_remaining = if loop_state_is_invariant {
            let loop_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg);
            builder.ins().load(
                types::I64,
                MemFlags::new(),
                loop_ptr,
                LUA_VALUE_VALUE_OFFSET,
            )
        } else {
            builder.ins().iconst(types::I64, 0)
        };
        let initial_index = if loop_state_is_invariant {
            let index_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(2));
            builder.ins().load(
                types::I64,
                MemFlags::new(),
                index_ptr,
                LUA_VALUE_VALUE_OFFSET,
            )
        } else {
            builder.ins().iconst(types::I64, 0)
        };

        if loop_state_is_invariant {
            builder.ins().jump(
                loop_block,
                &[
                    cranelift::codegen::ir::BlockArg::Value(initial_remaining),
                    cranelift::codegen::ir::BlockArg::Value(initial_index),
                ],
            );
        } else {
            builder.ins().jump(loop_block, &[]);
        }

        builder.switch_to_block(loop_block);
        let current_hits = builder.use_var(hits_var);
        let loop_carried_values = if let Some(step_value) = hoisted_step_value {
            let carried_remaining = builder.block_params(loop_block)[0];
            let carried_index = builder.block_params(loop_block)[1];
            builder.def_var(carried_remaining_var, carried_remaining);
            builder.def_var(carried_index_var, carried_index);
            vec![
                (loop_reg, carried_remaining),
                (loop_reg.saturating_add(1), step_value),
                (loop_reg.saturating_add(2), carried_index),
            ]
        } else {
            Vec::new()
        };
        let mut current_integer_values = Vec::new();

        for step in steps {
            emit_linear_int_step(
                &mut builder,
                &native_helpers,
                abi.base_ptr,
                hits_var,
                current_hits,
                fallback_block,
                *step,
                &mut known_integer_regs,
                &mut current_integer_values,
                &loop_carried_values,
            );
        }

        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        if loop_state_is_invariant {
            let carried_remaining = builder.use_var(carried_remaining_var);
            let carried_index = builder.use_var(carried_index_var);
            emit_linear_int_counted_loop_backedge(
                &mut builder,
                hits_var,
                next_hits,
                carried_remaining,
                carried_index,
                hoisted_step_value,
                loop_block,
                loop_exit_block,
            );
        } else {
            emit_counted_loop_backedge(
                &mut builder,
                abi.base_ptr,
                hits_var,
                current_hits,
                next_hits,
                loop_reg,
                None,
                false,
                loop_block,
                loop_exit_terminal_block,
                fallback_terminal_block,
            );
        }

        if loop_state_is_invariant {
            emit_linear_int_materialize_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_reg,
                carried_remaining_var,
                carried_index_var,
                fallback_block,
                fallback_terminal_block,
            );
            emit_linear_int_materialize_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_reg,
                carried_remaining_var,
                carried_index_var,
                loop_exit_block,
                loop_exit_terminal_block,
            );
        }

        emit_native_terminal_result(
            &mut builder,
            loop_exit_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::LoopExit,
            None,
            None,
        );
        emit_native_terminal_result(
            &mut builder,
            fallback_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Fallback,
            None,
            None,
        );

        builder.seal_block(loop_block);
        builder.finalize();
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::LinearIntForLoop {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    fn compile_native_linear_int_jmp_loop(
        &mut self,
        steps: &[LinearIntStep],
        guard: LinearIntLoopGuard,
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        let exit_pc = guard.exit_pc();
        let exit_index = lowered_trace.deopt_target_for_exit_pc(exit_pc)?.exit_index;
        let continue_when = match guard {
            LinearIntLoopGuard::HeadRegReg { continue_when, .. }
            | LinearIntLoopGuard::HeadRegImm { continue_when, .. }
            | LinearIntLoopGuard::TailRegReg { continue_when, .. }
            | LinearIntLoopGuard::TailRegImm { continue_when, .. } => continue_when,
        };
        let func_name = self.allocate_function_name("jit_native_linear_int_jmp_loop");
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let native_helpers = declare_native_helpers(
            &mut module,
            &mut context.func,
            pointer_ty,
            target_config.default_call_conv,
        )
        .ok()?;
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);

        let guard_block = builder.create_block();
        let body_block = builder.create_block();
        let fallback_block = builder.create_block();
        let side_exit_block = builder.create_block();

        let mut known_integer_regs = Vec::new();
        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        builder.ins().jump(guard_block, &[]);

        builder.switch_to_block(guard_block);
        let current_hits = builder.use_var(hits_var);
        let empty_current_integer_values = Vec::new();
        let loop_carried_values = Vec::new();
        if guard.is_head() {
            let cond = emit_linear_int_guard_condition(
                &mut builder,
                abi.base_ptr,
                hits_var,
                current_hits,
                fallback_block,
                &known_integer_regs,
                &empty_current_integer_values,
                &loop_carried_values,
                guard,
            );
            builder.def_var(hits_var, current_hits);
            if continue_when {
                builder
                    .ins()
                    .brif(cond, body_block, &[], side_exit_block, &[]);
            } else {
                builder
                    .ins()
                    .brif(cond, side_exit_block, &[], body_block, &[]);
            }
        } else {
            builder.ins().jump(body_block, &[]);
        }

        builder.switch_to_block(body_block);
        let mut current_integer_values = Vec::new();
        for step in steps {
            emit_linear_int_step(
                &mut builder,
                &native_helpers,
                abi.base_ptr,
                hits_var,
                current_hits,
                fallback_block,
                *step,
                &mut known_integer_regs,
                &mut current_integer_values,
                &loop_carried_values,
            );
        }

        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        if guard.is_tail() {
            let cond = emit_linear_int_guard_condition(
                &mut builder,
                abi.base_ptr,
                hits_var,
                next_hits,
                fallback_block,
                &known_integer_regs,
                &current_integer_values,
                &loop_carried_values,
                guard,
            );
            builder.def_var(hits_var, next_hits);
            if continue_when {
                builder
                    .ins()
                    .brif(cond, guard_block, &[], side_exit_block, &[]);
            } else {
                builder
                    .ins()
                    .brif(cond, side_exit_block, &[], guard_block, &[]);
            }
        } else {
            builder.def_var(hits_var, next_hits);
            builder.ins().jump(guard_block, &[]);
        }

        emit_native_terminal_result(
            &mut builder,
            side_exit_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::SideExit,
            Some(exit_pc),
            Some(exit_index),
        );
        emit_native_terminal_result(
            &mut builder,
            fallback_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Fallback,
            None,
            None,
        );

        builder.seal_block(guard_block);
        builder.seal_block(body_block);
        builder.finalize();
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::LinearIntJmpLoop {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    fn compile_native_tfor_loop(
        &mut self,
        loop_reg: u32,
        result_count: u32,
        tforcall_pc: u32,
        exit_pc: u32,
        exit_index: u16,
        steps: &[NumericStep],
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        if !steps.iter().all(native_supports_numeric_step) {
            return None;
        }

        let func_name = self.allocate_function_name("jit_native_tfor_loop");
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let native_helpers = declare_native_helpers(
            &mut module,
            &mut context.func,
            pointer_ty,
            target_config.default_call_conv,
        )
        .ok()?;
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();

        let loop_block = builder.create_block();
        let call_continuation_block = builder.create_block();
        let fallback_block = builder.create_block();
        let returned_block = builder.create_block();
        let side_exit_block = builder.create_block();
        let continue_block = builder.create_block();
        let helper_non_continue_block = builder.create_block();

        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        builder.ins().jump(loop_block, &[]);

        builder.switch_to_block(loop_block);
        let current_hits = builder.use_var(hits_var);
        let mut current_numeric_values = Vec::new();
        for step in steps {
            emit_numeric_step(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_block,
                *step,
                &mut known_value_kinds,
                &mut current_numeric_values,
                None,
                HoistedNumericGuardValues::default(),
            )?;
        }

        let loop_reg_value = builder.ins().iconst(types::I32, i64::from(loop_reg));
        let result_count_value = builder.ins().iconst(types::I32, i64::from(result_count));
        let tforcall_pc_value = builder.ins().iconst(types::I32, i64::from(tforcall_pc));
        let helper_call = builder.ins().call(
            native_helpers.tfor_call,
            &[
                abi.lua_state_ptr,
                abi.base_slots,
                loop_reg_value,
                result_count_value,
                tforcall_pc_value,
            ],
        );
        let helper_status = builder.inst_results(helper_call)[0];
        let helper_continue = builder.ins().icmp_imm(
            IntCC::Equal,
            helper_status,
            i64::from(NATIVE_TFOR_CALL_C_CONTINUE),
        );
        let helper_returned = builder.ins().icmp_imm(
            IntCC::Equal,
            helper_status,
            i64::from(NATIVE_TFOR_CALL_LUA_RETURNED),
        );
        builder.ins().brif(
            helper_continue,
            call_continuation_block,
            &[],
            helper_non_continue_block,
            &[],
        );

        builder.switch_to_block(helper_non_continue_block);
        builder
            .ins()
            .brif(helper_returned, returned_block, &[], fallback_block, &[]);

        builder.switch_to_block(call_continuation_block);
        let control_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(3));
        let control_tag =
            builder
                .ins()
                .load(types::I8, MemFlags::new(), control_ptr, LUA_VALUE_TT_OFFSET);
        let control_is_nil =
            builder
                .ins()
                .icmp_imm(IntCC::Equal, control_tag, i64::from(LUA_VNIL_TAG));
        let continue_hits = builder.ins().iadd_imm(current_hits, 1);
        builder
            .ins()
            .brif(control_is_nil, side_exit_block, &[], continue_block, &[]);

        builder.switch_to_block(continue_block);
        builder.def_var(hits_var, continue_hits);
        builder.ins().jump(loop_block, &[]);

        emit_native_terminal_result(
            &mut builder,
            fallback_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Fallback,
            None,
            None,
        );
        emit_native_terminal_result(
            &mut builder,
            returned_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Returned,
            None,
            None,
        );
        emit_native_terminal_result(
            &mut builder,
            side_exit_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::SideExit,
            Some(exit_pc),
            Some(exit_index),
        );

        builder.seal_block(loop_block);
        builder.seal_block(call_continuation_block);
        builder.seal_block(continue_block);
        builder.seal_block(helper_non_continue_block);
        builder.finalize();
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::TForLoop {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    pub fn compile_native_numeric_for_loop(
        &mut self,
        loop_reg: u32,
        lowering: &NumericLowering,
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        let (prologue_steps, body_steps) = extract_loop_prologue(&lowering.steps, lowered_trace);
        let effective_value_state = if prologue_steps.is_empty() {
            lowering.value_state
        } else {
            build_numeric_value_state(&body_steps, lowered_trace)
        };
        let steps = &body_steps;
        if !steps.iter().all(native_supports_numeric_step) {
            return None;
        }
        if !prologue_steps.iter().all(native_supports_numeric_step) {
            return None;
        }

        let func_name = self.allocate_function_name("jit_native_numeric_for_loop");
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let native_helpers = declare_native_helpers(
            &mut module,
            &mut context.func,
            pointer_ty,
            target_config.default_call_conv,
        )
        .ok()?;
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);
        let carried_remaining_var = builder.declare_var(types::I64);
        let carried_index_var = builder.declare_var(types::I64);
        let carried_integer_var = builder.declare_var(types::I64);
        let carried_float_raw_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();
        let loop_state_is_invariant = numeric_loop_state_is_invariant(loop_reg, steps);
        let carried_integer_flow = if loop_state_is_invariant {
            effective_value_state
                .self_update
                .filter(|flow| matches!(flow.kind, NumericSelfUpdateValueKind::Integer))
        } else {
            None
        };
        let carried_integer_step = carried_integer_flow
            .and_then(carried_integer_loop_step_from_value_flow)
            .filter(|step| {
                carried_integer_rhs_stable_reg(*step)
                    .is_none_or(|reg| numeric_steps_preserve_reg(steps, reg))
            });
        let carried_integer_span =
            carried_integer_flow.and_then(|flow| integer_self_update_step_span(steps, flow));
        let carried_float_flow = if loop_state_is_invariant && carried_integer_step.is_none() {
            effective_value_state
                .self_update
                .filter(|flow| matches!(flow.kind, NumericSelfUpdateValueKind::Float))
        } else {
            None
        };
        let carried_float_step = carried_float_flow
            .and_then(|flow| carried_float_loop_step_from_value_flow(flow, lowered_trace))
            .filter(|step| {
                carried_float_rhs_stable_reg(*step)
                    .is_none_or(|reg| numeric_steps_preserve_reg(steps, reg))
            });
        let carried_float_span =
            carried_float_flow.and_then(|flow| float_self_update_step_span(steps, flow));

        let loop_block = builder.create_block();
        if loop_state_is_invariant {
            builder.append_block_param(loop_block, types::I64);
            builder.append_block_param(loop_block, types::I64);
        }
        if carried_integer_step.is_some() {
            builder.append_block_param(loop_block, types::I64);
        }
        if carried_float_step.is_some() {
            builder.append_block_param(loop_block, types::I64);
        }
        let fallback_terminal_block = builder.create_block();
        let loop_exit_terminal_block = builder.create_block();
        let fallback_block = if loop_state_is_invariant {
            builder.create_block()
        } else {
            fallback_terminal_block
        };
        let loop_exit_block = if loop_state_is_invariant {
            builder.create_block()
        } else {
            loop_exit_terminal_block
        };

        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        let hoisted_step_value = if loop_state_is_invariant {
            let loop_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg);
            let step_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(1));
            let index_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(2));
            emit_integer_guard(&mut builder, loop_ptr, hits_var, zero_hits, fallback_block);
            emit_integer_guard(&mut builder, step_ptr, hits_var, zero_hits, fallback_block);
            emit_integer_guard(&mut builder, index_ptr, hits_var, zero_hits, fallback_block);
            set_numeric_reg_value_kind(&mut known_value_kinds, loop_reg, TraceValueKind::Integer);
            set_numeric_reg_value_kind(
                &mut known_value_kinds,
                loop_reg.saturating_add(1),
                TraceValueKind::Integer,
            );
            set_numeric_reg_value_kind(
                &mut known_value_kinds,
                loop_reg.saturating_add(2),
                TraceValueKind::Integer,
            );
            Some(builder.ins().load(
                types::I64,
                MemFlags::new(),
                step_ptr,
                LUA_VALUE_VALUE_OFFSET,
            ))
        } else {
            None
        };
        let initial_remaining = if loop_state_is_invariant {
            let loop_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg);
            builder.ins().load(
                types::I64,
                MemFlags::new(),
                loop_ptr,
                LUA_VALUE_VALUE_OFFSET,
            )
        } else {
            builder.ins().iconst(types::I64, 0)
        };
        let initial_index = if loop_state_is_invariant {
            let index_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(2));
            builder.ins().load(
                types::I64,
                MemFlags::new(),
                index_ptr,
                LUA_VALUE_VALUE_OFFSET,
            )
        } else {
            builder.ins().iconst(types::I64, 0)
        };

        // Emit prologue steps (loop-invariant code hoisted out of the loop body).
        // These run once in the entry block and write to stack slots that the loop
        // body will read.  If any prologue helper fails, fall back directly with
        // hits = 0.  We use fallback_terminal_block (not the materialization
        // fallback_block) because no loop iterations have executed yet—there is
        // no carried loop state to materialize.
        {
            let mut prologue_numeric_values = Vec::new();
            for &step in &prologue_steps {
                emit_numeric_step(
                    &mut builder,
                    &abi,
                    &native_helpers,
                    hits_var,
                    zero_hits,
                    fallback_terminal_block,
                    step,
                    &mut known_value_kinds,
                    &mut prologue_numeric_values,
                    None,
                    HoistedNumericGuardValues::default(),
                )?;
            }
        }

        let initial_integer_value = if let Some(step) = carried_integer_step {
            let slot_ptr = slot_addr(&mut builder, abi.base_ptr, step.reg);
            emit_exact_tag_guard(
                &mut builder,
                slot_ptr,
                LUA_VNUMINT,
                hits_var,
                zero_hits,
                fallback_block,
            );
            set_numeric_reg_value_kind(&mut known_value_kinds, step.reg, TraceValueKind::Integer);
            Some(builder.ins().load(
                types::I64,
                MemFlags::new(),
                slot_ptr,
                LUA_VALUE_VALUE_OFFSET,
            ))
        } else {
            None
        };
        let initial_float_raw = if let Some(step) = carried_float_step {
            let slot_ptr = slot_addr(&mut builder, abi.base_ptr, step.reg);
            emit_exact_tag_guard(
                &mut builder,
                slot_ptr,
                LUA_VNUMFLT,
                hits_var,
                zero_hits,
                fallback_block,
            );
            set_numeric_reg_value_kind(&mut known_value_kinds, step.reg, TraceValueKind::Float);
            Some(builder.ins().load(
                types::I64,
                MemFlags::new(),
                slot_ptr,
                LUA_VALUE_VALUE_OFFSET,
            ))
        } else {
            None
        };
        let carried_float_rhs = carried_float_step.map(|step| {
            resolve_carried_float_rhs(
                &mut builder,
                abi.base_ptr,
                hits_var,
                zero_hits,
                fallback_block,
                step,
            )
        });
        let carried_integer_rhs = carried_integer_step.map(|step| {
            resolve_carried_integer_rhs(
                &mut builder,
                abi.base_ptr,
                hits_var,
                zero_hits,
                fallback_block,
                step,
            )
        });

        let mut initial_args = Vec::new();
        if loop_state_is_invariant {
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(initial_remaining));
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(initial_index));
        }
        if let Some(value) = initial_integer_value {
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(value));
        }
        if let Some(raw) = initial_float_raw {
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(raw));
        }
        builder.ins().jump(loop_block, &initial_args);

        builder.switch_to_block(loop_block);
        let current_hits = builder.use_var(hits_var);
        if loop_state_is_invariant {
            let carried_remaining = builder.block_params(loop_block)[0];
            let carried_index = builder.block_params(loop_block)[1];
            builder.def_var(carried_remaining_var, carried_remaining);
            builder.def_var(carried_index_var, carried_index);
        }
        if carried_integer_step.is_some() {
            let integer_param_index = if loop_state_is_invariant { 2 } else { 0 };
            let carried_integer = builder.block_params(loop_block)[integer_param_index];
            builder.def_var(carried_integer_var, carried_integer);
        }
        if carried_float_step.is_some() {
            let float_param_index = if loop_state_is_invariant {
                2 + usize::from(carried_integer_step.is_some())
            } else {
                usize::from(carried_integer_step.is_some())
            };
            let carried_float_raw = builder.block_params(loop_block)[float_param_index];
            builder.def_var(carried_float_raw_var, carried_float_raw);
        }

        if let Some(step) = carried_integer_step {
            let (span_start, span_len) = carried_integer_span
                .expect("plain numeric carried-integer path requires a matching self-update span");
            let mut current_numeric_values = Vec::new();
            emit_numeric_steps_with_carried_integer(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_block,
                steps,
                carried_integer_var,
                step,
                carried_integer_rhs
                    .expect("plain numeric carried-integer path requires resolved rhs"),
                span_start,
                span_len,
                None,
                &mut known_value_kinds,
                &mut current_numeric_values,
            )?;
        } else if let Some(step) = carried_float_step {
            if let Some((span_start, span_len)) = carried_float_span {
                let mut current_numeric_values = Vec::new();
                emit_numeric_steps_with_carried_float(
                    &mut builder,
                    &abi,
                    &native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    steps,
                    carried_float_raw_var,
                    step,
                    carried_float_rhs
                        .expect("plain numeric carried-float path requires resolved rhs"),
                    span_start,
                    span_len,
                    None,
                    &mut known_value_kinds,
                    &mut current_numeric_values,
                )?;
            } else {
                emit_carried_float_loop_step(
                    &mut builder,
                    carried_float_raw_var,
                    step,
                    carried_float_rhs
                        .expect("plain numeric carried-float path requires resolved rhs"),
                    &mut known_value_kinds,
                );
            }
        } else {
            let mut current_numeric_values = Vec::new();
            for step in steps {
                emit_numeric_step(
                    &mut builder,
                    &abi,
                    &native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    *step,
                    &mut known_value_kinds,
                    &mut current_numeric_values,
                    None,
                    HoistedNumericGuardValues::default(),
                )?;
            }
        }

        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        if loop_state_is_invariant {
            let carried_remaining = builder.use_var(carried_remaining_var);
            let carried_index = builder.use_var(carried_index_var);
            if carried_integer_step.is_some() {
                let carried_integer = builder.use_var(carried_integer_var);
                emit_numeric_counted_loop_backedge_with_carried_integer(
                    &mut builder,
                    hits_var,
                    next_hits,
                    carried_remaining,
                    carried_index,
                    hoisted_step_value,
                    carried_integer,
                    loop_block,
                    loop_exit_block,
                );
            } else if carried_float_step.is_some() {
                let carried_float_raw = builder.use_var(carried_float_raw_var);
                emit_numeric_counted_loop_backedge_with_carried_float(
                    &mut builder,
                    hits_var,
                    next_hits,
                    carried_remaining,
                    carried_index,
                    hoisted_step_value,
                    carried_float_raw,
                    loop_block,
                    loop_exit_block,
                );
            } else {
                emit_linear_int_counted_loop_backedge(
                    &mut builder,
                    hits_var,
                    next_hits,
                    carried_remaining,
                    carried_index,
                    hoisted_step_value,
                    loop_block,
                    loop_exit_block,
                );
            }
        } else {
            emit_counted_loop_backedge(
                &mut builder,
                abi.base_ptr,
                hits_var,
                current_hits,
                next_hits,
                loop_reg,
                None,
                false,
                loop_block,
                loop_exit_terminal_block,
                fallback_terminal_block,
            );
        }

        if loop_state_is_invariant || carried_float_step.is_some() {
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_state_is_invariant.then_some((
                    loop_reg,
                    carried_remaining_var,
                    carried_index_var,
                )),
                carried_integer_step.map(|step| (step.reg, carried_integer_var)),
                carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                fallback_block,
                fallback_terminal_block,
            );
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_state_is_invariant.then_some((
                    loop_reg,
                    carried_remaining_var,
                    carried_index_var,
                )),
                carried_integer_step.map(|step| (step.reg, carried_integer_var)),
                carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                loop_exit_block,
                loop_exit_terminal_block,
            );
        }

        emit_native_terminal_result(
            &mut builder,
            loop_exit_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::LoopExit,
            None,
            None,
        );
        emit_native_terminal_result(
            &mut builder,
            fallback_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Fallback,
            None,
            None,
        );

        builder.seal_block(loop_block);
        builder.finalize();
        if module.define_function(func_id, &mut context).is_err() {
            return None;
        }
        module.clear_context(&mut context);
        if module.finalize_definitions().is_err() {
            return None;
        }
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::NumericForLoop {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    fn compile_native_call_for_loop(
        &mut self,
        loop_reg: u32,
        prep_steps: &[NumericStep],
        post_steps: &[NumericStep],
        call_a: u32,
        call_b: u32,
        call_c: u32,
        call_pc: u32,
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        let func_name = self.allocate_function_name("jit_native_call_for_loop");
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let native_helpers = declare_native_helpers(
            &mut module,
            &mut context.func,
            pointer_ty,
            target_config.default_call_conv,
        )
        .ok()?;
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();

        let loop_block = builder.create_block();
        let call_continue_block = builder.create_block();
        let fallback_terminal_block = builder.create_block();
        let loop_exit_terminal_block = builder.create_block();

        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        builder.ins().jump(loop_block, &[]);

        builder.switch_to_block(loop_block);
        let current_hits = builder.use_var(hits_var);
        let mut current_numeric_values = Vec::new();
        for step in prep_steps {
            emit_numeric_step(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_terminal_block,
                *step,
                &mut known_value_kinds,
                &mut current_numeric_values,
                None,
                HoistedNumericGuardValues::default(),
            )?;
        }

        let call_a_value = builder.ins().iconst(types::I32, i64::from(call_a));
        let call_b_value = builder.ins().iconst(types::I32, i64::from(call_b));
        let call_c_value = builder.ins().iconst(types::I32, i64::from(call_c));
        let call_pc_value = builder.ins().iconst(types::I32, i64::from(call_pc));
        let helper_call = builder.ins().call(
            native_helpers.call,
            &[
                abi.lua_state_ptr,
                abi.base_ptr,
                abi.base_slots,
                call_a_value,
                call_b_value,
                call_c_value,
                call_pc_value,
            ],
        );
        let helper_status = builder.inst_results(helper_call)[0];
        let helper_continue =
            builder
                .ins()
                .icmp_imm(IntCC::Equal, helper_status, i64::from(NATIVE_CALL_CONTINUE));
        builder.ins().brif(
            helper_continue,
            call_continue_block,
            &[],
            fallback_terminal_block,
            &[],
        );

        builder.switch_to_block(call_continue_block);
        builder.seal_block(call_continue_block);
        for step in post_steps {
            emit_numeric_step(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_terminal_block,
                *step,
                &mut known_value_kinds,
                &mut current_numeric_values,
                None,
                HoistedNumericGuardValues::default(),
            )?;
        }
        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        emit_counted_loop_backedge(
            &mut builder,
            abi.base_ptr,
            hits_var,
            current_hits,
            next_hits,
            loop_reg,
            None,
            false,
            loop_block,
            loop_exit_terminal_block,
            fallback_terminal_block,
        );

        emit_native_terminal_result(
            &mut builder,
            loop_exit_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::LoopExit,
            None,
            None,
        );
        emit_native_terminal_result(
            &mut builder,
            fallback_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Fallback,
            None,
            None,
        );

        builder.seal_block(loop_block);
        builder.finalize();
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::CallForLoop {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    fn compile_native_guarded_call_jmp_loop(
        &mut self,
        prep_steps: &[NumericStep],
        post_steps: &[NumericStep],
        guard: NumericJmpLoopGuard,
        call_a: u32,
        call_b: u32,
        call_c: u32,
        call_pc: u32,
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        if !prep_steps.iter().all(native_supports_numeric_step) {
            return None;
        }
        if !post_steps.iter().all(native_supports_numeric_step) {
            return None;
        }

        let (cond, continue_when, continue_preset, exit_preset, side_exit_pc) = match guard {
            NumericJmpLoopGuard::Head {
                cond,
                continue_when,
                continue_preset,
                exit_preset,
                exit_pc,
            } => (cond, continue_when, continue_preset, exit_preset, exit_pc),
            NumericJmpLoopGuard::Tail { .. } => return None,
        };
        let side_exit_index = lowered_trace
            .deopt_target_for_exit_pc(side_exit_pc)?
            .exit_index;

        if !native_supports_numeric_cond(cond)
            || continue_preset
                .as_ref()
                .is_some_and(|step| !native_supports_numeric_step(step))
            || exit_preset
                .as_ref()
                .is_some_and(|step| !native_supports_numeric_step(step))
        {
            return None;
        }

        let func_name = self.allocate_function_name("jit_native_guarded_call_jmp_loop");
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let native_helpers = declare_native_helpers(
            &mut module,
            &mut context.func,
            pointer_ty,
            target_config.default_call_conv,
        )
        .ok()?;
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();

        let loop_block = builder.create_block();
        let guard_continue_block = builder.create_block();
        let helper_continue_block = builder.create_block();
        let side_exit_terminal_block = builder.create_block();
        let fallback_terminal_block = builder.create_block();

        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        builder.ins().jump(loop_block, &[]);

        builder.switch_to_block(loop_block);
        let current_hits = builder.use_var(hits_var);
        let mut current_numeric_values = Vec::new();
        emit_numeric_guard_flow(
            &mut builder,
            &abi,
            &native_helpers,
            hits_var,
            current_hits,
            fallback_terminal_block,
            cond,
            continue_when,
            continue_preset.as_ref(),
            exit_preset.as_ref(),
            guard_continue_block,
            side_exit_terminal_block,
            &mut known_value_kinds,
            &mut current_numeric_values,
            None,
            HoistedNumericGuardValues::default(),
        )?;

        builder.switch_to_block(guard_continue_block);
        builder.seal_block(guard_continue_block);
        for step in prep_steps {
            emit_numeric_step(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_terminal_block,
                *step,
                &mut known_value_kinds,
                &mut current_numeric_values,
                None,
                HoistedNumericGuardValues::default(),
            )?;
        }

        let call_a_value = builder.ins().iconst(types::I32, i64::from(call_a));
        let call_b_value = builder.ins().iconst(types::I32, i64::from(call_b));
        let call_c_value = builder.ins().iconst(types::I32, i64::from(call_c));
        let call_pc_value = builder.ins().iconst(types::I32, i64::from(call_pc));
        let helper_call = builder.ins().call(
            native_helpers.call,
            &[
                abi.lua_state_ptr,
                abi.base_ptr,
                abi.base_slots,
                call_a_value,
                call_b_value,
                call_c_value,
                call_pc_value,
            ],
        );
        let helper_status = builder.inst_results(helper_call)[0];
        let helper_continue =
            builder
                .ins()
                .icmp_imm(IntCC::Equal, helper_status, i64::from(NATIVE_CALL_CONTINUE));
        builder.ins().brif(
            helper_continue,
            helper_continue_block,
            &[],
            fallback_terminal_block,
            &[],
        );

        builder.switch_to_block(helper_continue_block);
        builder.seal_block(helper_continue_block);
        for step in post_steps {
            emit_numeric_step(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_terminal_block,
                *step,
                &mut known_value_kinds,
                &mut current_numeric_values,
                None,
                HoistedNumericGuardValues::default(),
            )?;
        }

        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        builder.def_var(hits_var, next_hits);
        builder.ins().jump(loop_block, &[]);

        emit_native_terminal_result(
            &mut builder,
            side_exit_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::SideExit,
            Some(side_exit_pc),
            Some(side_exit_index),
        );
        emit_native_terminal_result(
            &mut builder,
            fallback_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Fallback,
            None,
            None,
        );

        builder.seal_block(loop_block);
        builder.finalize();
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::GuardedCallJmpLoop {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    fn compile_native_guarded_call_prefix_loop(
        &mut self,
        shape: GuardedCallPrefixShape,
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        let func_name = self.allocate_function_name("jit_native_guarded_call_prefix");
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let native_helpers = declare_native_helpers(
            &mut module,
            &mut context.func,
            pointer_ty,
            target_config.default_call_conv,
        )
        .ok()?;
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();
        let mut current_numeric_values = Vec::new();

        let entry_block = builder.create_block();
        let copy_done_block = builder.create_block();
        let sort_done_block = builder.create_block();
        let check_done_block = builder.create_block();
        let sorted_block = builder.create_block();
        let unsorted_block = builder.create_block();
        let fallback_block = builder.create_block();

        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        builder.ins().jump(entry_block, &[]);

        builder.switch_to_block(entry_block);
        let current_hits = builder.use_var(hits_var);

        for step in [
            NumericStep::Move { dst: 17, src: 6 },
            NumericStep::Move { dst: 18, src: 5 },
        ] {
            emit_numeric_step(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_block,
                step,
                &mut known_value_kinds,
                &mut current_numeric_values,
                None,
                HoistedNumericGuardValues::default(),
            )?;
        }

        let call1_a = builder.ins().iconst(types::I32, 17);
        let call1_b = builder.ins().iconst(types::I32, 2);
        let call1_c = builder.ins().iconst(types::I32, 2);
        let call1_pc = builder.ins().iconst(types::I32, 36);
        let helper_call = builder.ins().call(
            native_helpers.call,
            &[
                abi.lua_state_ptr,
                abi.base_ptr,
                abi.base_slots,
                call1_a,
                call1_b,
                call1_c,
                call1_pc,
            ],
        );
        let helper_status = builder.inst_results(helper_call)[0];
        let helper_continue =
            builder
                .ins()
                .icmp_imm(IntCC::Equal, helper_status, i64::from(NATIVE_CALL_CONTINUE));
        builder
            .ins()
            .brif(helper_continue, copy_done_block, &[], fallback_block, &[]);

        builder.switch_to_block(copy_done_block);
        builder.seal_block(copy_done_block);
        for step in [
            NumericStep::Move { dst: 18, src: 9 },
            NumericStep::Move { dst: 19, src: 17 },
            NumericStep::LoadI { dst: 20, imm: 1 },
            NumericStep::Len { dst: 21, src: 17 },
        ] {
            emit_numeric_step(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_block,
                step,
                &mut known_value_kinds,
                &mut current_numeric_values,
                None,
                HoistedNumericGuardValues::default(),
            )?;
        }

        let call2_a = builder.ins().iconst(types::I32, 18);
        let call2_b = builder.ins().iconst(types::I32, 4);
        let call2_c = builder.ins().iconst(types::I32, 1);
        let call2_pc = builder.ins().iconst(types::I32, 41);
        let helper_call = builder.ins().call(
            native_helpers.call,
            &[
                abi.lua_state_ptr,
                abi.base_ptr,
                abi.base_slots,
                call2_a,
                call2_b,
                call2_c,
                call2_pc,
            ],
        );
        let helper_status = builder.inst_results(helper_call)[0];
        let helper_continue =
            builder
                .ins()
                .icmp_imm(IntCC::Equal, helper_status, i64::from(NATIVE_CALL_CONTINUE));
        builder
            .ins()
            .brif(helper_continue, sort_done_block, &[], fallback_block, &[]);

        builder.switch_to_block(sort_done_block);
        builder.seal_block(sort_done_block);
        for step in [
            NumericStep::Move { dst: 18, src: 11 },
            NumericStep::Move { dst: 19, src: 17 },
        ] {
            emit_numeric_step(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_block,
                step,
                &mut known_value_kinds,
                &mut current_numeric_values,
                None,
                HoistedNumericGuardValues::default(),
            )?;
        }

        let call3_a = builder.ins().iconst(types::I32, 18);
        let call3_b = builder.ins().iconst(types::I32, 2);
        let call3_c = builder.ins().iconst(types::I32, 2);
        let call3_pc = builder.ins().iconst(types::I32, 44);
        let helper_call = builder.ins().call(
            native_helpers.call,
            &[
                abi.lua_state_ptr,
                abi.base_ptr,
                abi.base_slots,
                call3_a,
                call3_b,
                call3_c,
                call3_pc,
            ],
        );
        let helper_status = builder.inst_results(helper_call)[0];
        let helper_continue =
            builder
                .ins()
                .icmp_imm(IntCC::Equal, helper_status, i64::from(NATIVE_CALL_CONTINUE));
        builder
            .ins()
            .brif(helper_continue, check_done_block, &[], fallback_block, &[]);

        builder.switch_to_block(check_done_block);
        builder.seal_block(check_done_block);
        let cond = emit_numeric_condition_value(
            &mut builder,
            &abi,
            hits_var,
            current_hits,
            fallback_block,
            NumericIfElseCond::Truthy { reg: 18 },
            &mut known_value_kinds,
            &mut current_numeric_values,
            None,
            HoistedNumericGuardValues::default(),
        )?;
        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        builder.def_var(hits_var, next_hits);
        builder
            .ins()
            .brif(cond, sorted_block, &[], unsorted_block, &[]);

        builder.switch_to_block(sorted_block);
        let sorted_hits = builder.use_var(hits_var);
        emit_store_native_result(
            &mut builder,
            abi.result_ptr,
            NativeTraceStatus::SideExit,
            sorted_hits,
            shape.sorted_resume_pc,
            shape.exit_index,
        );
        builder.ins().return_(&[]);
        builder.seal_block(sorted_block);

        builder.switch_to_block(unsorted_block);
        let unsorted_hits = builder.use_var(hits_var);
        emit_store_native_result(
            &mut builder,
            abi.result_ptr,
            NativeTraceStatus::LoopExit,
            unsorted_hits,
            shape.unsorted_resume_pc,
            0,
        );
        builder.ins().return_(&[]);
        builder.seal_block(unsorted_block);

        emit_native_terminal_result(
            &mut builder,
            fallback_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Fallback,
            None,
            None,
        );

        builder.seal_block(entry_block);
        builder.finalize();
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::GuardedCallPrefix {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }


    pub fn compile_native_guarded_numeric_for_loop(
        &mut self,
        loop_reg: u32,
        lowering: &NumericLowering,
        guard: NumericJmpLoopGuard,
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        let steps = &lowering.steps;
        if !steps.iter().all(native_supports_numeric_step) {
            return None;
        }

        let (cond, continue_when, continue_preset, exit_preset, side_exit_pc) = match guard {
            NumericJmpLoopGuard::Tail {
                cond,
                continue_when,
                continue_preset,
                exit_preset,
                exit_pc,
            } => (cond, continue_when, continue_preset, exit_preset, exit_pc),
            NumericJmpLoopGuard::Head { .. } => return None,
        };
        let side_exit_index = lowered_trace
            .deopt_target_for_exit_pc(side_exit_pc)?
            .exit_index;

        if !native_supports_numeric_cond(cond)
            || continue_preset
                .as_ref()
                .is_some_and(|step| !native_supports_numeric_step(step))
            || exit_preset
                .as_ref()
                .is_some_and(|step| !native_supports_numeric_step(step))
        {
            return None;
        }

        let func_name = self.allocate_function_name("jit_native_guarded_numeric_for_loop");
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let native_helpers = declare_native_helpers(
            &mut module,
            &mut context.func,
            pointer_ty,
            target_config.default_call_conv,
        )
        .ok()?;
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);
        let carried_remaining_var = builder.declare_var(types::I64);
        let carried_index_var = builder.declare_var(types::I64);
        let carried_integer_var = builder.declare_var(types::I64);
        let carried_float_raw_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();
        let loop_state_is_invariant = numeric_loop_state_is_invariant(loop_reg, steps)
            && !numeric_guard_touches_reg(guard, loop_reg)
            && !numeric_guard_touches_reg(guard, loop_reg.saturating_add(1))
            && !numeric_guard_touches_reg(guard, loop_reg.saturating_add(2));
        let carried_integer_flow = if loop_state_is_invariant {
            lowering
                .value_state
                .self_update
                .filter(|flow| matches!(flow.kind, NumericSelfUpdateValueKind::Integer))
        } else {
            None
        };
        let carried_integer_step = carried_integer_flow
            .and_then(carried_integer_loop_step_from_value_flow)
            .filter(|step| {
                !numeric_guard_writes_reg_outside_condition(guard, step.reg)
                    && carried_integer_rhs_stable_reg(*step).is_none_or(|reg| {
                        numeric_steps_preserve_reg(steps, reg)
                            && !numeric_guard_writes_reg_outside_condition(guard, reg)
                    })
            });
        let carried_integer_span =
            carried_integer_flow.and_then(|flow| integer_self_update_step_span(steps, flow));
        let carried_float_flow = if loop_state_is_invariant && carried_integer_step.is_none() {
            lowering
                .value_state
                .self_update
                .filter(|flow| matches!(flow.kind, NumericSelfUpdateValueKind::Float))
        } else {
            None
        };
        let carried_float_step = carried_float_flow
            .and_then(|flow| carried_float_loop_step_from_value_flow(flow, lowered_trace))
            .or_else(|| exact_float_self_update_step(steps, lowered_trace))
            .filter(|step| {
                !numeric_guard_writes_reg_outside_condition(guard, step.reg)
                    && carried_float_rhs_stable_reg(*step).is_none_or(|reg| {
                        numeric_steps_preserve_reg(steps, reg)
                            && !numeric_guard_writes_reg_outside_condition(guard, reg)
                    })
            });
        let carried_float_span =
            carried_float_flow.and_then(|flow| float_self_update_step_span(steps, flow));

        let loop_block = builder.create_block();
        if loop_state_is_invariant {
            builder.append_block_param(loop_block, types::I64);
            builder.append_block_param(loop_block, types::I64);
        }
        if carried_integer_step.is_some() {
            builder.append_block_param(loop_block, types::I64);
        }
        if carried_float_step.is_some() {
            builder.append_block_param(loop_block, types::I64);
        }
        let fallback_terminal_block = builder.create_block();
        let loop_exit_terminal_block = builder.create_block();
        let side_exit_terminal_block = builder.create_block();
        let fallback_block = if loop_state_is_invariant
            || carried_integer_step.is_some()
            || carried_float_step.is_some()
        {
            builder.create_block()
        } else {
            fallback_terminal_block
        };
        let loop_exit_block = if loop_state_is_invariant
            || carried_integer_step.is_some()
            || carried_float_step.is_some()
        {
            builder.create_block()
        } else {
            loop_exit_terminal_block
        };
        let side_exit_block = if loop_state_is_invariant
            || carried_integer_step.is_some()
            || carried_float_step.is_some()
        {
            builder.create_block()
        } else {
            side_exit_terminal_block
        };

        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        let hoisted_step_value = if loop_state_is_invariant {
            let loop_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg);
            let step_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(1));
            let index_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(2));
            emit_integer_guard(&mut builder, loop_ptr, hits_var, zero_hits, fallback_block);
            emit_integer_guard(&mut builder, step_ptr, hits_var, zero_hits, fallback_block);
            emit_integer_guard(&mut builder, index_ptr, hits_var, zero_hits, fallback_block);
            set_numeric_reg_value_kind(&mut known_value_kinds, loop_reg, TraceValueKind::Integer);
            set_numeric_reg_value_kind(
                &mut known_value_kinds,
                loop_reg.saturating_add(1),
                TraceValueKind::Integer,
            );
            set_numeric_reg_value_kind(
                &mut known_value_kinds,
                loop_reg.saturating_add(2),
                TraceValueKind::Integer,
            );
            Some(builder.ins().load(
                types::I64,
                MemFlags::new(),
                step_ptr,
                LUA_VALUE_VALUE_OFFSET,
            ))
        } else {
            None
        };
        let initial_remaining = if loop_state_is_invariant {
            let loop_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg);
            builder.ins().load(
                types::I64,
                MemFlags::new(),
                loop_ptr,
                LUA_VALUE_VALUE_OFFSET,
            )
        } else {
            builder.ins().iconst(types::I64, 0)
        };
        let initial_index = if loop_state_is_invariant {
            let index_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(2));
            builder.ins().load(
                types::I64,
                MemFlags::new(),
                index_ptr,
                LUA_VALUE_VALUE_OFFSET,
            )
        } else {
            builder.ins().iconst(types::I64, 0)
        };
        let initial_integer_value = if let Some(step) = carried_integer_step {
            let slot_ptr = slot_addr(&mut builder, abi.base_ptr, step.reg);
            emit_exact_tag_guard(
                &mut builder,
                slot_ptr,
                LUA_VNUMINT,
                hits_var,
                zero_hits,
                fallback_block,
            );
            set_numeric_reg_value_kind(&mut known_value_kinds, step.reg, TraceValueKind::Integer);
            Some(builder.ins().load(
                types::I64,
                MemFlags::new(),
                slot_ptr,
                LUA_VALUE_VALUE_OFFSET,
            ))
        } else {
            None
        };
        let initial_float_raw = if let Some(step) = carried_float_step {
            let slot_ptr = slot_addr(&mut builder, abi.base_ptr, step.reg);
            emit_exact_tag_guard(
                &mut builder,
                slot_ptr,
                LUA_VNUMFLT,
                hits_var,
                zero_hits,
                fallback_block,
            );
            set_numeric_reg_value_kind(&mut known_value_kinds, step.reg, TraceValueKind::Float);
            Some(builder.ins().load(
                types::I64,
                MemFlags::new(),
                slot_ptr,
                LUA_VALUE_VALUE_OFFSET,
            ))
        } else {
            None
        };
        let carried_integer_rhs = carried_integer_step.map(|step| {
            resolve_carried_integer_rhs(
                &mut builder,
                abi.base_ptr,
                hits_var,
                zero_hits,
                fallback_block,
                step,
            )
        });
        let carried_float_rhs = carried_float_step.map(|step| {
            resolve_carried_float_rhs(
                &mut builder,
                abi.base_ptr,
                hits_var,
                zero_hits,
                fallback_block,
                step,
            )
        });
        let hoisted_guard_rhs = carried_float_step
            .zip(carried_float_rhs)
            .and_then(|(step, rhs)| hoisted_numeric_guard_value_from_carried_rhs(step, rhs));
        let hoisted_integer_rhs =
            carried_integer_step
                .zip(carried_integer_rhs)
                .and_then(|(step, rhs)| {
                    hoisted_numeric_guard_value_from_carried_integer_rhs(step, rhs)
                });

        let mut initial_args = Vec::new();
        if loop_state_is_invariant {
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(initial_remaining));
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(initial_index));
        }
        if let Some(value) = initial_integer_value {
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(value));
        }
        if let Some(raw) = initial_float_raw {
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(raw));
        }
        builder.ins().jump(loop_block, &initial_args);

        builder.switch_to_block(loop_block);
        let current_hits = builder.use_var(hits_var);
        if loop_state_is_invariant {
            let carried_remaining = builder.block_params(loop_block)[0];
            let carried_index = builder.block_params(loop_block)[1];
            builder.def_var(carried_remaining_var, carried_remaining);
            builder.def_var(carried_index_var, carried_index);
        }
        if carried_integer_step.is_some() {
            let integer_param_index = if loop_state_is_invariant { 2 } else { 0 };
            let carried_integer = builder.block_params(loop_block)[integer_param_index];
            builder.def_var(carried_integer_var, carried_integer);
        }
        if carried_float_step.is_some() {
            let float_param_index = if loop_state_is_invariant {
                2 + usize::from(carried_integer_step.is_some())
            } else {
                usize::from(carried_integer_step.is_some())
            };
            let carried_float_raw = builder.block_params(loop_block)[float_param_index];
            builder.def_var(carried_float_raw_var, carried_float_raw);
        }

        let mut current_numeric_values = Vec::new();

        if let Some(step) = carried_integer_step {
            let (span_start, span_len) = carried_integer_span.expect(
                "guarded numeric carried-integer path requires a matching self-update span",
            );
            emit_numeric_steps_with_carried_integer(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_block,
                steps,
                carried_integer_var,
                step,
                carried_integer_rhs
                    .expect("guarded numeric carried-integer path requires resolved rhs"),
                span_start,
                span_len,
                hoisted_integer_rhs,
                &mut known_value_kinds,
                &mut current_numeric_values,
            )?;
        } else if let Some(step) = carried_float_step {
            if let Some((span_start, span_len)) = carried_float_span {
                emit_numeric_steps_with_carried_float(
                    &mut builder,
                    &abi,
                    &native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    steps,
                    carried_float_raw_var,
                    step,
                    carried_float_rhs
                        .expect("guarded numeric carried-float path requires resolved rhs"),
                    span_start,
                    span_len,
                    hoisted_guard_rhs,
                    &mut known_value_kinds,
                    &mut current_numeric_values,
                )?;
            } else {
                emit_carried_float_loop_step(
                    &mut builder,
                    carried_float_raw_var,
                    step,
                    carried_float_rhs
                        .expect("guarded numeric carried-float path requires resolved rhs"),
                    &mut known_value_kinds,
                );
            }
        } else {
            for step in steps {
                emit_numeric_step(
                    &mut builder,
                    &abi,
                    &native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    *step,
                    &mut known_value_kinds,
                    &mut current_numeric_values,
                    None,
                    HoistedNumericGuardValues::default(),
                )?;
            }
        }

        let continue_block = builder.create_block();
        let carried_integer_guard_value =
            carried_integer_step.map(|step| HoistedNumericGuardValue {
                reg: step.reg,
                source: HoistedNumericGuardSource::Integer(builder.use_var(carried_integer_var)),
            });
        let hoisted_numeric = HoistedNumericGuardValues {
            first: carried_integer_guard_value.or(hoisted_guard_rhs),
            second: hoisted_integer_rhs,
        };
        emit_numeric_guard_flow(
            &mut builder,
            &abi,
            &native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            cond,
            continue_when,
            continue_preset.as_ref(),
            exit_preset.as_ref(),
            continue_block,
            side_exit_block,
            &mut known_value_kinds,
            &mut current_numeric_values,
            carried_float_step.map(|step| CarriedFloatGuardValue {
                reg: step.reg,
                raw_var: carried_float_raw_var,
            }),
            hoisted_numeric,
        )?;

        builder.switch_to_block(continue_block);
        builder.seal_block(continue_block);
        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        if loop_state_is_invariant {
            let carried_remaining = builder.use_var(carried_remaining_var);
            let carried_index = builder.use_var(carried_index_var);
            if carried_integer_step.is_some() {
                let carried_integer = builder.use_var(carried_integer_var);
                emit_numeric_counted_loop_backedge_with_carried_integer(
                    &mut builder,
                    hits_var,
                    next_hits,
                    carried_remaining,
                    carried_index,
                    hoisted_step_value,
                    carried_integer,
                    loop_block,
                    loop_exit_block,
                );
            } else if carried_float_step.is_some() {
                let carried_float_raw = builder.use_var(carried_float_raw_var);
                emit_numeric_counted_loop_backedge_with_carried_float(
                    &mut builder,
                    hits_var,
                    next_hits,
                    carried_remaining,
                    carried_index,
                    hoisted_step_value,
                    carried_float_raw,
                    loop_block,
                    loop_exit_block,
                );
            } else {
                emit_linear_int_counted_loop_backedge(
                    &mut builder,
                    hits_var,
                    next_hits,
                    carried_remaining,
                    carried_index,
                    hoisted_step_value,
                    loop_block,
                    loop_exit_block,
                );
            }
        } else {
            emit_counted_loop_backedge(
                &mut builder,
                abi.base_ptr,
                hits_var,
                current_hits,
                next_hits,
                loop_reg,
                None,
                false,
                loop_block,
                loop_exit_terminal_block,
                fallback_block,
            );
        }

        if loop_state_is_invariant || carried_integer_step.is_some() || carried_float_step.is_some()
        {
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_state_is_invariant.then_some((
                    loop_reg,
                    carried_remaining_var,
                    carried_index_var,
                )),
                carried_integer_step.map(|step| (step.reg, carried_integer_var)),
                carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                fallback_block,
                fallback_terminal_block,
            );
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_state_is_invariant.then_some((
                    loop_reg,
                    carried_remaining_var,
                    carried_index_var,
                )),
                carried_integer_step.map(|step| (step.reg, carried_integer_var)),
                carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                loop_exit_block,
                loop_exit_terminal_block,
            );
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_state_is_invariant.then_some((
                    loop_reg,
                    carried_remaining_var,
                    carried_index_var,
                )),
                carried_integer_step.map(|step| (step.reg, carried_integer_var)),
                carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                side_exit_block,
                side_exit_terminal_block,
            );
        }

        emit_native_terminal_result(
            &mut builder,
            side_exit_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::SideExit,
            Some(side_exit_pc),
            Some(side_exit_index),
        );
        emit_native_terminal_result(
            &mut builder,
            loop_exit_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::LoopExit,
            None,
            None,
        );
        emit_native_terminal_result(
            &mut builder,
            fallback_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Fallback,
            None,
            None,
        );

        builder.seal_block(loop_block);
        builder.finalize();
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::GuardedNumericForLoop {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    pub fn compile_native_numeric_jmp_loop(
        &mut self,
        head_blocks: &[NumericJmpLoopGuardBlock],
        lowering: &NumericLowering,
        tail_blocks: &[NumericJmpLoopGuardBlock],
        interior_guard_start: usize,
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        let steps = &lowering.steps;
        if head_blocks.is_empty() && tail_blocks.is_empty() {
            return None;
        }

        if !steps.iter().all(native_supports_numeric_step) {
            return None;
        }

        for (head_index, block) in head_blocks.iter().enumerate() {
            let is_interior = head_index >= interior_guard_start;
            if !numeric_jmp_guard_block_is_supported(block, false, is_interior, lowered_trace) {
                return None;
            }
        }
        for block in tail_blocks {
            if !numeric_jmp_guard_block_is_supported(block, true, false, lowered_trace) {
                return None;
            }
        }

        let func_name = self.allocate_function_name("jit_native_numeric_jmp_loop");
        let mut module = Self::build_module().ok()?;
        let target_config = module.target_config();
        let pointer_ty = target_config.pointer_type();
        let mut context = make_native_context(target_config);
        let native_helpers = declare_native_helpers(
            &mut module,
            &mut context.func,
            pointer_ty,
            target_config.default_call_conv,
        )
        .ok()?;
        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);
        let carried_integer_var = builder.declare_var(types::I64);
        let carried_float_raw_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();
        let carried_integer_flow = lowering
            .value_state
            .self_update
            .filter(|flow| matches!(flow.kind, NumericSelfUpdateValueKind::Integer));
        let carried_integer_step = carried_integer_flow
            .and_then(carried_integer_loop_step_from_value_flow)
            .filter(|step| {
                !head_blocks
                    .iter()
                    .chain(tail_blocks.iter())
                    .any(|block| numeric_guard_block_touches_reg(block, step.reg))
                    && carried_integer_rhs_stable_reg(*step).is_none_or(|reg| {
                        numeric_steps_preserve_reg(steps, reg)
                            && !head_blocks.iter().chain(tail_blocks.iter()).any(|block| {
                                numeric_guard_block_writes_reg_outside_condition(block, reg)
                            })
                    })
            });
        let carried_integer_span =
            carried_integer_flow.and_then(|flow| integer_self_update_step_span(steps, flow));
        let carried_float_flow = carried_integer_step.is_none().then_some(()).and_then(|_| {
            lowering
                .value_state
                .self_update
                .filter(|flow| matches!(flow.kind, NumericSelfUpdateValueKind::Float))
        });
        let carried_float_step =
            carried_float_flow
                .and_then(|flow| carried_float_loop_step_from_value_flow(flow, lowered_trace))
                .or_else(|| exact_float_self_update_step(steps, lowered_trace))
                .filter(|step| {
                    !head_blocks.iter().chain(tail_blocks.iter()).any(|block| {
                        numeric_guard_block_writes_reg_outside_condition(block, step.reg)
                    }) && (!head_blocks
                        .iter()
                        .chain(tail_blocks.iter())
                        .any(|block| numeric_guard_block_touches_reg(block, step.reg))
                        || entry_reg_has_explicit_float_hint(lowered_trace, step.reg))
                        && carried_float_rhs_stable_reg(*step).is_none_or(|reg| {
                            numeric_steps_preserve_reg(steps, reg)
                                && !head_blocks.iter().chain(tail_blocks.iter()).any(|block| {
                                    numeric_guard_block_writes_reg_outside_condition(block, reg)
                                })
                        })
                });
        let carried_float_span =
            carried_float_flow.and_then(|flow| float_self_update_step_span(steps, flow));

        let loop_block = builder.create_block();
        if carried_integer_step.is_some() {
            builder.append_block_param(loop_block, types::I64);
        }
        if carried_float_step.is_some() {
            builder.append_block_param(loop_block, types::I64);
        }
        let fallback_terminal_block = builder.create_block();
        let fallback_block = if carried_integer_step.is_some() || carried_float_step.is_some() {
            builder.create_block()
        } else {
            fallback_terminal_block
        };

        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        let initial_integer_value = if let Some(step) = carried_integer_step {
            let slot_ptr = slot_addr(&mut builder, abi.base_ptr, step.reg);
            emit_exact_tag_guard(
                &mut builder,
                slot_ptr,
                LUA_VNUMINT,
                hits_var,
                zero_hits,
                fallback_block,
            );
            set_numeric_reg_value_kind(&mut known_value_kinds, step.reg, TraceValueKind::Integer);
            Some(builder.ins().load(
                types::I64,
                MemFlags::new(),
                slot_ptr,
                LUA_VALUE_VALUE_OFFSET,
            ))
        } else {
            None
        };
        let initial_float_raw = if let Some(step) = carried_float_step {
            let slot_ptr = slot_addr(&mut builder, abi.base_ptr, step.reg);
            emit_exact_tag_guard(
                &mut builder,
                slot_ptr,
                LUA_VNUMFLT,
                hits_var,
                zero_hits,
                fallback_block,
            );
            set_numeric_reg_value_kind(&mut known_value_kinds, step.reg, TraceValueKind::Float);
            Some(builder.ins().load(
                types::I64,
                MemFlags::new(),
                slot_ptr,
                LUA_VALUE_VALUE_OFFSET,
            ))
        } else {
            None
        };
        let carried_integer_rhs = carried_integer_step.map(|step| {
            resolve_carried_integer_rhs(
                &mut builder,
                abi.base_ptr,
                hits_var,
                zero_hits,
                fallback_block,
                step,
            )
        });
        let carried_float_rhs = carried_float_step.map(|step| {
            resolve_carried_float_rhs(
                &mut builder,
                abi.base_ptr,
                hits_var,
                zero_hits,
                fallback_block,
                step,
            )
        });
        let hoisted_guard_rhs = carried_float_step
            .zip(carried_float_rhs)
            .and_then(|(step, rhs)| hoisted_numeric_guard_value_from_carried_rhs(step, rhs));
        let hoisted_integer_rhs =
            carried_integer_step
                .zip(carried_integer_rhs)
                .and_then(|(step, rhs)| {
                    hoisted_numeric_guard_value_from_carried_integer_rhs(step, rhs)
                });
        let mut initial_args = Vec::new();
        if let Some(value) = initial_integer_value {
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(value));
        }
        if let Some(raw) = initial_float_raw {
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(raw));
        }
        builder.ins().jump(loop_block, &initial_args);

        builder.switch_to_block(loop_block);
        let current_hits = builder.use_var(hits_var);
        if carried_integer_step.is_some() {
            let carried_integer = builder.block_params(loop_block)[0];
            builder.def_var(carried_integer_var, carried_integer);
        }
        if carried_float_step.is_some() {
            let float_param_index = usize::from(carried_integer_step.is_some());
            let carried_float_raw = builder.block_params(loop_block)[float_param_index];
            builder.def_var(carried_float_raw_var, carried_float_raw);
        }

        let carried_integer_guard_value =
            carried_integer_step.map(|step| HoistedNumericGuardValue {
                reg: step.reg,
                source: HoistedNumericGuardSource::Integer(builder.use_var(carried_integer_var)),
            });
        let hoisted_numeric = HoistedNumericGuardValues {
            first: carried_integer_guard_value.or(hoisted_guard_rhs),
            second: hoisted_integer_rhs,
        };

        let mut side_exit_sites = Vec::with_capacity(head_blocks.len() + tail_blocks.len());
        let mut current_numeric_values = Vec::new();

        for (head_index, block) in head_blocks.iter().enumerate() {
            let is_interior = head_index >= interior_guard_start;
            let continue_block = builder.create_block();
            let side_exit_terminal_block = builder.create_block();
            let side_exit_block = if carried_integer_step.is_some() || carried_float_step.is_some()
            {
                builder.create_block()
            } else {
                side_exit_terminal_block
            };
            emit_numeric_guard_block(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_block,
                block,
                continue_block,
                side_exit_block,
                &mut known_value_kinds,
                &mut current_numeric_values,
                carried_float_step.map(|step| CarriedFloatGuardValue {
                    reg: step.reg,
                    raw_var: carried_float_raw_var,
                }),
                hoisted_numeric,
            )?;
            side_exit_sites.push((
                side_exit_block,
                side_exit_terminal_block,
                numeric_jmp_guard_exit_pc(block.guard),
                lowered_trace
                    .deopt_target_for_exit_pc(numeric_jmp_guard_exit_pc(block.guard))?
                    .exit_index,
                is_interior,
            ));
            builder.switch_to_block(continue_block);
            builder.seal_block(continue_block);
        }

        if let Some(step) = carried_integer_step {
            let (span_start, span_len) = carried_integer_span
                .expect("numeric jmp carried-integer path requires a matching self-update span");
            emit_numeric_steps_with_carried_integer(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                current_hits,
                fallback_block,
                steps,
                carried_integer_var,
                step,
                carried_integer_rhs
                    .expect("numeric jmp carried-integer path requires resolved rhs"),
                span_start,
                span_len,
                hoisted_integer_rhs,
                &mut known_value_kinds,
                &mut current_numeric_values,
            )?;
        } else if let Some(step) = carried_float_step {
            if let Some((span_start, span_len)) = carried_float_span {
                emit_numeric_steps_with_carried_float(
                    &mut builder,
                    &abi,
                    &native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    steps,
                    carried_float_raw_var,
                    step,
                    carried_float_rhs
                        .expect("numeric jmp carried-float path requires resolved rhs"),
                    span_start,
                    span_len,
                    hoisted_guard_rhs,
                    &mut known_value_kinds,
                    &mut current_numeric_values,
                )?;
            } else {
                emit_carried_float_loop_step(
                    &mut builder,
                    carried_float_raw_var,
                    step,
                    carried_float_rhs
                        .expect("numeric jmp carried-float path requires resolved rhs"),
                    &mut known_value_kinds,
                );
            }
        } else {
            for step in steps {
                emit_numeric_step(
                    &mut builder,
                    &abi,
                    &native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    *step,
                    &mut known_value_kinds,
                    &mut current_numeric_values,
                    None,
                    HoistedNumericGuardValues::default(),
                )?;
            }
        }

        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        builder.def_var(hits_var, next_hits);

        for block in tail_blocks {
            let continue_block = builder.create_block();
            let side_exit_terminal_block = builder.create_block();
            let side_exit_block = if carried_integer_step.is_some() || carried_float_step.is_some()
            {
                builder.create_block()
            } else {
                side_exit_terminal_block
            };
            emit_numeric_guard_block(
                &mut builder,
                &abi,
                &native_helpers,
                hits_var,
                next_hits,
                fallback_block,
                block,
                continue_block,
                side_exit_block,
                &mut known_value_kinds,
                &mut current_numeric_values,
                carried_float_step.map(|step| CarriedFloatGuardValue {
                    reg: step.reg,
                    raw_var: carried_float_raw_var,
                }),
                hoisted_numeric,
            )?;
            side_exit_sites.push((
                side_exit_block,
                side_exit_terminal_block,
                numeric_jmp_guard_exit_pc(block.guard),
                lowered_trace
                    .deopt_target_for_exit_pc(numeric_jmp_guard_exit_pc(block.guard))?
                    .exit_index,
                false,
            ));
            builder.switch_to_block(continue_block);
            builder.seal_block(continue_block);
        }

        if carried_integer_step.is_some() {
            let carried_integer = builder.use_var(carried_integer_var);
            if carried_float_step.is_some() {
                let carried_float_raw = builder.use_var(carried_float_raw_var);
                builder.ins().jump(
                    loop_block,
                    &[
                        cranelift::codegen::ir::BlockArg::Value(carried_integer),
                        cranelift::codegen::ir::BlockArg::Value(carried_float_raw),
                    ],
                );
            } else {
                builder.ins().jump(
                    loop_block,
                    &[cranelift::codegen::ir::BlockArg::Value(carried_integer)],
                );
            }
        } else if carried_float_step.is_some() {
            let carried_float_raw = builder.use_var(carried_float_raw_var);
            builder.ins().jump(
                loop_block,
                &[cranelift::codegen::ir::BlockArg::Value(carried_float_raw)],
            );
        } else {
            builder.ins().jump(loop_block, &[]);
        }

        if carried_integer_step.is_some() || carried_float_step.is_some() {
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                None,
                carried_integer_step.map(|step| (step.reg, carried_integer_var)),
                carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                fallback_block,
                fallback_terminal_block,
            );
        }

        for (side_exit_block, side_exit_terminal_block, exit_pc, exit_index, is_interior) in
            side_exit_sites
        {
            if carried_integer_step.is_some() || carried_float_step.is_some() {
                emit_materialize_numeric_loop_state(
                    &mut builder,
                    abi.base_ptr,
                    None,
                    carried_integer_step.map(|step| (step.reg, carried_integer_var)),
                    carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                    side_exit_block,
                    side_exit_terminal_block,
                );
            }
            if is_interior {
                emit_native_terminal_result(
                    &mut builder,
                    side_exit_terminal_block,
                    abi.result_ptr,
                    hits_var,
                    NativeTraceStatus::LoopExit,
                    Some(exit_pc),
                    None,
                );
            } else {
                emit_native_terminal_result(
                    &mut builder,
                    side_exit_terminal_block,
                    abi.result_ptr,
                    hits_var,
                    NativeTraceStatus::SideExit,
                    Some(exit_pc),
                    Some(exit_index),
                );
            }
        }
        emit_native_terminal_result(
            &mut builder,
            fallback_terminal_block,
            abi.result_ptr,
            hits_var,
            NativeTraceStatus::Fallback,
            None,
            None,
        );

        builder.seal_block(loop_block);
        builder.finalize();
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::NumericJmpLoop {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    fn allocate_function_name(&mut self, prefix: &str) -> String {
        let func_name = format!("{}_{}", prefix, self.next_function_index);
        self.next_function_index = self.next_function_index.saturating_add(1);
        func_name
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CarriedFloatLoopStep {
    pub(crate) reg: u32,
    pub(crate) op: NumericBinaryOp,
    pub(crate) rhs: CarriedFloatRhs,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CarriedIntegerLoopStep {
    pub(crate) reg: u32,
    pub(crate) op: NumericBinaryOp,
    pub(crate) rhs: CarriedIntegerRhs,
}

#[derive(Clone, Copy)]
pub(crate) struct CarriedFloatGuardValue {
    pub(crate) reg: u32,
    pub(crate) raw_var: Variable,
}

#[derive(Clone, Copy)]
pub(crate) struct HoistedNumericGuardValue {
    pub(crate) reg: u32,
    pub(crate) source: HoistedNumericGuardSource,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct HoistedNumericGuardValues {
    pub(crate) first: Option<HoistedNumericGuardValue>,
    pub(crate) second: Option<HoistedNumericGuardValue>,
}

pub(crate) type CurrentNumericGuardValues = Vec<(u32, HoistedNumericGuardSource)>;

#[derive(Clone, Copy)]
pub(crate) enum HoistedNumericGuardSource {
    FloatRaw(Value),
    Integer(Value),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CarriedFloatRhs {
    Imm(f64),
    StableReg { reg: u32, kind: TraceValueKind },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CarriedIntegerRhs {
    Imm(i64),
    StableReg { reg: u32 },
}

#[derive(Clone, Copy)]
pub(crate) enum ResolvedCarriedFloatRhs {
    Imm(f64),
    FloatRaw(Value),
    Integer(Value),
}

#[derive(Clone, Copy)]
pub(crate) enum ResolvedCarriedIntegerRhs {
    Imm(Value),
    Integer(Value),
}

#[cfg(test)]
impl NativeTraceBackend {
    pub(crate) fn compile_test(
        &mut self,
        ir: &TraceIr,
        helper_plan: &HelperPlan,
    ) -> BackendCompileOutcome {
        let artifact = crate::lua_vm::jit::backend::synthetic_artifact_for_ir(ir);
        let lowered_trace = LoweredTrace::lower(&artifact, ir, helper_plan);
        <Self as TraceBackend>::compile(self, &artifact, ir, &lowered_trace, helper_plan)
    }

    pub(crate) fn compile_test_with_constants(
        &mut self,
        ir: &TraceIr,
        helper_plan: &HelperPlan,
        constants: Vec<LuaValue>,
    ) -> BackendCompileOutcome {
        let artifact = crate::lua_vm::jit::backend::synthetic_artifact_for_ir(ir);
        let mut lowered_trace = LoweredTrace::lower(&artifact, ir, helper_plan);
        lowered_trace.constants = constants;
        <Self as TraceBackend>::compile(self, &artifact, ir, &lowered_trace, helper_plan)
    }

    pub(crate) fn compile_test_numeric_jmp_blocks(
        &mut self,
        head_blocks: &[NumericJmpLoopGuardBlock],
        steps: &[NumericStep],
        tail_blocks: &[NumericJmpLoopGuardBlock],
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        use crate::lua_vm::jit::backend::NumericValueState;

        self.compile_native_numeric_jmp_loop(
            head_blocks,
            &NumericLowering {
                steps: steps.to_vec(),
                value_state: NumericValueState::default(),
            },
            tail_blocks,
            head_blocks.len(),
            lowered_trace,
        )
    }

    pub(crate) fn compile_test_with_artifact(
        &mut self,
        artifact: &TraceArtifact,
        ir: &TraceIr,
        helper_plan: &HelperPlan,
    ) -> BackendCompileOutcome {
        let lowered_trace = LoweredTrace::lower(artifact, ir, helper_plan);
        <Self as TraceBackend>::compile(self, artifact, ir, &lowered_trace, helper_plan)
    }
}

