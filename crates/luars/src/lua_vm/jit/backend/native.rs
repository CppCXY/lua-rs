use cranelift::codegen::isa::{CallConv, TargetFrontendConfig};
use cranelift_codegen::ir::FuncRef;
use cranelift::codegen::settings;
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module};

use crate::gc::UpvaluePtr;
use crate::lua_value::{LUA_VNUMFLT, LUA_VNUMINT};
use crate::lua_vm::execute::helper::{
    lua_fmod, lua_idiv, lua_imod, luai_numpow, pivalue, pttisinteger, ttisfloat, ttisinteger,
};
use crate::{Instruction, LuaState, LuaValue};
use crate::lua_vm::jit::helper_plan::HelperPlan;
use crate::lua_vm::jit::ir::TraceIr;
use crate::lua_vm::jit::lowering::{LoweredTrace, TraceValueKind};
use crate::lua_vm::jit::trace_recorder::TraceArtifact;

use super::compile::{
    lower_linear_int_guard_for_native, lower_linear_int_steps_for_native,
    lower_numeric_guard_for_native, lower_numeric_steps_for_native,
};
use super::{
    BackendCompileOutcome, CompiledTrace, CompiledTraceExecution,
    LinearIntGuardOp, LinearIntLoopGuard, LinearIntStep, NativeCompiledTrace,
    NativeLoweringProfile, NativeTraceResult, NativeTraceStatus, NumericBinaryOp, NumericIfElseCond,
    NumericJmpLoopGuard, NumericJmpLoopGuardBlock, NumericOperand, NumericStep, TraceBackend,
};

const LUA_VALUE_SIZE: i64 = std::mem::size_of::<LuaValue>() as i64;
const LUA_VALUE_TT_OFFSET: i32 = std::mem::offset_of!(LuaValue, tt) as i32;
const LUA_VALUE_VALUE_OFFSET: i32 = std::mem::offset_of!(LuaValue, value) as i32;
const LUA_VNIL_TAG: u8 = 0;
const LUA_VFALSE_TAG: u8 = 1;
const LUA_VTRUE_TAG: u8 = 17;
const NATIVE_HELPER_NUMERIC_GET_UPVAL_SYMBOL: &str = "jit_native_helper_numeric_get_upval";
const NATIVE_HELPER_NUMERIC_SET_UPVAL_SYMBOL: &str = "jit_native_helper_numeric_set_upval";
const NATIVE_HELPER_NUMERIC_GET_TABLE_INT_SYMBOL: &str = "jit_native_helper_numeric_get_table_int";
const NATIVE_HELPER_NUMERIC_SET_TABLE_INT_SYMBOL: &str = "jit_native_helper_numeric_set_table_int";
const NATIVE_HELPER_NUMERIC_BINARY_SYMBOL: &str = "jit_native_helper_numeric_binary";
const NATIVE_HELPER_NUMERIC_POW_SYMBOL: &str = "jit_native_helper_numeric_pow";
const NATIVE_HELPER_SHIFT_LEFT_SYMBOL: &str = "jit_native_helper_shift_left";
const NATIVE_HELPER_SHIFT_RIGHT_SYMBOL: &str = "jit_native_helper_shift_right";
const NATIVE_TRACE_RESULT_STATUS_OFFSET: i32 = std::mem::offset_of!(NativeTraceResult, status) as i32;
const NATIVE_TRACE_RESULT_HITS_OFFSET: i32 = std::mem::offset_of!(NativeTraceResult, hits) as i32;
const NATIVE_TRACE_RESULT_EXIT_PC_OFFSET: i32 = std::mem::offset_of!(NativeTraceResult, exit_pc) as i32;
const NATIVE_TRACE_RESULT_START_REG_OFFSET: i32 = std::mem::offset_of!(NativeTraceResult, start_reg) as i32;
const NATIVE_TRACE_RESULT_RESULT_COUNT_OFFSET: i32 = std::mem::offset_of!(NativeTraceResult, result_count) as i32;
const NATIVE_TRACE_RESULT_EXIT_INDEX_OFFSET: i32 = std::mem::offset_of!(NativeTraceResult, exit_index) as i32;
const NATIVE_NUMERIC_OPERAND_REG: i32 = 0;
const NATIVE_NUMERIC_OPERAND_IMM_I: i32 = 1;
const NATIVE_NUMERIC_OPERAND_CONST: i32 = 2;
const NATIVE_NUMERIC_BINARY_ADD: i32 = 0;
const NATIVE_NUMERIC_BINARY_SUB: i32 = 1;
const NATIVE_NUMERIC_BINARY_MUL: i32 = 2;
const NATIVE_NUMERIC_BINARY_DIV: i32 = 3;
const NATIVE_NUMERIC_BINARY_IDIV: i32 = 4;
const NATIVE_NUMERIC_BINARY_MOD: i32 = 5;
const NATIVE_NUMERIC_BINARY_POW: i32 = 6;

pub(crate) struct NativeTraceBackend {
    modules: Vec<JITModule>,
    next_function_index: u64,
}

impl Default for NativeTraceBackend {
    fn default() -> Self {
        Self {
            modules: Vec::new(),
            next_function_index: 0,
        }
    }
}

impl TraceBackend for NativeTraceBackend {
    fn compile(
        &mut self,
        artifact: &TraceArtifact,
        ir: &TraceIr,
        lowered_trace: &LoweredTrace,
        helper_plan: &HelperPlan,
    ) -> BackendCompileOutcome {
        if let Some((execution, native_profile)) =
            self.compile_native_generic_trace(ir, lowered_trace)
        {
            return match CompiledTrace::from_artifact_helper_plan_with_execution(
                artifact,
                ir,
                lowered_trace,
                helper_plan,
                execution,
                native_profile,
            ) {
                Some(compiled_trace) => BackendCompileOutcome::Compiled(compiled_trace),
                None => BackendCompileOutcome::NotYetSupported,
            };
        }

        match CompiledTrace::from_artifact_helper_plan_with_execution(
            artifact,
            ir,
            lowered_trace,
            helper_plan,
            CompiledTraceExecution::LoweredOnly,
            Some(NativeLoweringProfile::default()),
        ) {
            Some(compiled_trace) => BackendCompileOutcome::Compiled(compiled_trace),
            None => BackendCompileOutcome::NotYetSupported,
        }
    }
}

impl NativeTraceBackend {
    fn compile_native_generic_trace(
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
        let steps = lower_linear_int_steps_for_native(&ir.insts[..ir.insts.len() - 1], lowered_trace)?;
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
        let loop_backedge = ir.insts.last()?;
        if loop_backedge.opcode != crate::OpCode::ForLoop {
            return None;
        }

        let loop_reg = Instruction::from_u32(loop_backedge.raw_instruction).get_a();

        if ir.guards.is_empty() {
            let steps = lower_numeric_steps_for_native(&ir.insts[..ir.insts.len() - 1], lowered_trace)?;
            let native = self.compile_native_numeric_for_loop(loop_reg, &steps, lowered_trace)?;
            return Some((
                CompiledTraceExecution::Native(native),
                Some(profile_for_numeric_for_loop(&steps)),
            ));
        }

        if ir.guards.len() != 1 || ir.insts.len() < 4 {
            return None;
        }

        let guard = ir.guards[0];
        let lowered_exit = lowered_trace.deopt_target_for_exit_pc(guard.exit_pc)?;
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

        let steps = lower_numeric_steps_for_native(&ir.insts[..guard_index], lowered_trace)?;
        let loop_guard = lower_numeric_guard_for_native(&ir.insts[guard_index], true, lowered_exit.resume_pc)?;
        let native = self.compile_native_guarded_numeric_for_loop(loop_reg, &steps, loop_guard, lowered_trace)?;
        let profile = profile_for_guarded_numeric_for_loop(&steps, loop_guard);
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
            let loop_guard = lower_linear_int_guard_for_native(guard_inst, true, lowered_exit.resume_pc)?;
            let steps =
                lower_linear_int_steps_for_native(&ir.insts[..ir.insts.len() - 2], lowered_trace)?;
            (steps, loop_guard)
        } else {
            if ir.insts.len() < 4 || ir.insts[1].opcode != crate::OpCode::Jmp {
                return None;
            }

            let loop_guard = lower_linear_int_guard_for_native(
                &ir.insts[0],
                false,
                lowered_exit.resume_pc,
            )?;
            let steps = lower_linear_int_steps_for_native(&ir.insts[2..ir.insts.len() - 1], lowered_trace)?;
            (steps, loop_guard)
        };

        let native = self.compile_native_linear_int_jmp_loop(&steps, loop_guard, lowered_trace)?;
        let profile = profile_for_linear_int_jmp_loop(&steps, loop_guard);
        Some((CompiledTraceExecution::Native(native), Some(profile)))
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
        if ir.guards.len() != 1 {
            return None;
        }

        let guard = ir.guards[0];
        let lowered_exit = lowered_trace.deopt_target_for_exit_pc(guard.exit_pc)?;

        if guard.taken_on_trace {
            if ir.insts.len() < 3 {
                return None;
            }
            let guard_inst = &ir.insts[ir.insts.len() - 2];
            let loop_guard = lower_numeric_guard_for_native(guard_inst, true, lowered_exit.resume_pc)?;
            let steps = lower_numeric_steps_for_native(&ir.insts[..ir.insts.len() - 2], lowered_trace)?;
            let native = self.compile_native_numeric_jmp_loop(&[], &steps, &[NumericJmpLoopGuardBlock {
                pre_steps: Vec::new(),
                guard: loop_guard,
            }], lowered_trace)?;
            let profile = profile_for_numeric_jmp_loop(&[], &steps, &[NumericJmpLoopGuardBlock {
                pre_steps: Vec::new(),
                guard: loop_guard,
            }]);
            return Some((CompiledTraceExecution::Native(native), Some(profile)));
        }

        if ir.insts.len() < 4 || ir.insts[1].opcode != crate::OpCode::Jmp {
            return None;
        }

        let loop_guard = lower_numeric_guard_for_native(&ir.insts[0], false, lowered_exit.resume_pc)?;
        let steps = lower_numeric_steps_for_native(&ir.insts[2..ir.insts.len() - 1], lowered_trace)?;
        let head_blocks = [NumericJmpLoopGuardBlock {
            pre_steps: Vec::new(),
            guard: loop_guard,
        }];
        let native = self.compile_native_numeric_jmp_loop(&head_blocks, &steps, &[], lowered_trace)?;
        let profile = profile_for_numeric_jmp_loop(&head_blocks, &steps, &[]);
        Some((CompiledTraceExecution::Native(native), Some(profile)))
    }

    fn compile_native_return0(&mut self) -> Option<NativeCompiledTrace> {
        self.compile_native_return_trace("jit_native_return0", 0, 0, NativeReturnKind::Return0)
    }

    fn compile_native_return1(&mut self, src_reg: u32) -> Option<NativeCompiledTrace> {
        self.compile_native_return_trace("jit_native_return1", src_reg, 1, NativeReturnKind::Return1)
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
            NATIVE_HELPER_NUMERIC_GET_TABLE_INT_SYMBOL,
            jit_native_helper_numeric_get_table_int as *const u8,
        );
        builder.symbol(
            NATIVE_HELPER_NUMERIC_SET_TABLE_INT_SYMBOL,
            jit_native_helper_numeric_set_table_int as *const u8,
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
            emit_integer_guard(
                &mut builder,
                loop_ptr,
                hits_var,
                zero_hits,
                fallback_block,
            );
            emit_integer_guard(
                &mut builder,
                step_ptr,
                hits_var,
                zero_hits,
                fallback_block,
            );
            emit_integer_guard(
                &mut builder,
                index_ptr,
                hits_var,
                zero_hits,
                fallback_block,
            );
            Some(
                builder
                    .ins()
                    .load(types::I64, MemFlags::new(), step_ptr, LUA_VALUE_VALUE_OFFSET),
            )
        } else {
            None
        };
        let initial_remaining = if loop_state_is_invariant {
            let loop_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg);
            builder
                .ins()
                .load(types::I64, MemFlags::new(), loop_ptr, LUA_VALUE_VALUE_OFFSET)
        } else {
            builder.ins().iconst(types::I64, 0)
        };
        let initial_index = if loop_state_is_invariant {
            let index_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(2));
            builder
                .ins()
                .load(types::I64, MemFlags::new(), index_ptr, LUA_VALUE_VALUE_OFFSET)
        } else {
            builder.ins().iconst(types::I64, 0)
        };

        if loop_state_is_invariant {
            builder
                .ins()
                .jump(
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

        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        builder.ins().jump(guard_block, &[]);

        builder.switch_to_block(guard_block);
        let current_hits = builder.use_var(hits_var);
        if guard.is_head() {
            let cond = emit_linear_int_guard_condition(
                &mut builder,
                abi.base_ptr,
                hits_var,
                current_hits,
                fallback_block,
                guard,
            );
            builder.def_var(hits_var, current_hits);
            if continue_when {
                builder.ins().brif(cond, body_block, &[], side_exit_block, &[]);
            } else {
                builder.ins().brif(cond, side_exit_block, &[], body_block, &[]);
            }
        } else {
            builder.ins().jump(body_block, &[]);
        }

        builder.switch_to_block(body_block);
        let mut known_integer_regs = Vec::new();
        let loop_carried_values = Vec::new();
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
                guard,
            );
            builder.def_var(hits_var, next_hits);
            if continue_when {
                builder.ins().brif(cond, guard_block, &[], side_exit_block, &[]);
            } else {
                builder.ins().brif(cond, side_exit_block, &[], guard_block, &[]);
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

    fn compile_native_numeric_for_loop(
        &mut self,
        loop_reg: u32,
        steps: &[NumericStep],
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        if !steps.iter().all(native_supports_numeric_step) {
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
        let carried_float_raw_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();
        let loop_state_is_invariant = numeric_loop_state_is_invariant(loop_reg, steps);
        let carried_float_step = if loop_state_is_invariant {
            exact_float_self_update_step(steps, lowered_trace)
        } else {
            None
        };

        let loop_block = builder.create_block();
        if loop_state_is_invariant {
            builder.append_block_param(loop_block, types::I64);
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
            Some(
                builder
                    .ins()
                    .load(types::I64, MemFlags::new(), step_ptr, LUA_VALUE_VALUE_OFFSET),
            )
        } else {
            None
        };
        let initial_remaining = if loop_state_is_invariant {
            let loop_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg);
            builder
                .ins()
                .load(types::I64, MemFlags::new(), loop_ptr, LUA_VALUE_VALUE_OFFSET)
        } else {
            builder.ins().iconst(types::I64, 0)
        };
        let initial_index = if loop_state_is_invariant {
            let index_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(2));
            builder
                .ins()
                .load(types::I64, MemFlags::new(), index_ptr, LUA_VALUE_VALUE_OFFSET)
        } else {
            builder.ins().iconst(types::I64, 0)
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
            Some(
                builder
                    .ins()
                    .load(types::I64, MemFlags::new(), slot_ptr, LUA_VALUE_VALUE_OFFSET),
            )
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

        let mut initial_args = Vec::new();
        if loop_state_is_invariant {
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(initial_remaining));
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(initial_index));
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
        if carried_float_step.is_some() {
            let float_param_index = if loop_state_is_invariant { 2 } else { 0 };
            let carried_float_raw = builder.block_params(loop_block)[float_param_index];
            builder.def_var(carried_float_raw_var, carried_float_raw);
        }

        if let Some(step) = carried_float_step {
            emit_carried_float_loop_step(
                &mut builder,
                carried_float_raw_var,
                step,
                carried_float_rhs.expect("plain numeric carried-float path requires resolved rhs"),
                &mut known_value_kinds,
            );
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
                    None,
                    None,
                )?;
            }
        }

        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        if loop_state_is_invariant {
            let carried_remaining = builder.use_var(carried_remaining_var);
            let carried_index = builder.use_var(carried_index_var);
            if carried_float_step.is_some() {
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
                loop_state_is_invariant.then_some((loop_reg, carried_remaining_var, carried_index_var)),
                carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                fallback_block,
                fallback_terminal_block,
            );
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_state_is_invariant.then_some((loop_reg, carried_remaining_var, carried_index_var)),
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
        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        self.modules.push(module);
        Some(NativeCompiledTrace::NumericForLoop {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    fn compile_native_guarded_numeric_for_loop(
        &mut self,
        loop_reg: u32,
        steps: &[NumericStep],
        guard: NumericJmpLoopGuard,
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
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
        let side_exit_index = lowered_trace.deopt_target_for_exit_pc(side_exit_pc)?.exit_index;

        if !native_supports_numeric_cond(cond)
            || continue_preset.as_ref().is_some_and(|step| !native_supports_numeric_step(step))
            || exit_preset.as_ref().is_some_and(|step| !native_supports_numeric_step(step))
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
        let carried_float_raw_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();
        let loop_state_is_invariant = numeric_loop_state_is_invariant(loop_reg, steps)
            && !numeric_guard_touches_reg(guard, loop_reg)
            && !numeric_guard_touches_reg(guard, loop_reg.saturating_add(1))
            && !numeric_guard_touches_reg(guard, loop_reg.saturating_add(2));
        let carried_float_step = if loop_state_is_invariant {
            exact_float_self_update_step(steps, lowered_trace)
                .filter(|step| {
                    !numeric_guard_writes_reg_outside_condition(guard, step.reg)
                        && carried_float_rhs_stable_reg(*step).is_none_or(|reg| {
                            !numeric_guard_writes_reg_outside_condition(guard, reg)
                        })
                })
        } else {
            None
        };

        let loop_block = builder.create_block();
        if loop_state_is_invariant {
            builder.append_block_param(loop_block, types::I64);
            builder.append_block_param(loop_block, types::I64);
        }
        if carried_float_step.is_some() {
            builder.append_block_param(loop_block, types::I64);
        }
        let fallback_terminal_block = builder.create_block();
        let loop_exit_terminal_block = builder.create_block();
        let side_exit_terminal_block = builder.create_block();
        let fallback_block = if loop_state_is_invariant || carried_float_step.is_some() {
            builder.create_block()
        } else {
            fallback_terminal_block
        };
        let loop_exit_block = if loop_state_is_invariant || carried_float_step.is_some() {
            builder.create_block()
        } else {
            loop_exit_terminal_block
        };
        let side_exit_block = if loop_state_is_invariant || carried_float_step.is_some() {
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
            Some(
                builder
                    .ins()
                    .load(types::I64, MemFlags::new(), step_ptr, LUA_VALUE_VALUE_OFFSET),
            )
        } else {
            None
        };
        let initial_remaining = if loop_state_is_invariant {
            let loop_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg);
            builder
                .ins()
                .load(types::I64, MemFlags::new(), loop_ptr, LUA_VALUE_VALUE_OFFSET)
        } else {
            builder.ins().iconst(types::I64, 0)
        };
        let initial_index = if loop_state_is_invariant {
            let index_ptr = slot_addr(&mut builder, abi.base_ptr, loop_reg.saturating_add(2));
            builder
                .ins()
                .load(types::I64, MemFlags::new(), index_ptr, LUA_VALUE_VALUE_OFFSET)
        } else {
            builder.ins().iconst(types::I64, 0)
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
            Some(
                builder
                    .ins()
                    .load(types::I64, MemFlags::new(), slot_ptr, LUA_VALUE_VALUE_OFFSET),
            )
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
        let hoisted_guard_rhs = carried_float_step
            .zip(carried_float_rhs)
            .and_then(|(step, rhs)| hoisted_numeric_guard_value_from_carried_rhs(step, rhs));

        let mut initial_args = Vec::new();
        if loop_state_is_invariant {
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(initial_remaining));
            initial_args.push(cranelift::codegen::ir::BlockArg::Value(initial_index));
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
        if carried_float_step.is_some() {
            let float_param_index = if loop_state_is_invariant { 2 } else { 0 };
            let carried_float_raw = builder.block_params(loop_block)[float_param_index];
            builder.def_var(carried_float_raw_var, carried_float_raw);
        }

        if let Some(step) = carried_float_step {
            emit_carried_float_loop_step(
                &mut builder,
                carried_float_raw_var,
                step,
                carried_float_rhs.expect("guarded numeric carried-float path requires resolved rhs"),
                &mut known_value_kinds,
            );
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
                    None,
                    None,
                )?;
            }
        }

        let continue_block = builder.create_block();
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
            carried_float_step.map(|step| CarriedFloatGuardValue {
                reg: step.reg,
                raw_var: carried_float_raw_var,
            }),
            hoisted_guard_rhs,
        )?;

        builder.switch_to_block(continue_block);
        builder.seal_block(continue_block);
        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        if loop_state_is_invariant {
            let carried_remaining = builder.use_var(carried_remaining_var);
            let carried_index = builder.use_var(carried_index_var);
            if carried_float_step.is_some() {
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

        if loop_state_is_invariant || carried_float_step.is_some() {
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_state_is_invariant.then_some((loop_reg, carried_remaining_var, carried_index_var)),
                carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                fallback_block,
                fallback_terminal_block,
            );
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_state_is_invariant.then_some((loop_reg, carried_remaining_var, carried_index_var)),
                carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                loop_exit_block,
                loop_exit_terminal_block,
            );
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                loop_state_is_invariant.then_some((loop_reg, carried_remaining_var, carried_index_var)),
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

    fn compile_native_numeric_jmp_loop(
        &mut self,
        head_blocks: &[NumericJmpLoopGuardBlock],
        steps: &[NumericStep],
        tail_blocks: &[NumericJmpLoopGuardBlock],
        lowered_trace: &LoweredTrace,
    ) -> Option<NativeCompiledTrace> {
        if head_blocks.is_empty() && tail_blocks.is_empty() {
            return None;
        }

        if !steps.iter().all(native_supports_numeric_step) {
            return None;
        }

        for block in head_blocks {
            if !numeric_jmp_guard_block_is_supported(block, false, lowered_trace) {
                return None;
            }
        }
        for block in tail_blocks {
            if !numeric_jmp_guard_block_is_supported(block, true, lowered_trace) {
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
        let carried_float_raw_var = builder.declare_var(types::I64);
        let abi = init_native_entry(&mut builder, pointer_ty);
        let mut known_value_kinds = lowered_trace.entry_ssa_register_hints();
        let carried_float_step = exact_float_self_update_step(steps, lowered_trace).filter(|step| {
            !head_blocks
                .iter()
                .chain(tail_blocks.iter())
                .any(|block| numeric_guard_block_writes_reg_outside_condition(block, step.reg))
                && carried_float_rhs_stable_reg(*step).is_none_or(|reg| {
                    !head_blocks
                        .iter()
                        .chain(tail_blocks.iter())
                        .any(|block| numeric_guard_block_writes_reg_outside_condition(block, reg))
                })
        });

        let loop_block = builder.create_block();
        if carried_float_step.is_some() {
            builder.append_block_param(loop_block, types::I64);
        }
        let fallback_terminal_block = builder.create_block();
        let fallback_block = if carried_float_step.is_some() {
            builder.create_block()
        } else {
            fallback_terminal_block
        };

        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
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
            Some(
                builder
                    .ins()
                    .load(types::I64, MemFlags::new(), slot_ptr, LUA_VALUE_VALUE_OFFSET),
            )
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
        let hoisted_guard_rhs = carried_float_step
            .zip(carried_float_rhs)
            .and_then(|(step, rhs)| hoisted_numeric_guard_value_from_carried_rhs(step, rhs));
        if let Some(raw) = initial_float_raw {
            builder.ins().jump(
                loop_block,
                &[cranelift::codegen::ir::BlockArg::Value(raw)],
            );
        } else {
            builder.ins().jump(loop_block, &[]);
        }

        builder.switch_to_block(loop_block);
        let current_hits = builder.use_var(hits_var);
        if carried_float_step.is_some() {
            let carried_float_raw = builder.block_params(loop_block)[0];
            builder.def_var(carried_float_raw_var, carried_float_raw);
        }

        let mut side_exit_sites = Vec::with_capacity(head_blocks.len() + tail_blocks.len());

        for block in head_blocks {
            let continue_block = builder.create_block();
            let side_exit_terminal_block = builder.create_block();
            let side_exit_block = if carried_float_step.is_some() {
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
                carried_float_step.map(|step| CarriedFloatGuardValue {
                    reg: step.reg,
                    raw_var: carried_float_raw_var,
                }),
                hoisted_guard_rhs,
            )?;
            side_exit_sites.push((
                side_exit_block,
                side_exit_terminal_block,
                numeric_jmp_guard_exit_pc(block.guard),
                lowered_trace
                    .deopt_target_for_exit_pc(numeric_jmp_guard_exit_pc(block.guard))?
                    .exit_index,
            ));
            builder.switch_to_block(continue_block);
            builder.seal_block(continue_block);
        }

        if let Some(step) = carried_float_step {
            emit_carried_float_loop_step(
                &mut builder,
                carried_float_raw_var,
                step,
                carried_float_rhs.expect("numeric jmp carried-float path requires resolved rhs"),
                &mut known_value_kinds,
            );
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
                    None,
                    None,
                )?;
            }
        }

        let next_hits = builder.ins().iadd_imm(current_hits, 1);
        builder.def_var(hits_var, next_hits);

        for block in tail_blocks {
            let continue_block = builder.create_block();
            let side_exit_terminal_block = builder.create_block();
            let side_exit_block = if carried_float_step.is_some() {
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
                carried_float_step.map(|step| CarriedFloatGuardValue {
                    reg: step.reg,
                    raw_var: carried_float_raw_var,
                }),
                hoisted_guard_rhs,
            )?;
            side_exit_sites.push((
                side_exit_block,
                side_exit_terminal_block,
                numeric_jmp_guard_exit_pc(block.guard),
                lowered_trace
                    .deopt_target_for_exit_pc(numeric_jmp_guard_exit_pc(block.guard))?
                    .exit_index,
            ));
            builder.switch_to_block(continue_block);
            builder.seal_block(continue_block);
        }

        if carried_float_step.is_some() {
            let carried_float_raw = builder.use_var(carried_float_raw_var);
            builder.ins().jump(
                loop_block,
                &[cranelift::codegen::ir::BlockArg::Value(carried_float_raw)],
            );
        } else {
            builder.ins().jump(loop_block, &[]);
        }

        if carried_float_step.is_some() {
            emit_materialize_numeric_loop_state(
                &mut builder,
                abi.base_ptr,
                None,
                carried_float_step.map(|step| (step.reg, carried_float_raw_var)),
                fallback_block,
                fallback_terminal_block,
            );
        }

        for (side_exit_block, side_exit_terminal_block, exit_pc, exit_index) in side_exit_sites {
            if carried_float_step.is_some() {
                emit_materialize_numeric_loop_state(
                    &mut builder,
                    abi.base_ptr,
                    None,
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
                Some(exit_pc),
                Some(exit_index),
            );
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
struct CarriedFloatLoopStep {
    reg: u32,
    op: NumericBinaryOp,
    rhs: CarriedFloatRhs,
}

#[derive(Clone, Copy)]
struct CarriedFloatGuardValue {
    reg: u32,
    raw_var: Variable,
}

#[derive(Clone, Copy)]
struct HoistedNumericGuardValue {
    reg: u32,
    source: HoistedNumericGuardSource,
}

#[derive(Clone, Copy)]
enum HoistedNumericGuardSource {
    FloatRaw(Value),
    Integer(Value),
}

#[derive(Clone, Copy, Debug)]
enum CarriedFloatRhs {
    Imm(f64),
    StableReg { reg: u32, kind: TraceValueKind },
}

#[derive(Clone, Copy)]
enum ResolvedCarriedFloatRhs {
    Imm(f64),
    FloatRaw(Value),
    Integer(Value),
}

#[cfg(test)]
impl NativeTraceBackend {
    pub(crate) fn compile_test(
        &mut self,
        ir: &TraceIr,
        helper_plan: &HelperPlan,
    ) -> BackendCompileOutcome {
        let artifact = super::synthetic_artifact_for_ir(ir);
        let lowered_trace = LoweredTrace::lower(&artifact, ir, helper_plan);
        <Self as TraceBackend>::compile(self, &artifact, ir, &lowered_trace, helper_plan)
    }

    pub(crate) fn compile_test_with_constants(
        &mut self,
        ir: &TraceIr,
        helper_plan: &HelperPlan,
        constants: Vec<LuaValue>,
    ) -> BackendCompileOutcome {
        let artifact = super::synthetic_artifact_for_ir(ir);
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
        self.compile_native_numeric_jmp_loop(head_blocks, steps, tail_blocks, lowered_trace)
    }
}

fn profile_for_linear_int_for_loop(steps: &[LinearIntStep]) -> NativeLoweringProfile {
    steps.iter().copied().fold(NativeLoweringProfile::default(), |acc, step| {
        merge_native_profiles(acc, profile_for_linear_int_step(step))
    })
}

fn profile_for_linear_int_jmp_loop(steps: &[LinearIntStep], _guard: LinearIntLoopGuard) -> NativeLoweringProfile {
    merge_native_profiles(profile_for_linear_int_for_loop(steps), profile_for_linear_guard())
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

fn profile_for_numeric_for_loop(steps: &[NumericStep]) -> NativeLoweringProfile {
    profile_for_numeric_steps(steps)
}

fn profile_for_guarded_numeric_for_loop(
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

fn profile_for_numeric_jmp_loop(
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

fn profile_for_numeric_steps(steps: &[NumericStep]) -> NativeLoweringProfile {
    steps.iter().copied().fold(NativeLoweringProfile::default(), |acc, step| {
        merge_native_profiles(acc, profile_for_numeric_step(step))
    })
}

fn profile_for_numeric_step(step: NumericStep) -> NativeLoweringProfile {
    match step {
        NumericStep::GetUpval { .. } | NumericStep::SetUpval { .. } => NativeLoweringProfile {
            upvalue_helper_steps: 1,
            ..NativeLoweringProfile::default()
        },
        NumericStep::GetTableInt { .. } | NumericStep::SetTableInt { .. } => {
            NativeLoweringProfile {
                table_helper_steps: 1,
                ..NativeLoweringProfile::default()
            }
        }
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
        linear_guard_steps: lhs.linear_guard_steps.saturating_add(rhs.linear_guard_steps),
        numeric_int_compare_guard_steps: lhs
            .numeric_int_compare_guard_steps
            .saturating_add(rhs.numeric_int_compare_guard_steps),
        numeric_reg_compare_guard_steps: lhs
            .numeric_reg_compare_guard_steps
            .saturating_add(rhs.numeric_reg_compare_guard_steps),
        truthy_guard_steps: lhs.truthy_guard_steps.saturating_add(rhs.truthy_guard_steps),
        arithmetic_helper_steps: lhs
            .arithmetic_helper_steps
            .saturating_add(rhs.arithmetic_helper_steps),
        table_helper_steps: lhs.table_helper_steps.saturating_add(rhs.table_helper_steps),
        upvalue_helper_steps: lhs.upvalue_helper_steps.saturating_add(rhs.upvalue_helper_steps),
        shift_helper_steps: lhs.shift_helper_steps.saturating_add(rhs.shift_helper_steps),
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
        NumericIfElseCond::Truthy { .. } => NativeLoweringProfile {
            guard_steps: 1,
            truthy_guard_steps: 1,
            ..NativeLoweringProfile::default()
        },
    }
}

fn guard_continue_preset(guard: NumericJmpLoopGuard) -> Option<NumericStep> {
    match guard {
        NumericJmpLoopGuard::Head { continue_preset, .. }
        | NumericJmpLoopGuard::Tail { continue_preset, .. } => continue_preset,
    }
}

fn guard_exit_preset(guard: NumericJmpLoopGuard) -> Option<NumericStep> {
    match guard {
        NumericJmpLoopGuard::Head { exit_preset, .. }
        | NumericJmpLoopGuard::Tail { exit_preset, .. } => exit_preset,
    }
}

fn numeric_jmp_guard_exit_pc(guard: NumericJmpLoopGuard) -> u32 {
    match guard {
        NumericJmpLoopGuard::Head { exit_pc, .. } | NumericJmpLoopGuard::Tail { exit_pc, .. } => exit_pc,
    }
}

fn numeric_jmp_guard_block_is_supported(
    block: &NumericJmpLoopGuardBlock,
    tail: bool,
    lowered_trace: &LoweredTrace,
) -> bool {
    if !block.pre_steps.iter().all(native_supports_numeric_step) {
        return false;
    }

    let guard = block.guard;
    let matches_position = matches!((tail, guard), (false, NumericJmpLoopGuard::Head { .. }) | (true, NumericJmpLoopGuard::Tail { .. }));
    if !matches_position {
        return false;
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

fn emit_numeric_guard_block(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    block: &NumericJmpLoopGuardBlock,
    continue_block: Block,
    exit_block: Block,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<()> {
    for step in &block.pre_steps {
        emit_numeric_step(
            builder,
            abi,
            native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            *step,
            known_value_kinds,
            carried_float,
            hoisted_numeric,
        )?;
    }

    let (cond, continue_when, continue_preset, exit_preset) = match block.guard {
        NumericJmpLoopGuard::Head {
            cond,
            continue_when,
            continue_preset,
            exit_preset,
            ..
        }
        | NumericJmpLoopGuard::Tail {
            cond,
            continue_when,
            continue_preset,
            exit_preset,
            ..
        } => (cond, continue_when, continue_preset, exit_preset),
    };

    emit_numeric_guard_flow(
        builder,
        abi,
        native_helpers,
        hits_var,
        current_hits,
        fallback_block,
        cond,
        continue_when,
        continue_preset.as_ref(),
        exit_preset.as_ref(),
        continue_block,
        exit_block,
        known_value_kinds,
        carried_float,
        hoisted_numeric,
    )
}

#[derive(Clone, Copy)]
struct NativeAbi {
    pointer_ty: Type,
    base_ptr: Value,
    constants_ptr: Value,
    constants_len: Value,
    lua_state_ptr: Value,
    upvalue_ptrs: Value,
    result_ptr: Value,
}

#[derive(Clone, Copy)]
struct NativeHelpers {
    get_upval: FuncRef,
    set_upval: FuncRef,
    get_table_int: FuncRef,
    set_table_int: FuncRef,
    numeric_binary: FuncRef,
    numeric_pow: FuncRef,
    shift_left: FuncRef,
    shift_right: FuncRef,
}

#[derive(Clone, Copy)]
enum NativeReturnKind {
    Return,
    Return0,
    Return1,
}

fn init_native_entry(builder: &mut FunctionBuilder<'_>, pointer_ty: Type) -> NativeAbi {
    let entry_block = builder.create_block();
    builder.append_block_params_for_function_params(entry_block);
    builder.switch_to_block(entry_block);
    builder.seal_block(entry_block);

    let params = builder.block_params(entry_block).to_vec();
    let stack_ptr = params[0];
    let base_slots = params[1];
    let constants_ptr = params[2];
    let constants_len = params[3];
    let lua_state_ptr = params[4];
    let upvalue_ptrs = params[5];
    let result_ptr = params[6];
    let slot_scale = builder.ins().iconst(pointer_ty, LUA_VALUE_SIZE);
    let base_bytes = builder.ins().imul(base_slots, slot_scale);
    let base_ptr = builder.ins().iadd(stack_ptr, base_bytes);

    NativeAbi {
        pointer_ty,
        base_ptr,
        constants_ptr,
        constants_len,
        lua_state_ptr,
        upvalue_ptrs,
        result_ptr,
    }
}

fn make_native_context(target_config: TargetFrontendConfig) -> cranelift_codegen::Context {
    let mut context = cranelift_codegen::Context::new();
    context.func.signature.call_conv = target_config.default_call_conv;
    let pointer_ty = target_config.pointer_type();
    context.func.signature.params.push(AbiParam::new(pointer_ty));
    context.func.signature.params.push(AbiParam::new(pointer_ty));
    context.func.signature.params.push(AbiParam::new(pointer_ty));
    context.func.signature.params.push(AbiParam::new(pointer_ty));
    context.func.signature.params.push(AbiParam::new(pointer_ty));
    context.func.signature.params.push(AbiParam::new(pointer_ty));
    context.func.signature.params.push(AbiParam::new(pointer_ty));
    context
}

fn declare_native_helpers(
    module: &mut JITModule,
    func: &mut cranelift_codegen::ir::Function,
    pointer_ty: Type,
    call_conv: CallConv,
) -> Result<NativeHelpers, String> {
    fn import_helper(
        module: &mut JITModule,
        func: &mut cranelift_codegen::ir::Function,
        name: &str,
        params: &[Type],
        returns: &[Type],
        call_conv: CallConv,
    ) -> Result<FuncRef, String> {
        let mut sig = module.make_signature();
        sig.call_conv = call_conv;
        for param in params {
            sig.params.push(AbiParam::new(*param));
        }
        for ret in returns {
            sig.returns.push(AbiParam::new(*ret));
        }
        let func_id = module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|err| err.to_string())?;
        Ok(module.declare_func_in_func(func_id, func))
    }

    Ok(NativeHelpers {
        get_upval: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_GET_UPVAL_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        set_upval: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_SET_UPVAL_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        get_table_int: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_GET_TABLE_INT_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        set_table_int: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_SET_TABLE_INT_SYMBOL,
            &[pointer_ty, pointer_ty, pointer_ty, pointer_ty],
            &[types::I32],
            call_conv,
        )?,
        numeric_binary: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_BINARY_SYMBOL,
            &[
                pointer_ty,
                pointer_ty,
                pointer_ty,
                pointer_ty,
                types::I32,
                types::I64,
                types::I32,
                types::I64,
                types::I32,
            ],
            &[types::I32],
            call_conv,
        )?,
        numeric_pow: import_helper(
            module,
            func,
            NATIVE_HELPER_NUMERIC_POW_SYMBOL,
            &[types::F64, types::F64],
            &[types::F64],
            call_conv,
        )?,
        shift_left: import_helper(
            module,
            func,
            NATIVE_HELPER_SHIFT_LEFT_SYMBOL,
            &[types::I64, types::I64],
            &[types::I64],
            call_conv,
        )?,
        shift_right: import_helper(
            module,
            func,
            NATIVE_HELPER_SHIFT_RIGHT_SYMBOL,
            &[types::I64, types::I64],
            &[types::I64],
            call_conv,
        )?,
    })
}

unsafe extern "C" fn jit_native_helper_numeric_get_upval(
    dst_ptr: *mut LuaValue,
    upvalue_ptrs: *const UpvaluePtr,
    upvalue_index: usize,
) -> i32 {
    if upvalue_ptrs.is_null() {
        return 0;
    }

    let upvalue_ptr = unsafe { *upvalue_ptrs.add(upvalue_index) };
    let src = upvalue_ptr.as_ref().data.get_value_ref();
    unsafe {
        (*dst_ptr).value = src.value;
        (*dst_ptr).tt = src.tt;
    }
    1
}

unsafe extern "C" fn jit_native_helper_numeric_set_upval(
    lua_state: *mut LuaState,
    upvalue_ptrs: *const UpvaluePtr,
    src_ptr: *const LuaValue,
    upvalue_index: usize,
) -> i32 {
    if upvalue_ptrs.is_null() {
        return 0;
    }

    let value = unsafe { *src_ptr };
    let upvalue_ptr = unsafe { *upvalue_ptrs.add(upvalue_index) };
    upvalue_ptr
        .as_mut_ref()
        .data
        .set_value_parts(value.value, value.tt);

    if value.tt & 0x40 != 0 {
        let Some(gc_ptr) = value.as_gc_ptr() else {
            return 0;
        };
        if lua_state.is_null() {
            return 0;
        }
        unsafe { (*lua_state).gc_barrier(upvalue_ptr, gc_ptr) };
    }

    1
}

unsafe extern "C" fn jit_native_helper_numeric_get_table_int(
    dst_ptr: *mut LuaValue,
    table_ptr: *const LuaValue,
    index_ptr: *const LuaValue,
) -> i32 {
    if unsafe { !(*table_ptr).is_table() || !pttisinteger(index_ptr) } {
        return 0;
    }

    let table = unsafe { (*table_ptr).hvalue() };
    let index = unsafe { pivalue(index_ptr) };
    let loaded = unsafe { table.impl_table.fast_geti_into(index, dst_ptr) }
        || unsafe { table.impl_table.get_int_from_hash_into(index, dst_ptr) };
    if !loaded {
        return 0;
    }

    let loaded_value = unsafe { &*dst_ptr };
    i32::from(ttisinteger(loaded_value) || ttisfloat(loaded_value))
}

unsafe extern "C" fn jit_native_helper_numeric_set_table_int(
    lua_state: *mut LuaState,
    table_ptr: *const LuaValue,
    index_ptr: *const LuaValue,
    value_ptr: *const LuaValue,
) -> i32 {
    if unsafe { !(*table_ptr).is_table() || !pttisinteger(index_ptr) } {
        return 0;
    }

    let table = unsafe { (*table_ptr).hvalue_mut() };
    let index = unsafe { pivalue(index_ptr) };
    let value = unsafe { *value_ptr };
    if !table.impl_table.fast_seti_parts(index, value.value, value.tt) {
        return 0;
    }

    if value.tt & 0x40 != 0 {
        if lua_state.is_null() {
            return 0;
        }
        unsafe { (*lua_state).gc_barrier_back((*table_ptr).as_gc_ptr_table_unchecked()) };
    }

    1
}

extern "C" fn jit_native_helper_shift_left(lhs: i64, rhs: i64) -> i64 {
    crate::lua_vm::execute::helper::lua_shiftl(lhs, rhs)
}

extern "C" fn jit_native_helper_shift_right(lhs: i64, rhs: i64) -> i64 {
    crate::lua_vm::execute::helper::lua_shiftr(lhs, rhs)
}

extern "C" fn jit_native_helper_numeric_pow(lhs: f64, rhs: f64) -> f64 {
    luai_numpow(lhs, rhs)
}

unsafe extern "C" fn jit_native_helper_numeric_binary(
    dst_ptr: *mut LuaValue,
    base_ptr: *const LuaValue,
    constants_ptr: *const LuaValue,
    constants_len: usize,
    lhs_kind: i32,
    lhs_payload: i64,
    rhs_kind: i32,
    rhs_payload: i64,
    op: i32,
) -> i32 {
    unsafe fn operand_ptr(
        base_ptr: *const LuaValue,
        constants_ptr: *const LuaValue,
        constants_len: usize,
        kind: i32,
        payload: i64,
        immediate: &mut LuaValue,
    ) -> Option<*const LuaValue> {
        match kind {
            NATIVE_NUMERIC_OPERAND_REG => {
                let reg = usize::try_from(payload).ok()?;
                Some(unsafe { base_ptr.add(reg) })
            }
            NATIVE_NUMERIC_OPERAND_IMM_I => {
                *immediate = LuaValue::integer(payload);
                Some(immediate as *const LuaValue)
            }
            NATIVE_NUMERIC_OPERAND_CONST => {
                let index = usize::try_from(payload).ok()?;
                if index >= constants_len {
                    return None;
                }
                Some(unsafe { constants_ptr.add(index) })
            }
            _ => None,
        }
    }

    let mut lhs_immediate = LuaValue::nil();
    let mut rhs_immediate = LuaValue::nil();
    let Some(lhs_ptr) = (unsafe {
        operand_ptr(
            base_ptr,
            constants_ptr,
            constants_len,
            lhs_kind,
            lhs_payload,
            &mut lhs_immediate,
        )
    }) else {
        return 0;
    };
    let Some(rhs_ptr) = (unsafe {
        operand_ptr(
            base_ptr,
            constants_ptr,
            constants_len,
            rhs_kind,
            rhs_payload,
            &mut rhs_immediate,
        )
    }) else {
        return 0;
    };

    let lhs = unsafe { &*lhs_ptr };
    let rhs = unsafe { &*rhs_ptr };
    let result = match op {
        NATIVE_NUMERIC_BINARY_ADD => {
            if let (Some(lhs_int), Some(rhs_int)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
                LuaValue::integer(lhs_int.wrapping_add(rhs_int))
            } else {
                let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
                let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
                if lhs_num.is_nan() || rhs_num.is_nan() {
                    return 0;
                }
                LuaValue::float(lhs_num + rhs_num)
            }
        }
        NATIVE_NUMERIC_BINARY_SUB => {
            if let (Some(lhs_int), Some(rhs_int)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
                LuaValue::integer(lhs_int.wrapping_sub(rhs_int))
            } else {
                let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
                let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
                if lhs_num.is_nan() || rhs_num.is_nan() {
                    return 0;
                }
                LuaValue::float(lhs_num - rhs_num)
            }
        }
        NATIVE_NUMERIC_BINARY_MUL => {
            if let (Some(lhs_int), Some(rhs_int)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
                LuaValue::integer(lhs_int.wrapping_mul(rhs_int))
            } else {
                let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
                let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
                if lhs_num.is_nan() || rhs_num.is_nan() {
                    return 0;
                }
                LuaValue::float(lhs_num * rhs_num)
            }
        }
        NATIVE_NUMERIC_BINARY_DIV => {
            let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
            let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
            if lhs_num.is_nan() || rhs_num.is_nan() {
                return 0;
            }
            LuaValue::float(lhs_num / rhs_num)
        }
        NATIVE_NUMERIC_BINARY_IDIV => {
            if let (Some(lhs_int), Some(rhs_int)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
                if rhs_int == 0 {
                    return 0;
                }
                LuaValue::integer(lua_idiv(lhs_int, rhs_int))
            } else {
                let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
                let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
                if lhs_num.is_nan() || rhs_num.is_nan() || rhs_num == 0.0 {
                    return 0;
                }
                LuaValue::float((lhs_num / rhs_num).floor())
            }
        }
        NATIVE_NUMERIC_BINARY_MOD => {
            if let (Some(lhs_int), Some(rhs_int)) = (lhs.as_integer_strict(), rhs.as_integer_strict()) {
                if rhs_int == 0 {
                    return 0;
                }
                LuaValue::integer(lua_imod(lhs_int, rhs_int))
            } else {
                let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
                let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
                if lhs_num.is_nan() || rhs_num.is_nan() || rhs_num == 0.0 {
                    return 0;
                }
                LuaValue::float(lua_fmod(lhs_num, rhs_num))
            }
        }
        NATIVE_NUMERIC_BINARY_POW => {
            let lhs_num = lhs.as_float().unwrap_or(f64::NAN);
            let rhs_num = rhs.as_float().unwrap_or(f64::NAN);
            if lhs_num.is_nan() || rhs_num.is_nan() {
                return 0;
            }
            LuaValue::float(luai_numpow(lhs_num, rhs_num))
        }
        _ => return 0,
    };

    unsafe {
        (*dst_ptr).value = result.value;
        (*dst_ptr).tt = result.tt;
    }
    1
}

fn slot_addr(builder: &mut FunctionBuilder<'_>, base_ptr: Value, reg: u32) -> Value {
    builder
        .ins()
        .iadd_imm(base_ptr, i64::from(reg).saturating_mul(LUA_VALUE_SIZE))
}

fn const_addr(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    index: u32,
) -> Value {
    let idx_value = builder.ins().iconst(abi.pointer_ty, i64::from(index));
    let in_bounds = builder
        .ins()
        .icmp(IntCC::UnsignedLessThan, idx_value, abi.constants_len);
    let continue_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    builder.ins().brif(in_bounds, continue_block, &[], fallback_block, &[]);
    builder.switch_to_block(continue_block);
    builder.seal_block(continue_block);
    builder
        .ins()
        .iadd_imm(abi.constants_ptr, i64::from(index).saturating_mul(LUA_VALUE_SIZE))
}

fn emit_copy_luavalue(builder: &mut FunctionBuilder<'_>, dst_ptr: Value, src_ptr: Value) {
    let mem = MemFlags::new();
    let raw_value = builder.ins().load(types::I64, mem, src_ptr, LUA_VALUE_VALUE_OFFSET);
    let raw_tag = builder.ins().load(types::I8, mem, src_ptr, LUA_VALUE_TT_OFFSET);
    builder.ins().store(mem, raw_value, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    builder.ins().store(mem, raw_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
}

fn emit_store_boolean_with_known_tag(
    builder: &mut FunctionBuilder<'_>,
    dst_ptr: Value,
    value: bool,
    dst_known_boolean: bool,
) {
    let mem = MemFlags::new();
    let zero = builder.ins().iconst(types::I64, 0);
    builder.ins().store(mem, zero, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    if !dst_known_boolean {
        let bool_tag = builder.ins().iconst(
            types::I8,
            if value { LUA_VTRUE_TAG as i64 } else { LUA_VFALSE_TAG as i64 },
        );
        builder.ins().store(mem, bool_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
    }
}

fn emit_store_float_with_known_tag(
    builder: &mut FunctionBuilder<'_>,
    dst_ptr: Value,
    value: f64,
    dst_known_float: bool,
) {
    let mem = MemFlags::new();
    let raw = builder.ins().iconst(types::I64, value.to_bits() as i64);
    builder.ins().store(mem, raw, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    if !dst_known_float {
        let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
        builder.ins().store(mem, float_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
    }
}

fn emit_store_float_value_with_known_tag(
    builder: &mut FunctionBuilder<'_>,
    dst_ptr: Value,
    value: Value,
    dst_known_float: bool,
) {
    let mem = MemFlags::new();
    let raw = builder.ins().bitcast(types::I64, mem, value);
    builder.ins().store(mem, raw, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    if !dst_known_float {
        let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
        builder.ins().store(mem, float_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
    }
}

fn emit_numeric_tagged_value_to_float(
    builder: &mut FunctionBuilder<'_>,
    tag: Value,
    value: Value,
) -> Value {
    let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
    let is_int = builder.ins().icmp(IntCC::Equal, tag, int_tag);
    let as_float_int = builder.ins().fcvt_from_sint(types::F64, value);
    let as_float_raw = builder.ins().bitcast(types::F64, MemFlags::new(), value);
    builder.ins().select(is_int, as_float_int, as_float_raw)
}

fn emit_numeric_operand_kind_and_payload(
    builder: &mut FunctionBuilder<'_>,
    operand: NumericOperand,
) -> (Value, Value) {
    match operand {
        NumericOperand::Reg(reg) => (
            builder.ins().iconst(types::I32, i64::from(NATIVE_NUMERIC_OPERAND_REG)),
            builder.ins().iconst(types::I64, i64::from(reg)),
        ),
        NumericOperand::ImmI(imm) => (
            builder.ins().iconst(types::I32, i64::from(NATIVE_NUMERIC_OPERAND_IMM_I)),
            builder.ins().iconst(types::I64, i64::from(imm)),
        ),
        NumericOperand::Const(index) => (
            builder.ins().iconst(types::I32, i64::from(NATIVE_NUMERIC_OPERAND_CONST)),
            builder.ins().iconst(types::I64, i64::from(index)),
        ),
    }
}

fn emit_numeric_binary_helper_opcode(
    builder: &mut FunctionBuilder<'_>,
    op: NumericBinaryOp,
) -> Option<Value> {
    let opcode = match op {
        NumericBinaryOp::Add => NATIVE_NUMERIC_BINARY_ADD,
        NumericBinaryOp::Sub => NATIVE_NUMERIC_BINARY_SUB,
        NumericBinaryOp::Mul => NATIVE_NUMERIC_BINARY_MUL,
        NumericBinaryOp::Div => NATIVE_NUMERIC_BINARY_DIV,
        NumericBinaryOp::IDiv => NATIVE_NUMERIC_BINARY_IDIV,
        NumericBinaryOp::Mod => NATIVE_NUMERIC_BINARY_MOD,
        NumericBinaryOp::Pow => NATIVE_NUMERIC_BINARY_POW,
        _ => return None,
    };
    Some(builder.ins().iconst(types::I32, i64::from(opcode)))
}

fn emit_integer_guard(
    builder: &mut FunctionBuilder<'_>,
    slot_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
) {
    emit_exact_tag_guard(builder, slot_ptr, LUA_VNUMINT, hits_var, current_hits, bail_block);
}

fn emit_exact_tag_guard(
    builder: &mut FunctionBuilder<'_>,
    slot_ptr: Value,
    expected_tag: u8,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
) {
    let mem = MemFlags::new();
    let tt = builder.ins().load(types::I8, mem, slot_ptr, LUA_VALUE_TT_OFFSET);
    let tag_matches = builder.ins().icmp_imm(IntCC::Equal, tt, i64::from(expected_tag));
    let next_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    builder.ins().brif(tag_matches, next_block, &[], bail_block, &[]);
    builder.switch_to_block(next_block);
    builder.seal_block(next_block);
}

fn emit_native_terminal_result(
    builder: &mut FunctionBuilder<'_>,
    block: Block,
    result_ptr: Value,
    hits_var: Variable,
    status: NativeTraceStatus,
    exit_pc: Option<u32>,
    exit_index: Option<u16>,
) {
    builder.switch_to_block(block);
    let hits = builder.use_var(hits_var);
    emit_store_native_result(
        builder,
        result_ptr,
        status,
        hits,
        exit_pc.unwrap_or(0),
        exit_index.unwrap_or(0),
    );
    builder.ins().return_(&[]);
    builder.seal_block(block);
}

fn emit_store_native_result(
    builder: &mut FunctionBuilder<'_>,
    result_ptr: Value,
    status: NativeTraceStatus,
    hits: Value,
    exit_pc: u32,
    exit_index: u16,
) {
    emit_store_native_result_extended(
        builder,
        result_ptr,
        status,
        hits,
        exit_pc,
        0,
        0,
        u32::from(exit_index),
    );
}

fn emit_store_native_result_extended(
    builder: &mut FunctionBuilder<'_>,
    result_ptr: Value,
    status: NativeTraceStatus,
    hits: Value,
    exit_pc: u32,
    start_reg: u32,
    result_count: u32,
    exit_index: u32,
) {
    let mem = MemFlags::new();
    let status_value = builder.ins().iconst(types::I32, status as i64);
    let hits_value = builder.ins().ireduce(types::I32, hits);
    let exit_pc_value = builder.ins().iconst(types::I32, i64::from(exit_pc));
    let start_reg_value = builder.ins().iconst(types::I32, i64::from(start_reg));
    let result_count_value = builder.ins().iconst(types::I32, i64::from(result_count));
    let exit_index_value = builder.ins().iconst(types::I32, i64::from(exit_index));
    builder
        .ins()
        .store(mem, status_value, result_ptr, NATIVE_TRACE_RESULT_STATUS_OFFSET);
    builder
        .ins()
        .store(mem, hits_value, result_ptr, NATIVE_TRACE_RESULT_HITS_OFFSET);
    builder
        .ins()
        .store(mem, exit_pc_value, result_ptr, NATIVE_TRACE_RESULT_EXIT_PC_OFFSET);
    builder
        .ins()
        .store(mem, start_reg_value, result_ptr, NATIVE_TRACE_RESULT_START_REG_OFFSET);
    builder
        .ins()
        .store(mem, result_count_value, result_ptr, NATIVE_TRACE_RESULT_RESULT_COUNT_OFFSET);
    builder
        .ins()
        .store(mem, exit_index_value, result_ptr, NATIVE_TRACE_RESULT_EXIT_INDEX_OFFSET);
}

fn emit_native_return_result(
    builder: &mut FunctionBuilder<'_>,
    result_ptr: Value,
    start_reg: u32,
    result_count: u32,
) {
    let hits = builder.ins().iconst(types::I64, 1);
    emit_store_native_result_extended(
        builder,
        result_ptr,
        NativeTraceStatus::Returned,
        hits,
        0,
        start_reg,
        result_count,
        0,
    );
    builder.ins().return_(&[]);
}

fn emit_linear_int_counted_loop_backedge(
    builder: &mut FunctionBuilder<'_>,
    hits_var: Variable,
    next_hits: Value,
    carried_remaining: Value,
    carried_index: Value,
    hoisted_step_value: Option<Value>,
    loop_block: Block,
    loop_exit_block: Block,
) {
    let has_more = builder
        .ins()
        .icmp_imm(IntCC::UnsignedGreaterThan, carried_remaining, 0);
    let continue_block = builder.create_block();
    builder.def_var(hits_var, next_hits);
    builder.ins().brif(has_more, continue_block, &[], loop_exit_block, &[]);

    builder.switch_to_block(continue_block);
    let step_val = hoisted_step_value.expect("linear-int for-loop invariant path requires hoisted step");
    let updated_remaining = builder.ins().iadd_imm(carried_remaining, -1);
    let updated_index = builder.ins().iadd(carried_index, step_val);
    builder
        .ins()
        .jump(
            loop_block,
            &[
                cranelift::codegen::ir::BlockArg::Value(updated_remaining),
                cranelift::codegen::ir::BlockArg::Value(updated_index),
            ],
        );
    builder.seal_block(continue_block);
}

    fn emit_numeric_counted_loop_backedge_with_carried_float(
        builder: &mut FunctionBuilder<'_>,
        hits_var: Variable,
        next_hits: Value,
        carried_remaining: Value,
        carried_index: Value,
        hoisted_step_value: Option<Value>,
        carried_float_raw: Value,
        loop_block: Block,
        loop_exit_block: Block,
    ) {
        let has_more = builder
            .ins()
            .icmp_imm(IntCC::UnsignedGreaterThan, carried_remaining, 0);
        let continue_block = builder.create_block();
        builder.def_var(hits_var, next_hits);
        builder.ins().brif(has_more, continue_block, &[], loop_exit_block, &[]);

        builder.switch_to_block(continue_block);
        let step_val = hoisted_step_value.expect("numeric invariant path requires hoisted step");
        let updated_remaining = builder.ins().iadd_imm(carried_remaining, -1);
        let updated_index = builder.ins().iadd(carried_index, step_val);
        builder.ins().jump(
            loop_block,
            &[
                cranelift::codegen::ir::BlockArg::Value(updated_remaining),
                cranelift::codegen::ir::BlockArg::Value(updated_index),
                cranelift::codegen::ir::BlockArg::Value(carried_float_raw),
            ],
        );
        builder.seal_block(continue_block);
    }

fn emit_linear_int_materialize_loop_state(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    loop_reg: u32,
    carried_remaining_var: Variable,
    carried_index_var: Variable,
    source_block: Block,
    target_block: Block,
) {
    builder.switch_to_block(source_block);
    let loop_ptr = slot_addr(builder, base_ptr, loop_reg);
    let index_ptr = slot_addr(builder, base_ptr, loop_reg.saturating_add(2));
    let carried_remaining = builder.use_var(carried_remaining_var);
    let carried_index = builder.use_var(carried_index_var);
    emit_store_integer_with_known_tag(builder, loop_ptr, carried_remaining, true);
    emit_store_integer_with_known_tag(builder, index_ptr, carried_index, true);
    builder.ins().jump(target_block, &[]);
    builder.seal_block(source_block);
}

fn emit_store_float_raw_with_known_tag(
    builder: &mut FunctionBuilder<'_>,
    dst_ptr: Value,
    raw: Value,
    dst_known_float: bool,
) {
    let mem = MemFlags::new();
    builder.ins().store(mem, raw, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    if !dst_known_float {
        let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
        builder.ins().store(mem, float_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
    }
}

fn emit_materialize_numeric_loop_state(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    loop_state: Option<(u32, Variable, Variable)>,
    carried_float: Option<(u32, Variable)>,
    source_block: Block,
    target_block: Block,
) {
    builder.switch_to_block(source_block);
    if let Some((loop_reg, carried_remaining_var, carried_index_var)) = loop_state {
        let loop_ptr = slot_addr(builder, base_ptr, loop_reg);
        let index_ptr = slot_addr(builder, base_ptr, loop_reg.saturating_add(2));
        let carried_remaining = builder.use_var(carried_remaining_var);
        let carried_index = builder.use_var(carried_index_var);
        emit_store_integer_with_known_tag(builder, loop_ptr, carried_remaining, true);
        emit_store_integer_with_known_tag(builder, index_ptr, carried_index, true);
    }
    if let Some((reg, carried_float_raw_var)) = carried_float {
        let ptr = slot_addr(builder, base_ptr, reg);
        let raw = builder.use_var(carried_float_raw_var);
        emit_store_float_raw_with_known_tag(builder, ptr, raw, true);
    }
    builder.ins().jump(target_block, &[]);
    builder.seal_block(source_block);
}

fn emit_counted_loop_backedge(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    next_hits: Value,
    loop_reg: u32,
    hoisted_step_value: Option<Value>,
    loop_state_is_invariant: bool,
    loop_block: Block,
    loop_exit_block: Block,
    fallback_block: Block,
) {
    let loop_ptr = slot_addr(builder, base_ptr, loop_reg);
    let step_ptr = slot_addr(builder, base_ptr, loop_reg.saturating_add(1));
    let index_ptr = slot_addr(builder, base_ptr, loop_reg.saturating_add(2));
    if !loop_state_is_invariant {
        emit_integer_guard(builder, loop_ptr, hits_var, current_hits, fallback_block);
        emit_integer_guard(builder, step_ptr, hits_var, current_hits, fallback_block);
        emit_integer_guard(builder, index_ptr, hits_var, current_hits, fallback_block);
    }

    let mem = MemFlags::new();
    let remaining = builder.ins().load(types::I64, mem, loop_ptr, LUA_VALUE_VALUE_OFFSET);
    let has_more = builder
        .ins()
        .icmp_imm(IntCC::UnsignedGreaterThan, remaining, 0);
    let continue_block = builder.create_block();
    builder.def_var(hits_var, next_hits);
    builder.ins().brif(has_more, continue_block, &[], loop_exit_block, &[]);

    builder.switch_to_block(continue_block);
    let step_val = hoisted_step_value
        .unwrap_or_else(|| builder.ins().load(types::I64, mem, step_ptr, LUA_VALUE_VALUE_OFFSET));
    let index_val = builder.ins().load(types::I64, mem, index_ptr, LUA_VALUE_VALUE_OFFSET);
    let updated_remaining = builder.ins().iadd_imm(remaining, -1);
    let updated_index = builder.ins().iadd(index_val, step_val);
    builder
        .ins()
        .store(mem, updated_remaining, loop_ptr, LUA_VALUE_VALUE_OFFSET);
    builder
        .ins()
        .store(mem, updated_index, index_ptr, LUA_VALUE_VALUE_OFFSET);
    if !loop_state_is_invariant {
        let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
        builder.ins().store(mem, int_tag, loop_ptr, LUA_VALUE_TT_OFFSET);
        builder.ins().store(mem, int_tag, index_ptr, LUA_VALUE_TT_OFFSET);
    }
    builder.ins().jump(loop_block, &[]);
    builder.seal_block(continue_block);
}

fn linear_int_step_writes_reg(step: LinearIntStep, reg: u32) -> bool {
    match step {
        LinearIntStep::Move { dst, .. }
        | LinearIntStep::LoadI { dst, .. }
        | LinearIntStep::BNot { dst, .. }
        | LinearIntStep::Add { dst, .. }
        | LinearIntStep::AddI { dst, .. }
        | LinearIntStep::Sub { dst, .. }
        | LinearIntStep::SubI { dst, .. }
        | LinearIntStep::Mul { dst, .. }
        | LinearIntStep::MulI { dst, .. }
        | LinearIntStep::IDiv { dst, .. }
        | LinearIntStep::IDivI { dst, .. }
        | LinearIntStep::Mod { dst, .. }
        | LinearIntStep::ModI { dst, .. }
        | LinearIntStep::BAnd { dst, .. }
        | LinearIntStep::BAndI { dst, .. }
        | LinearIntStep::BOr { dst, .. }
        | LinearIntStep::BOrI { dst, .. }
        | LinearIntStep::BXor { dst, .. }
        | LinearIntStep::BXorI { dst, .. }
        | LinearIntStep::Shl { dst, .. }
        | LinearIntStep::ShlI { dst, .. }
        | LinearIntStep::Shr { dst, .. }
        | LinearIntStep::ShrI { dst, .. } => dst == reg,
    }
}

fn linear_int_loop_state_is_invariant(loop_reg: u32, steps: &[LinearIntStep]) -> bool {
    let step_reg = loop_reg.saturating_add(1);
    let index_reg = loop_reg.saturating_add(2);
    !steps.iter().copied().any(|step| {
        linear_int_step_writes_reg(step, loop_reg)
            || linear_int_step_writes_reg(step, step_reg)
            || linear_int_step_writes_reg(step, index_reg)
    })
}

fn numeric_step_writes_reg(step: NumericStep, reg: u32) -> bool {
    match step {
        NumericStep::Move { dst, .. }
        | NumericStep::LoadBool { dst, .. }
        | NumericStep::LoadI { dst, .. }
        | NumericStep::LoadF { dst, .. }
        | NumericStep::GetUpval { dst, .. }
        | NumericStep::GetTableInt { dst, .. }
        | NumericStep::Binary { dst, .. } => dst == reg,
        NumericStep::SetUpval { .. } | NumericStep::SetTableInt { .. } => false,
    }
}

fn numeric_operand_reads_reg(operand: NumericOperand, reg: u32) -> bool {
    matches!(operand, NumericOperand::Reg(operand_reg) if operand_reg == reg)
}

fn numeric_step_reads_reg(step: NumericStep, reg: u32) -> bool {
    match step {
        NumericStep::Move { src, .. } => src == reg,
        NumericStep::LoadBool { .. } | NumericStep::LoadI { .. } | NumericStep::LoadF { .. } => {
            false
        }
        NumericStep::GetUpval { .. } => false,
        NumericStep::SetUpval { src, .. } => src == reg,
        NumericStep::GetTableInt { table, index, .. } => table == reg || index == reg,
        NumericStep::SetTableInt { table, index, value } => {
            table == reg || index == reg || value == reg
        }
        NumericStep::Binary { lhs, rhs, .. } => {
            numeric_operand_reads_reg(lhs, reg) || numeric_operand_reads_reg(rhs, reg)
        }
    }
}

fn numeric_loop_state_is_invariant(loop_reg: u32, steps: &[NumericStep]) -> bool {
    let step_reg = loop_reg.saturating_add(1);
    let index_reg = loop_reg.saturating_add(2);
    !steps.iter().copied().any(|step| {
        numeric_step_reads_reg(step, loop_reg)
            || numeric_step_reads_reg(step, step_reg)
            || numeric_step_reads_reg(step, index_reg)
            || numeric_step_writes_reg(step, loop_reg)
            || numeric_step_writes_reg(step, step_reg)
            || numeric_step_writes_reg(step, index_reg)
    })
}

fn numeric_cond_reads_reg(cond: NumericIfElseCond, reg: u32) -> bool {
    match cond {
        NumericIfElseCond::RegCompare { lhs, rhs, .. } => lhs == reg || rhs == reg,
        NumericIfElseCond::Truthy { reg: cond_reg } => cond_reg == reg,
    }
}

fn numeric_guard_touches_reg(guard: NumericJmpLoopGuard, reg: u32) -> bool {
    let (cond, continue_preset, exit_preset) = match guard {
        NumericJmpLoopGuard::Head {
            cond,
            continue_preset,
            exit_preset,
            ..
        }
        | NumericJmpLoopGuard::Tail {
            cond,
            continue_preset,
            exit_preset,
            ..
        } => (cond, continue_preset, exit_preset),
    };

    numeric_cond_reads_reg(cond, reg)
        || continue_preset.is_some_and(|step| {
            numeric_step_reads_reg(step, reg) || numeric_step_writes_reg(step, reg)
        })
        || exit_preset.is_some_and(|step| {
            numeric_step_reads_reg(step, reg) || numeric_step_writes_reg(step, reg)
        })
}

    fn numeric_guard_writes_reg_outside_condition(guard: NumericJmpLoopGuard, reg: u32) -> bool {
        let (_, continue_preset, exit_preset) = match guard {
            NumericJmpLoopGuard::Head {
                cond: _,
                continue_preset,
                exit_preset,
                ..
            }
            | NumericJmpLoopGuard::Tail {
                cond: _,
                continue_preset,
                exit_preset,
                ..
            } => ((), continue_preset, exit_preset),
        };

        continue_preset.is_some_and(|step| numeric_step_writes_reg(step, reg))
            || exit_preset.is_some_and(|step| numeric_step_writes_reg(step, reg))
    }

    fn numeric_guard_block_writes_reg_outside_condition(block: &NumericJmpLoopGuardBlock, reg: u32) -> bool {
        block
            .pre_steps
            .iter()
            .copied()
            .any(|step| numeric_step_writes_reg(step, reg))
            || numeric_guard_writes_reg_outside_condition(block.guard, reg)
    }

fn linear_int_reg_is_known_integer(known_integer_regs: &[u32], reg: u32) -> bool {
    known_integer_regs.contains(&reg)
}

fn mark_linear_int_reg_known_integer(known_integer_regs: &mut Vec<u32>, reg: u32) {
    if !linear_int_reg_is_known_integer(known_integer_regs, reg) {
        known_integer_regs.push(reg);
    }
}

fn numeric_reg_value_kind(known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint], reg: u32) -> TraceValueKind {
    known_value_kinds
        .iter()
        .rev()
        .find_map(|hint| (hint.reg == reg).then_some(hint.kind))
        .unwrap_or(TraceValueKind::Unknown)
}

fn set_numeric_reg_value_kind(
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
    reg: u32,
    kind: TraceValueKind,
) {
    if let Some(existing) = known_value_kinds.iter_mut().rev().find(|hint| hint.reg == reg) {
        existing.kind = kind;
    } else {
        known_value_kinds.push(crate::lua_vm::jit::lowering::RegisterValueHint { reg, kind });
    }
}

fn trace_value_kind_tag(kind: TraceValueKind) -> Option<u8> {
    match kind {
        TraceValueKind::Integer => Some(LUA_VNUMINT),
        TraceValueKind::Float => Some(LUA_VNUMFLT),
        TraceValueKind::Boolean => Some(LUA_VTRUE_TAG),
        _ => None,
    }
}

fn emit_store_integer_with_known_tag(
    builder: &mut FunctionBuilder<'_>,
    dst_ptr: Value,
    value: Value,
    dst_known_integer: bool,
) {
    let mem = MemFlags::new();
    builder.ins().store(mem, value, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    if !dst_known_integer {
        let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
        builder.ins().store(mem, int_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
    }
}

fn emit_known_linear_int_reg_value(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &[u32],
    loop_carried_values: &[(u32, Value)],
    reg: u32,
) -> Value {
    if let Some(value) = loop_carried_values
        .iter()
        .find_map(|(carried_reg, value)| (*carried_reg == reg).then_some(*value))
    {
        return value;
    }
    let mem = MemFlags::new();
    let reg_ptr = slot_addr(builder, base_ptr, reg);
    if !linear_int_reg_is_known_integer(known_integer_regs, reg) {
        emit_integer_guard(builder, reg_ptr, hits_var, current_hits, bail_block);
    }
    builder.ins().load(types::I64, mem, reg_ptr, LUA_VALUE_VALUE_OFFSET)
}

fn emit_linear_int_guard_condition(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    guard: LinearIntLoopGuard,
) -> Value {
    let mem = MemFlags::new();
    let (op, lhs_val, rhs_val) = match guard {
        LinearIntLoopGuard::HeadRegReg { op, lhs, rhs, .. }
        | LinearIntLoopGuard::TailRegReg { op, lhs, rhs, .. } => {
            let lhs_ptr = slot_addr(builder, base_ptr, lhs);
            let rhs_ptr = slot_addr(builder, base_ptr, rhs);
            emit_integer_guard(builder, lhs_ptr, hits_var, current_hits, fallback_block);
            emit_integer_guard(builder, rhs_ptr, hits_var, current_hits, fallback_block);
            let lhs_val = builder.ins().load(types::I64, mem, lhs_ptr, LUA_VALUE_VALUE_OFFSET);
            let rhs_val = builder.ins().load(types::I64, mem, rhs_ptr, LUA_VALUE_VALUE_OFFSET);
            (op, lhs_val, rhs_val)
        }
        LinearIntLoopGuard::HeadRegImm { op, reg, imm, .. }
        | LinearIntLoopGuard::TailRegImm { op, reg, imm, .. } => {
            let reg_ptr = slot_addr(builder, base_ptr, reg);
            emit_integer_guard(builder, reg_ptr, hits_var, current_hits, fallback_block);
            let lhs_val = builder.ins().load(types::I64, mem, reg_ptr, LUA_VALUE_VALUE_OFFSET);
            let rhs_val = builder.ins().iconst(types::I64, i64::from(imm));
            (op, lhs_val, rhs_val)
        }
    };

    emit_linear_compare(builder, lhs_val, rhs_val, op)
}

fn emit_linear_int_step(
    builder: &mut FunctionBuilder<'_>,
    native_helpers: &NativeHelpers,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    step: LinearIntStep,
    known_integer_regs: &mut Vec<u32>,
    loop_carried_values: &[(u32, Value)],
) {
    match step {
        LinearIntStep::Move { dst, src } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let src_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                src,
            );
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, src_val, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
        }
        LinearIntStep::LoadI { dst, imm } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let dst_val = builder.ins().iconst(types::I64, i64::from(imm));
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, dst_val, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
        }
        LinearIntStep::BNot { dst, src } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let src_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                src,
            );
            let result = builder.ins().bnot(src_val);
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
        }
        LinearIntStep::Add { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().iadd(l, r),
            );
        }
        LinearIntStep::AddI { dst, src, imm } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let src_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                src,
            );
            let imm_val = builder.ins().iconst(types::I64, i64::from(imm));
            let result = builder.ins().iadd(src_val, imm_val);
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
        }
        LinearIntStep::Sub { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().isub(l, r),
            );
        }
        LinearIntStep::SubI { dst, src, imm } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let src_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                src,
            );
            let imm_val = builder.ins().iconst(types::I64, i64::from(imm));
            let result = builder.ins().isub(src_val, imm_val);
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
        }
        LinearIntStep::Mul { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().imul(l, r),
            );
        }
        LinearIntStep::MulI { dst, src, imm } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let src_val = emit_known_linear_int_reg_value(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                src,
            );
            let imm_val = builder.ins().iconst(types::I64, i64::from(imm));
            let result = builder.ins().imul(src_val, imm_val);
            let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            mark_linear_int_reg_known_integer(known_integer_regs, dst);
        }
        LinearIntStep::IDiv { dst, lhs, rhs } => {
            emit_linear_int_div_mod_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                false,
            );
        }
        LinearIntStep::IDivI { dst, src, imm } => {
            emit_linear_int_div_mod_imm(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                src,
                imm,
                false,
            );
        }
        LinearIntStep::Mod { dst, lhs, rhs } => {
            emit_linear_int_div_mod_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                true,
            );
        }
        LinearIntStep::ModI { dst, src, imm } => {
            emit_linear_int_div_mod_imm(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                src,
                imm,
                true,
            );
        }
        LinearIntStep::BAnd { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().band(l, r),
            );
        }
        LinearIntStep::BAndI { dst, src, imm } => {
            emit_linear_int_imm_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                src,
                imm,
                |b, value, rhs| b.ins().band(value, rhs),
            );
        }
        LinearIntStep::BOr { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().bor(l, r),
            );
        }
        LinearIntStep::BOrI { dst, src, imm } => {
            emit_linear_int_imm_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                src,
                imm,
                |b, value, rhs| b.ins().bor(value, rhs),
            );
        }
        LinearIntStep::BXor { dst, lhs, rhs } => {
            emit_binary_int_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                |b, l, r| b.ins().bxor(l, r),
            );
        }
        LinearIntStep::BXorI { dst, src, imm } => {
            emit_linear_int_imm_op(
                builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                src,
                imm,
                |b, value, rhs| b.ins().bxor(value, rhs),
            );
        }
        LinearIntStep::Shl { dst, lhs, rhs } => {
            emit_linear_int_shift_op(
                builder,
                native_helpers,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                true,
            );
        }
        LinearIntStep::ShlI { dst, imm, src } => {
            emit_linear_int_shift_imm_lhs(
                builder,
                native_helpers,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                imm,
                src,
            );
        }
        LinearIntStep::Shr { dst, lhs, rhs } => {
            emit_linear_int_shift_op(
                builder,
                native_helpers,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                lhs,
                rhs,
                false,
            );
        }
        LinearIntStep::ShrI { dst, src, imm } => {
            emit_linear_int_imm_shift_rhs(
                builder,
                native_helpers,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                known_integer_regs,
                loop_carried_values,
                dst,
                src,
                imm,
            );
        }
    }
}

fn emit_linear_int_imm_op<F>(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    src: u32,
    imm: i32,
    op: F,
) where
    F: Fn(&mut FunctionBuilder<'_>, Value, Value) -> Value,
{
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let src_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        loop_carried_values,
        src,
    );
    let imm_val = builder.ins().iconst(types::I64, i64::from(imm));
    let result = op(builder, src_val, imm_val);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
}

fn emit_linear_int_div_mod_op(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    lhs: u32,
    rhs: u32,
    modulo: bool,
) {
    let lhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        loop_carried_values,
        lhs,
    );
    let rhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        loop_carried_values,
        rhs,
    );
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    if modulo {
        emit_integer_mod(
            builder,
            hits_var,
            current_hits,
            bail_block,
            dst_ptr,
            lhs_val,
            rhs_val,
            dst_known_integer,
        );
    } else {
        emit_integer_idiv(
            builder,
            hits_var,
            current_hits,
            bail_block,
            dst_ptr,
            lhs_val,
            rhs_val,
            dst_known_integer,
        );
    }
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
}

fn emit_linear_int_div_mod_imm(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    src: u32,
    imm: i32,
    modulo: bool,
) {
    let lhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        loop_carried_values,
        src,
    );
    let rhs_val = builder.ins().iconst(types::I64, i64::from(imm));
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    if modulo {
        emit_integer_mod(
            builder,
            hits_var,
            current_hits,
            bail_block,
            dst_ptr,
            lhs_val,
            rhs_val,
            dst_known_integer,
        );
    } else {
        emit_integer_idiv(
            builder,
            hits_var,
            current_hits,
            bail_block,
            dst_ptr,
            lhs_val,
            rhs_val,
            dst_known_integer,
        );
    }
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
}

fn emit_linear_int_shift_op(
    builder: &mut FunctionBuilder<'_>,
    native_helpers: &NativeHelpers,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    lhs: u32,
    rhs: u32,
    shift_left: bool,
) {
    let lhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        loop_carried_values,
        lhs,
    );
    let rhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        loop_carried_values,
        rhs,
    );
    let call = if shift_left {
        builder.ins().call(native_helpers.shift_left, &[lhs_val, rhs_val])
    } else {
        builder.ins().call(native_helpers.shift_right, &[lhs_val, rhs_val])
    };
    let result = builder.inst_results(call)[0];
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
}

fn emit_linear_int_shift_imm_lhs(
    builder: &mut FunctionBuilder<'_>,
    native_helpers: &NativeHelpers,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    imm: i32,
    src: u32,
) {
    let lhs_val = builder.ins().iconst(types::I64, i64::from(imm));
    let rhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        loop_carried_values,
        src,
    );
    let call = builder.ins().call(native_helpers.shift_left, &[lhs_val, rhs_val]);
    let result = builder.inst_results(call)[0];
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
}

fn emit_linear_int_imm_shift_rhs(
    builder: &mut FunctionBuilder<'_>,
    native_helpers: &NativeHelpers,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    src: u32,
    imm: i32,
) {
    let lhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        loop_carried_values,
        src,
    );
    let rhs_val = builder.ins().iconst(types::I64, i64::from(imm));
    let call = builder.ins().call(native_helpers.shift_right, &[lhs_val, rhs_val]);
    let result = builder.inst_results(call)[0];
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
}

fn emit_binary_int_op<F>(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    known_integer_regs: &mut Vec<u32>,
    loop_carried_values: &[(u32, Value)],
    dst: u32,
    lhs: u32,
    rhs: u32,
    op: F,
) where
    F: Fn(&mut FunctionBuilder<'_>, Value, Value) -> Value,
{
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    let lhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        loop_carried_values,
        lhs,
    );
    let rhs_val = emit_known_linear_int_reg_value(
        builder,
        base_ptr,
        hits_var,
        current_hits,
        bail_block,
        known_integer_regs,
        loop_carried_values,
        rhs,
    );
    let result = op(builder, lhs_val, rhs_val);
    let dst_known_integer = linear_int_reg_is_known_integer(known_integer_regs, dst);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    mark_linear_int_reg_known_integer(known_integer_regs, dst);
}

fn emit_helper_success_guard(
    builder: &mut FunctionBuilder<'_>,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    success: Value,
) {
    let continue_block = builder.create_block();
    let ok = builder.ins().icmp_imm(IntCC::NotEqual, success, 0);
    builder.def_var(hits_var, current_hits);
    builder.ins().brif(ok, continue_block, &[], fallback_block, &[]);
    builder.switch_to_block(continue_block);
    builder.seal_block(continue_block);
}

fn exact_float_self_update_step(
    steps: &[NumericStep],
    lowered_trace: &LoweredTrace,
) -> Option<CarriedFloatLoopStep> {
    let (dst, lhs, rhs, op) = match steps {
        [NumericStep::Binary { dst, lhs, rhs, op }] => (*dst, *lhs, *rhs, *op),
        [
            NumericStep::Move { dst: alias_dst, src: alias_src },
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
    if dst != lhs_reg
        || !matches!(op, NumericBinaryOp::Add | NumericBinaryOp::Sub | NumericBinaryOp::Mul | NumericBinaryOp::Div)
    {
        return None;
    }
    let rhs = match rhs {
        NumericOperand::ImmI(imm) => CarriedFloatRhs::Imm(f64::from(imm)),
        NumericOperand::Const(index) => CarriedFloatRhs::Imm(
            lowered_trace
                .float_constant(index)
                .or_else(|| lowered_trace.integer_constant(index).map(f64::from))?,
        ),
        NumericOperand::Reg(rhs_reg) => {
            if rhs_reg == dst {
                return None;
            }
            match lowered_trace.entry_stable_register_value_kind(rhs_reg) {
                Some(TraceValueKind::Float) => CarriedFloatRhs::StableReg {
                    reg: rhs_reg,
                    kind: TraceValueKind::Float,
                },
                Some(TraceValueKind::Integer) => CarriedFloatRhs::StableReg {
                    reg: rhs_reg,
                    kind: TraceValueKind::Integer,
                },
                _ => return None,
            }
        }
    };
    Some(CarriedFloatLoopStep {
        reg: dst,
        op,
        rhs,
    })
}

fn carried_float_rhs_stable_reg(step: CarriedFloatLoopStep) -> Option<u32> {
    match step.rhs {
        CarriedFloatRhs::StableReg { reg, .. } => Some(reg),
        CarriedFloatRhs::Imm(_) => None,
    }
}

fn resolve_carried_float_rhs(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    step: CarriedFloatLoopStep,
) -> ResolvedCarriedFloatRhs {
    match step.rhs {
        CarriedFloatRhs::Imm(value) => ResolvedCarriedFloatRhs::Imm(value),
        CarriedFloatRhs::StableReg { reg, kind } => {
            let ptr = slot_addr(builder, base_ptr, reg);
            match kind {
                TraceValueKind::Float => {
                    emit_exact_tag_guard(
                        builder,
                        ptr,
                        LUA_VNUMFLT,
                        hits_var,
                        current_hits,
                        bail_block,
                    );
                    ResolvedCarriedFloatRhs::FloatRaw(
                        builder
                            .ins()
                            .load(types::I64, MemFlags::new(), ptr, LUA_VALUE_VALUE_OFFSET),
                    )
                }
                TraceValueKind::Integer => {
                    emit_exact_tag_guard(
                        builder,
                        ptr,
                        LUA_VNUMINT,
                        hits_var,
                        current_hits,
                        bail_block,
                    );
                    ResolvedCarriedFloatRhs::Integer(
                        builder
                            .ins()
                            .load(types::I64, MemFlags::new(), ptr, LUA_VALUE_VALUE_OFFSET),
                    )
                }
                _ => unreachable!(),
            }
        }
    }
}

fn hoisted_numeric_guard_value_from_carried_rhs(
    step: CarriedFloatLoopStep,
    rhs: ResolvedCarriedFloatRhs,
) -> Option<HoistedNumericGuardValue> {
    match (step.rhs, rhs) {
        (
            CarriedFloatRhs::StableReg {
                reg,
                kind: TraceValueKind::Float,
            },
            ResolvedCarriedFloatRhs::FloatRaw(raw),
        ) => Some(HoistedNumericGuardValue {
            reg,
            source: HoistedNumericGuardSource::FloatRaw(raw),
        }),
        (
            CarriedFloatRhs::StableReg {
                reg,
                kind: TraceValueKind::Integer,
            },
            ResolvedCarriedFloatRhs::Integer(value),
        ) => Some(HoistedNumericGuardValue {
            reg,
            source: HoistedNumericGuardSource::Integer(value),
        }),
        _ => None,
    }
}

fn emit_materialize_guard_numeric_override(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    reg: u32,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> bool {
    let dst_ptr = slot_addr(builder, abi.base_ptr, reg);
    let mem = MemFlags::new();

    if let Some(carried) = carried_float.filter(|carried| carried.reg == reg) {
        let raw = builder.use_var(carried.raw_var);
        let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
        builder.ins().store(mem, raw, dst_ptr, LUA_VALUE_VALUE_OFFSET);
        builder.ins().store(mem, float_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
        return true;
    }

    if let Some(hoisted) = hoisted_numeric.filter(|hoisted| hoisted.reg == reg) {
        match hoisted.source {
            HoistedNumericGuardSource::FloatRaw(raw) => {
                let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
                builder.ins().store(mem, raw, dst_ptr, LUA_VALUE_VALUE_OFFSET);
                builder.ins().store(mem, float_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
            }
            HoistedNumericGuardSource::Integer(value) => {
                emit_store_integer_with_known_tag(builder, dst_ptr, value, false);
            }
        }
        return true;
    }

    false
}

fn emit_guard_numeric_override_tag_and_value(
    builder: &mut FunctionBuilder<'_>,
    reg: u32,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<(Value, Value)> {
    if let Some(carried) = carried_float.filter(|carried| carried.reg == reg) {
        return Some((
            builder.ins().iconst(types::I8, LUA_VNUMFLT as i64),
            builder.use_var(carried.raw_var),
        ));
    }

    if let Some(hoisted) = hoisted_numeric.filter(|hoisted| hoisted.reg == reg) {
        return Some(match hoisted.source {
            HoistedNumericGuardSource::FloatRaw(raw) => (
                builder.ins().iconst(types::I8, LUA_VNUMFLT as i64),
                raw,
            ),
            HoistedNumericGuardSource::Integer(value) => (
                builder.ins().iconst(types::I8, LUA_VNUMINT as i64),
                value,
            ),
        });
    }

    None
}

fn emit_guard_numeric_override_integer_value(
    builder: &mut FunctionBuilder<'_>,
    reg: u32,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<Value> {
    hoisted_numeric
        .filter(|hoisted| hoisted.reg == reg)
        .and_then(|hoisted| match hoisted.source {
            HoistedNumericGuardSource::Integer(value) => Some(value),
            HoistedNumericGuardSource::FloatRaw(_) => {
                let _ = builder;
                None
            }
        })
}

fn emit_carried_float_loop_step(
    builder: &mut FunctionBuilder<'_>,
    carried_float_raw_var: Variable,
    step: CarriedFloatLoopStep,
    rhs: ResolvedCarriedFloatRhs,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
) {
    let carried_raw = builder.use_var(carried_float_raw_var);
    let lhs = builder.ins().bitcast(types::F64, MemFlags::new(), carried_raw);
    let rhs = match rhs {
        ResolvedCarriedFloatRhs::Imm(value) => {
            let rhs_raw = builder.ins().iconst(types::I64, value.to_bits() as i64);
            builder.ins().bitcast(types::F64, MemFlags::new(), rhs_raw)
        }
        ResolvedCarriedFloatRhs::FloatRaw(raw) => {
            builder.ins().bitcast(types::F64, MemFlags::new(), raw)
        }
        ResolvedCarriedFloatRhs::Integer(value) => builder.ins().fcvt_from_sint(types::F64, value),
    };
    let result = match step.op {
        NumericBinaryOp::Add => builder.ins().fadd(lhs, rhs),
        NumericBinaryOp::Sub => builder.ins().fsub(lhs, rhs),
        NumericBinaryOp::Mul => builder.ins().fmul(lhs, rhs),
        NumericBinaryOp::Div => builder.ins().fdiv(lhs, rhs),
        _ => unreachable!(),
    };
    let raw = builder.ins().bitcast(types::I64, MemFlags::new(), result);
    builder.def_var(carried_float_raw_var, raw);
    set_numeric_reg_value_kind(known_value_kinds, step.reg, TraceValueKind::Float);
}

fn native_supports_numeric_step(step: &NumericStep) -> bool {
    match step {
        NumericStep::Move { .. }
        | NumericStep::LoadBool { .. }
        | NumericStep::LoadI { .. }
        | NumericStep::LoadF { .. }
        | NumericStep::GetUpval { .. }
        | NumericStep::SetUpval { .. }
        | NumericStep::GetTableInt { .. }
        | NumericStep::SetTableInt { .. } => true,
        NumericStep::Binary { lhs, rhs, op, .. } => {
            native_supports_numeric_operand(lhs)
                && native_supports_numeric_operand(rhs)
                && matches!(
                    op,
                    NumericBinaryOp::Add
                        | NumericBinaryOp::Sub
                        | NumericBinaryOp::Mul
                        | NumericBinaryOp::Div
                        | NumericBinaryOp::IDiv
                        | NumericBinaryOp::Mod
                        | NumericBinaryOp::Pow
                        | NumericBinaryOp::BAnd
                        | NumericBinaryOp::BOr
                        | NumericBinaryOp::BXor
                        | NumericBinaryOp::Shl
                        | NumericBinaryOp::Shr
                )
        }
    }
}

fn native_supports_numeric_operand(operand: &NumericOperand) -> bool {
    matches!(operand, NumericOperand::Reg(_) | NumericOperand::ImmI(_) | NumericOperand::Const(_))
}

fn native_supports_numeric_cond(cond: NumericIfElseCond) -> bool {
    matches!(cond, NumericIfElseCond::RegCompare { .. } | NumericIfElseCond::Truthy { .. })
}

fn emit_numeric_step(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    step: NumericStep,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<()> {
    match step {
        NumericStep::Move { dst, src } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let src_kind = if let Some((src_tag, src_val)) =
                emit_guard_numeric_override_tag_and_value(builder, src, carried_float, hoisted_numeric)
            {
                let mem = MemFlags::new();
                builder.ins().store(mem, src_val, dst_ptr, LUA_VALUE_VALUE_OFFSET);
                builder.ins().store(mem, src_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
                match src_tag {
                    _ if carried_float.is_some_and(|carried| carried.reg == src) => TraceValueKind::Float,
                    _ => hoisted_numeric
                        .filter(|hoisted| hoisted.reg == src)
                        .map(|hoisted| match hoisted.source {
                            HoistedNumericGuardSource::FloatRaw(_) => TraceValueKind::Float,
                            HoistedNumericGuardSource::Integer(_) => TraceValueKind::Integer,
                        })
                        .unwrap_or(TraceValueKind::Unknown),
                }
            } else {
                let src_ptr = slot_addr(builder, abi.base_ptr, src);
                emit_copy_luavalue(builder, dst_ptr, src_ptr);
                numeric_reg_value_kind(known_value_kinds, src)
            };
            set_numeric_reg_value_kind(known_value_kinds, dst, src_kind);
            Some(())
        }
        NumericStep::LoadBool { dst, value } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let dst_known_boolean = matches!(
                numeric_reg_value_kind(known_value_kinds, dst),
                TraceValueKind::Boolean
            );
            emit_store_boolean_with_known_tag(builder, dst_ptr, value, dst_known_boolean);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Boolean);
            Some(())
        }
        NumericStep::LoadI { dst, imm } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let value = builder.ins().iconst(types::I64, i64::from(imm));
            let dst_known_integer = matches!(
                numeric_reg_value_kind(known_value_kinds, dst),
                TraceValueKind::Integer
            );
            emit_store_integer_with_known_tag(builder, dst_ptr, value, dst_known_integer);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Integer);
            Some(())
        }
        NumericStep::LoadF { dst, imm } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let dst_known_float = matches!(
                numeric_reg_value_kind(known_value_kinds, dst),
                TraceValueKind::Float
            );
            emit_store_float_with_known_tag(builder, dst_ptr, imm as f64, dst_known_float);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Float);
            Some(())
        }
        NumericStep::GetUpval { dst, upvalue } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let upvalue_index = builder.ins().iconst(abi.pointer_ty, i64::from(upvalue));
            let call = builder
                .ins()
                .call(native_helpers.get_upval, &[dst_ptr, abi.upvalue_ptrs, upvalue_index]);
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Unknown);
            Some(())
        }
        NumericStep::SetUpval { src, upvalue } => {
            let src_ptr = slot_addr(builder, abi.base_ptr, src);
            let upvalue_index = builder.ins().iconst(abi.pointer_ty, i64::from(upvalue));
            let call = builder.ins().call(
                native_helpers.set_upval,
                &[abi.lua_state_ptr, abi.upvalue_ptrs, src_ptr, upvalue_index],
            );
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            Some(())
        }
        NumericStep::GetTableInt { dst, table, index } => {
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let table_ptr = slot_addr(builder, abi.base_ptr, table);
            let index_ptr = slot_addr(builder, abi.base_ptr, index);
            let call = builder
                .ins()
                .call(native_helpers.get_table_int, &[dst_ptr, table_ptr, index_ptr]);
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
            Some(())
        }
        NumericStep::SetTableInt { table, index, value } => {
            let table_ptr = slot_addr(builder, abi.base_ptr, table);
            let index_ptr = slot_addr(builder, abi.base_ptr, index);
            let value_ptr = slot_addr(builder, abi.base_ptr, value);
            let call = builder.ins().call(
                native_helpers.set_table_int,
                &[abi.lua_state_ptr, table_ptr, index_ptr, value_ptr],
            );
            let success = builder.inst_results(call)[0];
            emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
            Some(())
        }
        NumericStep::Binary { dst, lhs, rhs, op } => {
            let dst_known_kind = numeric_reg_value_kind(known_value_kinds, dst);
            if matches!(op, NumericBinaryOp::Add | NumericBinaryOp::Sub | NumericBinaryOp::Mul) {
                emit_integer_add_sub_mul_with_helper_fallback(
                    builder,
                    abi,
                    native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    dst,
                    lhs,
                    rhs,
                    op,
                    known_value_kinds,
                    matches!(dst_known_kind, TraceValueKind::Integer),
                    matches!(dst_known_kind, TraceValueKind::Float),
                    carried_float,
                    hoisted_numeric,
                )?;
                set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
                return Some(());
            }

            if matches!(op, NumericBinaryOp::Div) {
                emit_numeric_div_with_helper_fallback(
                    builder,
                    abi,
                    native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    dst,
                    lhs,
                    rhs,
                    known_value_kinds,
                    matches!(dst_known_kind, TraceValueKind::Float),
                    carried_float,
                    hoisted_numeric,
                )?;
                set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
                return Some(());
            }

            if matches!(op, NumericBinaryOp::Pow) {
                emit_numeric_pow_with_helper_fallback(
                    builder,
                    abi,
                    native_helpers,
                    hits_var,
                    current_hits,
                    fallback_block,
                    dst,
                    lhs,
                    rhs,
                    known_value_kinds,
                    matches!(dst_known_kind, TraceValueKind::Float),
                    carried_float,
                    hoisted_numeric,
                )?;
                set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Numeric);
                return Some(());
            }

            if matches!(op, NumericBinaryOp::Mod | NumericBinaryOp::IDiv) {
                let lhs_val = emit_numeric_integer_operand(
                    builder,
                    abi,
                    hits_var,
                    current_hits,
                    fallback_block,
                    lhs,
                    known_value_kinds,
                    carried_float,
                    hoisted_numeric,
                )?;
                let rhs_val = emit_numeric_integer_operand(
                    builder,
                    abi,
                    hits_var,
                    current_hits,
                    fallback_block,
                    rhs,
                    known_value_kinds,
                    carried_float,
                    hoisted_numeric,
                )?;
                let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
                if matches!(op, NumericBinaryOp::Mod) {
                    emit_integer_mod(
                        builder,
                        hits_var,
                        current_hits,
                        fallback_block,
                        dst_ptr,
                        lhs_val,
                        rhs_val,
                        matches!(dst_known_kind, TraceValueKind::Integer),
                    );
                } else {
                    emit_integer_idiv(
                        builder,
                        hits_var,
                        current_hits,
                        fallback_block,
                        dst_ptr,
                        lhs_val,
                        rhs_val,
                        matches!(dst_known_kind, TraceValueKind::Integer),
                    );
                }
                set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Integer);
                return Some(());
            }

            if matches!(
                op,
                NumericBinaryOp::Add
                    | NumericBinaryOp::Sub
                    | NumericBinaryOp::Mul
                    | NumericBinaryOp::Div
                    | NumericBinaryOp::IDiv
                    | NumericBinaryOp::Mod
                    | NumericBinaryOp::Pow
            ) {
                let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
                let (lhs_kind, lhs_payload) = emit_numeric_operand_kind_and_payload(builder, lhs);
                let (rhs_kind, rhs_payload) = emit_numeric_operand_kind_and_payload(builder, rhs);
                let opcode = emit_numeric_binary_helper_opcode(builder, op)?;
                let call = builder.ins().call(
                    native_helpers.numeric_binary,
                    &[
                        dst_ptr,
                        abi.base_ptr,
                        abi.constants_ptr,
                        abi.constants_len,
                        lhs_kind,
                        lhs_payload,
                        rhs_kind,
                        rhs_payload,
                        opcode,
                    ],
                );
                let success = builder.inst_results(call)[0];
                emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
                return Some(());
            }

            let lhs_val = emit_numeric_integer_operand(
                builder,
                abi,
                hits_var,
                current_hits,
                fallback_block,
                lhs,
                known_value_kinds,
                carried_float,
                hoisted_numeric,
            )?;
            let rhs_val = emit_numeric_integer_operand(
                builder,
                abi,
                hits_var,
                current_hits,
                fallback_block,
                rhs,
                known_value_kinds,
                carried_float,
                hoisted_numeric,
            )?;
            let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
            let result = match op {
                NumericBinaryOp::Add => unreachable!(),
                NumericBinaryOp::Sub => unreachable!(),
                NumericBinaryOp::Mul => unreachable!(),
                NumericBinaryOp::BAnd => builder.ins().band(lhs_val, rhs_val),
                NumericBinaryOp::BOr => builder.ins().bor(lhs_val, rhs_val),
                NumericBinaryOp::BXor => builder.ins().bxor(lhs_val, rhs_val),
                NumericBinaryOp::Shl => {
                    let call = builder.ins().call(native_helpers.shift_left, &[lhs_val, rhs_val]);
                    builder.inst_results(call)[0]
                }
                NumericBinaryOp::Shr => {
                    let call = builder.ins().call(native_helpers.shift_right, &[lhs_val, rhs_val]);
                    builder.inst_results(call)[0]
                }
                NumericBinaryOp::Div
                | NumericBinaryOp::IDiv
                | NumericBinaryOp::Mod
                | NumericBinaryOp::Pow => unreachable!(),
            };
            emit_store_integer_with_known_tag(
                builder,
                dst_ptr,
                result,
                matches!(dst_known_kind, TraceValueKind::Integer),
            );
            set_numeric_reg_value_kind(known_value_kinds, dst, TraceValueKind::Integer);
            Some(())
        }
    }
}

fn emit_numeric_integer_operand(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    operand: NumericOperand,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<Value> {
    let mem = MemFlags::new();
    match operand {
        NumericOperand::ImmI(imm) => Some(builder.ins().iconst(types::I64, i64::from(imm))),
        NumericOperand::Reg(reg) => {
            if let Some(value) = emit_guard_numeric_override_integer_value(builder, reg, hoisted_numeric) {
                return Some(value);
            }
            let reg_ptr = slot_addr(builder, abi.base_ptr, reg);
            let _ = emit_materialize_guard_numeric_override(
                builder,
                abi,
                reg,
                carried_float,
                hoisted_numeric,
            );
            if !matches!(numeric_reg_value_kind(known_value_kinds, reg), TraceValueKind::Integer) {
                emit_integer_guard(builder, reg_ptr, hits_var, current_hits, fallback_block);
            }
            Some(builder.ins().load(types::I64, mem, reg_ptr, LUA_VALUE_VALUE_OFFSET))
        }
        NumericOperand::Const(index) => {
            let const_ptr = const_addr(builder, abi, hits_var, current_hits, fallback_block, index);
            emit_integer_guard(builder, const_ptr, hits_var, current_hits, fallback_block);
            Some(builder.ins().load(types::I64, mem, const_ptr, LUA_VALUE_VALUE_OFFSET))
        }
    }
}

fn emit_numeric_operand_tag_and_value(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    operand: NumericOperand,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<(Value, Value)> {
    let mem = MemFlags::new();
    match operand {
        NumericOperand::ImmI(imm) => Some((
            builder.ins().iconst(types::I8, LUA_VNUMINT as i64),
            builder.ins().iconst(types::I64, i64::from(imm)),
        )),
        NumericOperand::Reg(reg) => {
            if let Some(result) =
                emit_guard_numeric_override_tag_and_value(builder, reg, carried_float, hoisted_numeric)
            {
                return Some(result);
            }
            let reg_ptr = slot_addr(builder, abi.base_ptr, reg);
            let tag = if let Some(tag) = trace_value_kind_tag(numeric_reg_value_kind(known_value_kinds, reg)) {
                builder.ins().iconst(types::I8, i64::from(tag))
            } else {
                builder.ins().load(types::I8, mem, reg_ptr, LUA_VALUE_TT_OFFSET)
            };
            let value = builder.ins().load(types::I64, mem, reg_ptr, LUA_VALUE_VALUE_OFFSET);
            Some((tag, value))
        }
        NumericOperand::Const(index) => {
            let const_ptr = const_addr(builder, abi, hits_var, current_hits, fallback_block, index);
            let tag = builder.ins().load(types::I8, mem, const_ptr, LUA_VALUE_TT_OFFSET);
            let value = builder.ins().load(types::I64, mem, const_ptr, LUA_VALUE_VALUE_OFFSET);
            Some((tag, value))
        }
    }
}

fn emit_numeric_binary_helper_call(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst: u32,
    lhs: NumericOperand,
    rhs: NumericOperand,
    op: NumericBinaryOp,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<()> {
    if let NumericOperand::Reg(reg) = lhs {
        let _ = emit_materialize_guard_numeric_override(builder, abi, reg, carried_float, hoisted_numeric);
    }
    if let NumericOperand::Reg(reg) = rhs {
        let _ = emit_materialize_guard_numeric_override(builder, abi, reg, carried_float, hoisted_numeric);
    }
    let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
    let (lhs_kind, lhs_payload) = emit_numeric_operand_kind_and_payload(builder, lhs);
    let (rhs_kind, rhs_payload) = emit_numeric_operand_kind_and_payload(builder, rhs);
    let opcode = emit_numeric_binary_helper_opcode(builder, op)?;
    let call = builder.ins().call(
        native_helpers.numeric_binary,
        &[
            dst_ptr,
            abi.base_ptr,
            abi.constants_ptr,
            abi.constants_len,
            lhs_kind,
            lhs_payload,
            rhs_kind,
            rhs_payload,
            opcode,
        ],
    );
    let success = builder.inst_results(call)[0];
    emit_helper_success_guard(builder, hits_var, current_hits, fallback_block, success);
    Some(())
}

fn emit_integer_add_sub_mul_with_helper_fallback(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst: u32,
    lhs: NumericOperand,
    rhs: NumericOperand,
    op: NumericBinaryOp,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    dst_known_integer: bool,
    dst_known_float: bool,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<()> {
    let (lhs_tag, lhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        lhs,
        known_value_kinds,
        carried_float,
        hoisted_numeric,
    )?;
    let (rhs_tag, rhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        rhs,
        known_value_kinds,
        carried_float,
        hoisted_numeric,
    )?;
    let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
    let int_fast_block = builder.create_block();
    let numeric_fast_block = builder.create_block();
    let helper_block = builder.create_block();
    let done_block = builder.create_block();
    let float_store_block = builder.create_block();
    let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
    let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
    let lhs_is_int = builder.ins().icmp(IntCC::Equal, lhs_tag, int_tag);
    let rhs_is_int = builder.ins().icmp(IntCC::Equal, rhs_tag, int_tag);
    let both_int = builder.ins().band(lhs_is_int, rhs_is_int);
    let lhs_is_float = builder.ins().icmp(IntCC::Equal, lhs_tag, float_tag);
    let rhs_is_float = builder.ins().icmp(IntCC::Equal, rhs_tag, float_tag);
    let lhs_is_numeric = builder.ins().bor(lhs_is_int, lhs_is_float);
    let rhs_is_numeric = builder.ins().bor(rhs_is_int, rhs_is_float);
    let both_numeric = builder.ins().band(lhs_is_numeric, rhs_is_numeric);
    builder.def_var(hits_var, current_hits);
    builder.ins().brif(both_int, int_fast_block, &[], numeric_fast_block, &[]);

    builder.switch_to_block(int_fast_block);
    match op {
        NumericBinaryOp::Add => {
            let int_store_block = builder.create_block();
            let result = builder.ins().iadd(lhs_val, rhs_val);
            let lhs_xor_result = builder.ins().bxor(lhs_val, result);
            let rhs_xor_result = builder.ins().bxor(rhs_val, result);
            let overflow_bits = builder.ins().band(lhs_xor_result, rhs_xor_result);
            let overflow = builder.ins().icmp_imm(IntCC::SignedLessThan, overflow_bits, 0);
            builder.ins().brif(overflow, helper_block, &[], int_store_block, &[]);

            builder.switch_to_block(int_store_block);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            builder.ins().jump(done_block, &[]);
            builder.seal_block(int_store_block);
        }
        NumericBinaryOp::Sub => {
            let int_store_block = builder.create_block();
            let result = builder.ins().isub(lhs_val, rhs_val);
            let lhs_xor_rhs = builder.ins().bxor(lhs_val, rhs_val);
            let lhs_xor_result = builder.ins().bxor(lhs_val, result);
            let overflow_bits = builder.ins().band(lhs_xor_rhs, lhs_xor_result);
            let overflow = builder.ins().icmp_imm(IntCC::SignedLessThan, overflow_bits, 0);
            builder.ins().brif(overflow, helper_block, &[], int_store_block, &[]);

            builder.switch_to_block(int_store_block);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            builder.ins().jump(done_block, &[]);
            builder.seal_block(int_store_block);
        }
        NumericBinaryOp::Mul => {
            let zero = builder.ins().iconst(types::I64, 0);
            let neg_one = builder.ins().iconst(types::I64, -1);
            let lhs_is_zero = builder.ins().icmp(IntCC::Equal, lhs_val, zero);
            let rhs_is_zero = builder.ins().icmp(IntCC::Equal, rhs_val, zero);
            let either_zero = builder.ins().bor(lhs_is_zero, rhs_is_zero);
            let zero_block = builder.create_block();
            let nonzero_block = builder.create_block();
            let mul_store_block = builder.create_block();
            builder.ins().brif(either_zero, zero_block, &[], nonzero_block, &[]);

            builder.switch_to_block(zero_block);
            emit_store_integer_with_known_tag(builder, dst_ptr, zero, dst_known_integer);
            builder.ins().jump(done_block, &[]);
            builder.seal_block(zero_block);

            builder.switch_to_block(nonzero_block);
            let lhs_is_min = builder.ins().icmp_imm(IntCC::Equal, lhs_val, i64::MIN);
            let rhs_is_min = builder.ins().icmp_imm(IntCC::Equal, rhs_val, i64::MIN);
            let lhs_is_neg_one = builder.ins().icmp(IntCC::Equal, lhs_val, neg_one);
            let rhs_is_neg_one = builder.ins().icmp(IntCC::Equal, rhs_val, neg_one);
            let lhs_min_rhs_neg_one = builder.ins().band(lhs_is_min, rhs_is_neg_one);
            let rhs_min_lhs_neg_one = builder.ins().band(rhs_is_min, lhs_is_neg_one);
            let special_overflow = builder.ins().bor(lhs_min_rhs_neg_one, rhs_min_lhs_neg_one);
            let mul_compute_block = builder.create_block();
            builder.ins().brif(special_overflow, helper_block, &[], mul_compute_block, &[]);

            builder.switch_to_block(mul_compute_block);
            let result = builder.ins().imul(lhs_val, rhs_val);
            let quotient = builder.ins().sdiv(result, rhs_val);
            let overflow = builder.ins().icmp(IntCC::NotEqual, quotient, lhs_val);
            builder.ins().brif(overflow, helper_block, &[], mul_store_block, &[]);
            builder.seal_block(mul_compute_block);

            builder.switch_to_block(mul_store_block);
            emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
            builder.ins().jump(done_block, &[]);
            builder.seal_block(mul_store_block);
            builder.seal_block(nonzero_block);
        }
        _ => unreachable!(),
    }
    builder.seal_block(int_fast_block);

    builder.switch_to_block(numeric_fast_block);
    builder.ins().brif(both_numeric, float_store_block, &[], helper_block, &[]);
    builder.seal_block(numeric_fast_block);

    builder.switch_to_block(float_store_block);
    let lhs_num = emit_numeric_tagged_value_to_float(builder, lhs_tag, lhs_val);
    let rhs_num = emit_numeric_tagged_value_to_float(builder, rhs_tag, rhs_val);
    let result = match op {
        NumericBinaryOp::Add => builder.ins().fadd(lhs_num, rhs_num),
        NumericBinaryOp::Sub => builder.ins().fsub(lhs_num, rhs_num),
        NumericBinaryOp::Mul => builder.ins().fmul(lhs_num, rhs_num),
        _ => unreachable!(),
    };
    emit_store_float_value_with_known_tag(builder, dst_ptr, result, dst_known_float);
    builder.ins().jump(done_block, &[]);
    builder.seal_block(float_store_block);

    builder.switch_to_block(helper_block);
    emit_numeric_binary_helper_call(
        builder,
        abi,
        native_helpers,
        hits_var,
        current_hits,
        fallback_block,
        dst,
        lhs,
        rhs,
        op,
        carried_float,
        hoisted_numeric,
    )?;
    builder.ins().jump(done_block, &[]);
    builder.seal_block(helper_block);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);
    Some(())
}

fn emit_numeric_div_with_helper_fallback(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst: u32,
    lhs: NumericOperand,
    rhs: NumericOperand,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    dst_known_float: bool,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<()> {
    let (lhs_tag, lhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        lhs,
        known_value_kinds,
        carried_float,
        hoisted_numeric,
    )?;
    let (rhs_tag, rhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        rhs,
        known_value_kinds,
        carried_float,
        hoisted_numeric,
    )?;
    let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
    let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
    let lhs_is_int = builder.ins().icmp(IntCC::Equal, lhs_tag, int_tag);
    let lhs_is_float = builder.ins().icmp(IntCC::Equal, lhs_tag, float_tag);
    let rhs_is_int = builder.ins().icmp(IntCC::Equal, rhs_tag, int_tag);
    let rhs_is_float = builder.ins().icmp(IntCC::Equal, rhs_tag, float_tag);
    let lhs_is_numeric = builder.ins().bor(lhs_is_int, lhs_is_float);
    let rhs_is_numeric = builder.ins().bor(rhs_is_int, rhs_is_float);
    let both_numeric = builder.ins().band(lhs_is_numeric, rhs_is_numeric);
    let fast_block = builder.create_block();
    let helper_block = builder.create_block();
    let done_block = builder.create_block();
    let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
    builder.def_var(hits_var, current_hits);
    builder.ins().brif(both_numeric, fast_block, &[], helper_block, &[]);

    builder.switch_to_block(fast_block);
    let lhs_as_float_int = builder.ins().fcvt_from_sint(types::F64, lhs_val);
    let lhs_as_float_raw = builder.ins().bitcast(types::F64, MemFlags::new(), lhs_val);
    let lhs_as_float = builder.ins().select(lhs_is_int, lhs_as_float_int, lhs_as_float_raw);
    let rhs_as_float_int = builder.ins().fcvt_from_sint(types::F64, rhs_val);
    let rhs_as_float_raw = builder.ins().bitcast(types::F64, MemFlags::new(), rhs_val);
    let rhs_as_float = builder.ins().select(rhs_is_int, rhs_as_float_int, rhs_as_float_raw);
    let result = builder.ins().fdiv(lhs_as_float, rhs_as_float);
    emit_store_float_value_with_known_tag(builder, dst_ptr, result, dst_known_float);
    builder.ins().jump(done_block, &[]);
    builder.seal_block(fast_block);

    builder.switch_to_block(helper_block);
    emit_numeric_binary_helper_call(
        builder,
        abi,
        native_helpers,
        hits_var,
        current_hits,
        fallback_block,
        dst,
        lhs,
        rhs,
        NumericBinaryOp::Div,
        carried_float,
        hoisted_numeric,
    )?;
    builder.ins().jump(done_block, &[]);
    builder.seal_block(helper_block);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);
    Some(())
}

fn emit_numeric_pow_with_helper_fallback(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst: u32,
    lhs: NumericOperand,
    rhs: NumericOperand,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    dst_known_float: bool,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<()> {
    let (lhs_tag, lhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        lhs,
        known_value_kinds,
        carried_float,
        hoisted_numeric,
    )?;
    let (rhs_tag, rhs_val) = emit_numeric_operand_tag_and_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        rhs,
        known_value_kinds,
        carried_float,
        hoisted_numeric,
    )?;
    let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
    let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
    let lhs_is_int = builder.ins().icmp(IntCC::Equal, lhs_tag, int_tag);
    let lhs_is_float = builder.ins().icmp(IntCC::Equal, lhs_tag, float_tag);
    let rhs_is_int = builder.ins().icmp(IntCC::Equal, rhs_tag, int_tag);
    let rhs_is_float = builder.ins().icmp(IntCC::Equal, rhs_tag, float_tag);
    let lhs_is_numeric = builder.ins().bor(lhs_is_int, lhs_is_float);
    let rhs_is_numeric = builder.ins().bor(rhs_is_int, rhs_is_float);
    let both_numeric = builder.ins().band(lhs_is_numeric, rhs_is_numeric);
    let fast_block = builder.create_block();
    let helper_block = builder.create_block();
    let done_block = builder.create_block();
    let dst_ptr = slot_addr(builder, abi.base_ptr, dst);
    builder.def_var(hits_var, current_hits);
    builder.ins().brif(both_numeric, fast_block, &[], helper_block, &[]);

    builder.switch_to_block(fast_block);
    let lhs_num = emit_numeric_tagged_value_to_float(builder, lhs_tag, lhs_val);
    let rhs_num = emit_numeric_tagged_value_to_float(builder, rhs_tag, rhs_val);
    let call = builder.ins().call(native_helpers.numeric_pow, &[lhs_num, rhs_num]);
    let result = builder.inst_results(call)[0];
    emit_store_float_value_with_known_tag(builder, dst_ptr, result, dst_known_float);
    builder.ins().jump(done_block, &[]);
    builder.seal_block(fast_block);

    builder.switch_to_block(helper_block);
    emit_numeric_binary_helper_call(
        builder,
        abi,
        native_helpers,
        hits_var,
        current_hits,
        fallback_block,
        dst,
        lhs,
        rhs,
        NumericBinaryOp::Pow,
        carried_float,
        hoisted_numeric,
    )?;
    builder.ins().jump(done_block, &[]);
    builder.seal_block(helper_block);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);
    Some(())
}

fn emit_integer_mod(
    builder: &mut FunctionBuilder<'_>,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst_ptr: Value,
    lhs_val: Value,
    rhs_val: Value,
    dst_known_integer: bool,
) {
    let zero = builder.ins().iconst(types::I64, 0);
    let rhs_is_zero = builder.ins().icmp(IntCC::Equal, rhs_val, zero);
    let compute_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    builder.ins().brif(rhs_is_zero, fallback_block, &[], compute_block, &[]);

    builder.switch_to_block(compute_block);
    builder.seal_block(compute_block);
    let remainder = builder.ins().srem(lhs_val, rhs_val);
    let rem_is_zero = builder.ins().icmp(IntCC::Equal, remainder, zero);
    let xor = builder.ins().bxor(remainder, rhs_val);
    let sign_diff = builder.ins().icmp_imm(IntCC::SignedLessThan, xor, 0);
    let adjusted = builder.ins().iadd(remainder, rhs_val);
    let maybe_adjusted = builder.ins().select(sign_diff, adjusted, remainder);
    let result = builder.ins().select(rem_is_zero, remainder, maybe_adjusted);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
}

fn emit_integer_idiv(
    builder: &mut FunctionBuilder<'_>,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    dst_ptr: Value,
    lhs_val: Value,
    rhs_val: Value,
    dst_known_integer: bool,
) {
    let zero = builder.ins().iconst(types::I64, 0);
    let neg_one = builder.ins().iconst(types::I64, -1);
    let rhs_is_zero = builder.ins().icmp(IntCC::Equal, rhs_val, zero);
    let neg_one_block = builder.create_block();
    let compute_block = builder.create_block();
    let normal_block = builder.create_block();
    let done_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    builder.ins().brif(rhs_is_zero, fallback_block, &[], compute_block, &[]);

    builder.switch_to_block(compute_block);
    builder.seal_block(compute_block);
    let rhs_is_neg_one = builder.ins().icmp(IntCC::Equal, rhs_val, neg_one);
    builder.def_var(hits_var, current_hits);
    builder.ins().brif(rhs_is_neg_one, neg_one_block, &[], normal_block, &[]);

    builder.switch_to_block(neg_one_block);
    builder.seal_block(neg_one_block);
    let negated = builder.ins().ineg(lhs_val);
    emit_store_integer_with_known_tag(builder, dst_ptr, negated, dst_known_integer);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(normal_block);
    builder.seal_block(normal_block);
    let quotient = builder.ins().sdiv(lhs_val, rhs_val);
    let remainder = builder.ins().srem(lhs_val, rhs_val);
    let rem_is_zero = builder.ins().icmp(IntCC::Equal, remainder, zero);
    let xor = builder.ins().bxor(lhs_val, rhs_val);
    let sign_diff = builder.ins().icmp_imm(IntCC::SignedLessThan, xor, 0);
    let floor_adjust = builder.ins().iadd_imm(quotient, -1);
    let adjusted = builder.ins().select(sign_diff, floor_adjust, quotient);
    let result = builder.ins().select(rem_is_zero, quotient, adjusted);
    emit_store_integer_with_known_tag(builder, dst_ptr, result, dst_known_integer);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(done_block);
    builder.seal_block(done_block);
}

fn emit_numeric_condition_value(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    cond: NumericIfElseCond,
    known_value_kinds: &[crate::lua_vm::jit::lowering::RegisterValueHint],
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<Value> {
    let mem = MemFlags::new();
    match cond {
        NumericIfElseCond::RegCompare { op, lhs, rhs } => {
            let lhs_ptr = slot_addr(builder, abi.base_ptr, lhs);
            let rhs_ptr = slot_addr(builder, abi.base_ptr, rhs);
            let lhs_tag = if carried_float.is_some_and(|carried| carried.reg == lhs) {
                builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
            } else if let Some(hoisted) = hoisted_numeric.filter(|hoisted| hoisted.reg == lhs) {
                match hoisted.source {
                    HoistedNumericGuardSource::FloatRaw(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
                    }
                    HoistedNumericGuardSource::Integer(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMINT as i64)
                    }
                }
            } else if let Some(tag) = trace_value_kind_tag(numeric_reg_value_kind(known_value_kinds, lhs)) {
                builder.ins().iconst(types::I8, i64::from(tag))
            } else {
                builder.ins().load(types::I8, mem, lhs_ptr, LUA_VALUE_TT_OFFSET)
            };
            let rhs_tag = if carried_float.is_some_and(|carried| carried.reg == rhs) {
                builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
            } else if let Some(hoisted) = hoisted_numeric.filter(|hoisted| hoisted.reg == rhs) {
                match hoisted.source {
                    HoistedNumericGuardSource::FloatRaw(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
                    }
                    HoistedNumericGuardSource::Integer(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMINT as i64)
                    }
                }
            } else if let Some(tag) = trace_value_kind_tag(numeric_reg_value_kind(known_value_kinds, rhs)) {
                builder.ins().iconst(types::I8, i64::from(tag))
            } else {
                builder.ins().load(types::I8, mem, rhs_ptr, LUA_VALUE_TT_OFFSET)
            };
            let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
            let float_tag = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
            let lhs_is_int = builder.ins().icmp(IntCC::Equal, lhs_tag, int_tag);
            let lhs_is_float = builder.ins().icmp(IntCC::Equal, lhs_tag, float_tag);
            let rhs_is_int = builder.ins().icmp(IntCC::Equal, rhs_tag, int_tag);
            let rhs_is_float = builder.ins().icmp(IntCC::Equal, rhs_tag, float_tag);
            let lhs_is_numeric = builder.ins().bor(lhs_is_int, lhs_is_float);
            let rhs_is_numeric = builder.ins().bor(rhs_is_int, rhs_is_float);
            let both_numeric = builder.ins().band(lhs_is_numeric, rhs_is_numeric);
            let compare_block = builder.create_block();
            builder.def_var(hits_var, current_hits);
            builder.ins().brif(both_numeric, compare_block, &[], fallback_block, &[]);
            builder.switch_to_block(compare_block);
            builder.seal_block(compare_block);

            let lhs_val = if let Some(carried) = carried_float.filter(|carried| carried.reg == lhs) {
                builder.use_var(carried.raw_var)
            } else if let Some(hoisted) = hoisted_numeric.filter(|hoisted| hoisted.reg == lhs) {
                match hoisted.source {
                    HoistedNumericGuardSource::FloatRaw(raw) => raw,
                    HoistedNumericGuardSource::Integer(value) => value,
                }
            } else {
                builder.ins().load(types::I64, mem, lhs_ptr, LUA_VALUE_VALUE_OFFSET)
            };
            let rhs_val = if let Some(carried) = carried_float.filter(|carried| carried.reg == rhs) {
                builder.use_var(carried.raw_var)
            } else if let Some(hoisted) = hoisted_numeric.filter(|hoisted| hoisted.reg == rhs) {
                match hoisted.source {
                    HoistedNumericGuardSource::FloatRaw(raw) => raw,
                    HoistedNumericGuardSource::Integer(value) => value,
                }
            } else {
                builder.ins().load(types::I64, mem, rhs_ptr, LUA_VALUE_VALUE_OFFSET)
            };
            let lhs_num = emit_numeric_tagged_value_to_float(builder, lhs_tag, lhs_val);
            let rhs_num = emit_numeric_tagged_value_to_float(builder, rhs_tag, rhs_val);
            Some(emit_numeric_compare(builder, lhs_num, rhs_num, op))
        }
        NumericIfElseCond::Truthy { reg } => {
            let reg_ptr = slot_addr(builder, abi.base_ptr, reg);
            let tag = if carried_float.is_some_and(|carried| carried.reg == reg) {
                builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
            } else if let Some(hoisted) = hoisted_numeric.filter(|hoisted| hoisted.reg == reg) {
                match hoisted.source {
                    HoistedNumericGuardSource::FloatRaw(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMFLT as i64)
                    }
                    HoistedNumericGuardSource::Integer(_) => {
                        builder.ins().iconst(types::I8, LUA_VNUMINT as i64)
                    }
                }
            } else {
                builder.ins().load(types::I8, mem, reg_ptr, LUA_VALUE_TT_OFFSET)
            };
            let is_nil = builder.ins().icmp_imm(IntCC::Equal, tag, LUA_VNIL_TAG as i64);
            let is_false = builder.ins().icmp_imm(IntCC::Equal, tag, LUA_VFALSE_TAG as i64);
            let is_falsey = builder.ins().bor(is_nil, is_false);
            Some(builder.ins().bnot(is_falsey))
        }
    }
}

fn emit_linear_compare(
    builder: &mut FunctionBuilder<'_>,
    lhs: Value,
    rhs: Value,
    op: LinearIntGuardOp,
) -> Value {
    match op {
        LinearIntGuardOp::Eq => builder.ins().icmp(IntCC::Equal, lhs, rhs),
        LinearIntGuardOp::Lt => builder.ins().icmp(IntCC::SignedLessThan, lhs, rhs),
        LinearIntGuardOp::Le => builder.ins().icmp(IntCC::SignedLessThanOrEqual, lhs, rhs),
        LinearIntGuardOp::Gt => builder.ins().icmp(IntCC::SignedGreaterThan, lhs, rhs),
        LinearIntGuardOp::Ge => builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, lhs, rhs),
    }
}

fn emit_numeric_compare(
    builder: &mut FunctionBuilder<'_>,
    lhs: Value,
    rhs: Value,
    op: LinearIntGuardOp,
) -> Value {
    match op {
        LinearIntGuardOp::Eq => builder.ins().fcmp(FloatCC::Equal, lhs, rhs),
        LinearIntGuardOp::Lt => builder.ins().fcmp(FloatCC::LessThan, lhs, rhs),
        LinearIntGuardOp::Le => builder.ins().fcmp(FloatCC::LessThanOrEqual, lhs, rhs),
        LinearIntGuardOp::Gt => builder.ins().fcmp(FloatCC::GreaterThan, lhs, rhs),
        LinearIntGuardOp::Ge => builder.ins().fcmp(FloatCC::GreaterThanOrEqual, lhs, rhs),
    }
}

fn emit_numeric_guard_flow(
    builder: &mut FunctionBuilder<'_>,
    abi: &NativeAbi,
    native_helpers: &NativeHelpers,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    cond: NumericIfElseCond,
    continue_when: bool,
    continue_preset: Option<&NumericStep>,
    exit_preset: Option<&NumericStep>,
    continue_block: Block,
    exit_block: Block,
    known_value_kinds: &mut Vec<crate::lua_vm::jit::lowering::RegisterValueHint>,
    carried_float: Option<CarriedFloatGuardValue>,
    hoisted_numeric: Option<HoistedNumericGuardValue>,
) -> Option<()> {
    let cond_value = emit_numeric_condition_value(
        builder,
        abi,
        hits_var,
        current_hits,
        fallback_block,
        cond,
        known_value_kinds,
        carried_float,
        hoisted_numeric,
    )?;

    let hold_block = builder.create_block();
    let fail_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    if continue_when {
        builder.ins().brif(cond_value, hold_block, &[], fail_block, &[]);
    } else {
        builder.ins().brif(cond_value, fail_block, &[], hold_block, &[]);
    }

    builder.switch_to_block(hold_block);
    if let Some(step) = continue_preset {
        emit_numeric_step(
            builder,
            abi,
            native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            *step,
            known_value_kinds,
            carried_float,
            hoisted_numeric,
        )?;
    }
    builder.def_var(hits_var, current_hits);
    builder.ins().jump(continue_block, &[]);
    builder.seal_block(hold_block);

    builder.switch_to_block(fail_block);
    if let Some(step) = exit_preset {
        emit_numeric_step(
            builder,
            abi,
            native_helpers,
            hits_var,
            current_hits,
            fallback_block,
            *step,
            known_value_kinds,
            carried_float,
            hoisted_numeric,
        )?;
    }
    builder.def_var(hits_var, current_hits);
    builder.ins().jump(exit_block, &[]);
    builder.seal_block(fail_block);
    Some(())
}
