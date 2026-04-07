use cranelift::codegen::settings;
use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

use crate::LuaValue;
use crate::lua_value::LUA_VNUMINT;
use crate::lua_vm::jit::helper_plan::HelperPlan;
use crate::lua_vm::jit::ir::TraceIr;
use crate::lua_vm::jit::lowering::LoweredTrace;
use crate::lua_vm::jit::trace_recorder::TraceArtifact;

use super::compile::compile_executor;
use super::{
    BackendCompileOutcome, CompiledTrace, CompiledTraceExecution, CompiledTraceExecutor,
    LinearIntGuardOp, LinearIntLoopGuard, LinearIntStep, NativeCompiledTrace,
    NullTraceBackend, TraceBackend,
};

const LUA_VALUE_SIZE: i64 = std::mem::size_of::<LuaValue>() as i64;
const LUA_VALUE_TT_OFFSET: i32 = std::mem::offset_of!(LuaValue, tt) as i32;
const LUA_VALUE_VALUE_OFFSET: i32 = std::mem::offset_of!(LuaValue, value) as i32;

pub(crate) struct NativeTraceBackend {
    fallback: NullTraceBackend,
    module: Option<JITModule>,
    next_function_index: u64,
}

impl Default for NativeTraceBackend {
    fn default() -> Self {
        Self {
            fallback: NullTraceBackend,
            module: Self::build_module().ok(),
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
        let execution = match compile_executor(artifact, ir, lowered_trace) {
            Some(CompiledTraceExecutor::LinearIntForLoop { loop_reg, steps }) => self
                .compile_native_linear_int_for_loop(loop_reg, &steps)
                .map(CompiledTraceExecution::Native)
                .unwrap_or_else(|| {
                    CompiledTraceExecution::Interpreter(CompiledTraceExecutor::LinearIntForLoop {
                        loop_reg,
                        steps,
                    })
                }),
            Some(CompiledTraceExecutor::LinearIntJmpLoop { steps, guard }) => self
                .compile_native_linear_int_jmp_loop(&steps, guard)
                .map(CompiledTraceExecution::Native)
                .unwrap_or_else(|| {
                    CompiledTraceExecution::Interpreter(CompiledTraceExecutor::LinearIntJmpLoop {
                        steps,
                        guard,
                    })
                }),
            Some(executor) => CompiledTraceExecution::Interpreter(executor),
            None => CompiledTraceExecution::LoweredOnly,
        };

        match CompiledTrace::from_artifact_helper_plan_with_execution(
            artifact,
            ir,
            lowered_trace,
            helper_plan,
            execution,
        ) {
            Some(compiled_trace) => BackendCompileOutcome::Compiled(compiled_trace),
            None => self.fallback.compile(artifact, ir, lowered_trace, helper_plan),
        }
    }
}

impl NativeTraceBackend {
    fn build_module() -> Result<JITModule, String> {
        let mut flags = settings::builder();
        let _ = flags.set("opt_level", "speed");
        let isa = cranelift_native::builder()
            .map_err(|err| err.to_string())?
            .finish(settings::Flags::new(flags))
            .map_err(|err| err.to_string())?;
        let builder = JITBuilder::with_isa(isa, default_libcall_names());
        Ok(JITModule::new(builder))
    }

    fn compile_native_linear_int_for_loop(
        &mut self,
        loop_reg: u32,
        steps: &[LinearIntStep],
    ) -> Option<NativeCompiledTrace> {
        debug_assert_eq!(std::mem::size_of::<LuaValue>(), 16);

        let module = self.module.as_mut()?;
        let pointer_ty = module.target_config().pointer_type();

        let mut context = module.make_context();
        context.func.signature.params.push(AbiParam::new(pointer_ty));
        context.func.signature.params.push(AbiParam::new(pointer_ty));
        context.func.signature.returns.push(AbiParam::new(types::I64));

        let func_name = format!("jit_native_linear_int_for_loop_{}", self.next_function_index);
        self.next_function_index = self.next_function_index.saturating_add(1);

        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;

        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);

        let entry_block = builder.create_block();
        let loop_block = builder.create_block();
        let bail_block = builder.create_block();
        let complete_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);

        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let params = builder.block_params(entry_block).to_vec();
        let stack_ptr = params[0];
        let base_slots = params[1];
        let slot_scale = builder.ins().iconst(pointer_ty, LUA_VALUE_SIZE);
        let base_bytes = builder.ins().imul(base_slots, slot_scale);
        let base_ptr = builder.ins().iadd(stack_ptr, base_bytes);
        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        builder.ins().jump(loop_block, &[]);

        builder.switch_to_block(loop_block);
        let current_hits = builder.use_var(hits_var);

        for step in steps {
            emit_linear_int_step(
                &mut builder,
                base_ptr,
                hits_var,
                current_hits,
                bail_block,
                *step,
            );
        }

        let one = builder.ins().iconst(types::I64, 1);
        let next_hits = builder.ins().iadd(current_hits, one);

        let loop_ptr = slot_addr(&mut builder, base_ptr, loop_reg);
        let step_ptr = slot_addr(&mut builder, base_ptr, loop_reg.saturating_add(1));
        let index_ptr = slot_addr(&mut builder, base_ptr, loop_reg.saturating_add(2));
        emit_integer_guard(&mut builder, loop_ptr, hits_var, current_hits, bail_block);
        emit_integer_guard(&mut builder, step_ptr, hits_var, current_hits, bail_block);
        emit_integer_guard(&mut builder, index_ptr, hits_var, current_hits, bail_block);

        let mem = MemFlags::new();
        let remaining = builder
            .ins()
            .load(types::I64, mem, loop_ptr, LUA_VALUE_VALUE_OFFSET);
        let has_more = builder
            .ins()
            .icmp_imm(IntCC::UnsignedGreaterThan, remaining, 0);
        let continue_block = builder.create_block();
        builder.def_var(hits_var, next_hits);
        builder.ins().brif(
            has_more,
            continue_block,
            &[],
            complete_block,
            &[],
        );

        builder.switch_to_block(continue_block);
        let step_val = builder
            .ins()
            .load(types::I64, mem, step_ptr, LUA_VALUE_VALUE_OFFSET);
        let index_val = builder
            .ins()
            .load(types::I64, mem, index_ptr, LUA_VALUE_VALUE_OFFSET);
        let updated_remaining = builder.ins().iadd_imm(remaining, -1);
        let updated_index = builder.ins().iadd(index_val, step_val);
        let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
        builder
            .ins()
            .store(mem, updated_remaining, loop_ptr, LUA_VALUE_VALUE_OFFSET);
        builder.ins().store(mem, int_tag, loop_ptr, LUA_VALUE_TT_OFFSET);
        builder
            .ins()
            .store(mem, updated_index, index_ptr, LUA_VALUE_VALUE_OFFSET);
        builder.ins().store(mem, int_tag, index_ptr, LUA_VALUE_TT_OFFSET);
        builder.ins().jump(loop_block, &[]);

        builder.switch_to_block(complete_block);
        let completed_hits = builder.use_var(hits_var);
        let completed = encode_trace_result(&mut builder, completed_hits, true);
        builder.ins().return_(&[completed]);

        builder.switch_to_block(bail_block);
        let bailed_hits = builder.use_var(hits_var);
        let bailed = encode_trace_result(&mut builder, bailed_hits, false);
        builder.ins().return_(&[bailed]);

        builder.seal_block(loop_block);
        builder.seal_block(continue_block);
        builder.seal_block(complete_block);
        builder.seal_block(bail_block);
        builder.finalize();

        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);

        Some(NativeCompiledTrace::LinearIntForLoop {
            entry: unsafe { std::mem::transmute(entry) },
        })
    }

    fn compile_native_linear_int_jmp_loop(
        &mut self,
        steps: &[LinearIntStep],
        guard: LinearIntLoopGuard,
    ) -> Option<NativeCompiledTrace> {
        let module = self.module.as_mut()?;
        let pointer_ty = module.target_config().pointer_type();

        let mut context = module.make_context();
        context.func.signature.params.push(AbiParam::new(pointer_ty));
        context.func.signature.params.push(AbiParam::new(pointer_ty));
        context.func.signature.returns.push(AbiParam::new(types::I64));

        let func_name = format!("jit_native_linear_int_jmp_loop_{}", self.next_function_index);
        self.next_function_index = self.next_function_index.saturating_add(1);

        let func_id = module
            .declare_function(&func_name, Linkage::Local, &context.func.signature)
            .ok()?;

        let mut builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_ctx);
        let hits_var = builder.declare_var(types::I64);

        let entry_block = builder.create_block();
        let loop_block = builder.create_block();
        let fallback_block = builder.create_block();
        let exit_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);

        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let params = builder.block_params(entry_block).to_vec();
        let stack_ptr = params[0];
        let base_slots = params[1];
        let slot_scale = builder.ins().iconst(pointer_ty, LUA_VALUE_SIZE);
        let base_bytes = builder.ins().imul(base_slots, slot_scale);
        let base_ptr = builder.ins().iadd(stack_ptr, base_bytes);
        let zero_hits = builder.ins().iconst(types::I64, 0);
        builder.def_var(hits_var, zero_hits);
        builder.ins().jump(loop_block, &[]);

        builder.switch_to_block(loop_block);
        let current_hits = builder.use_var(hits_var);

        if matches!(
            guard,
            LinearIntLoopGuard::HeadRegReg { .. } | LinearIntLoopGuard::HeadRegImm { .. }
        ) {
            let continue_block = builder.create_block();
            emit_linear_int_loop_guard(
                &mut builder,
                base_ptr,
                hits_var,
                current_hits,
                fallback_block,
                exit_block,
                continue_block,
                guard,
            );
            builder.switch_to_block(continue_block);
            builder.seal_block(continue_block);
        }

        for step in steps {
            emit_linear_int_step(
                &mut builder,
                base_ptr,
                hits_var,
                current_hits,
                fallback_block,
                *step,
            );
        }

        let one = builder.ins().iconst(types::I64, 1);
        let next_hits = builder.ins().iadd(current_hits, one);
        builder.def_var(hits_var, next_hits);

        if matches!(
            guard,
            LinearIntLoopGuard::TailRegReg { .. } | LinearIntLoopGuard::TailRegImm { .. }
        ) {
            let continue_block = builder.create_block();
            emit_linear_int_loop_guard(
                &mut builder,
                base_ptr,
                hits_var,
                next_hits,
                fallback_block,
                exit_block,
                continue_block,
                guard,
            );
            builder.switch_to_block(continue_block);
            builder.seal_block(continue_block);
        }

        builder.ins().jump(loop_block, &[]);

        builder.switch_to_block(exit_block);
        let exited_hits = builder.use_var(hits_var);
        let exited = encode_trace_result(&mut builder, exited_hits, true);
        builder.ins().return_(&[exited]);

        builder.switch_to_block(fallback_block);
        let fallback_hits = builder.use_var(hits_var);
        let fallback = encode_trace_result(&mut builder, fallback_hits, false);
        builder.ins().return_(&[fallback]);

        builder.seal_block(loop_block);
        builder.seal_block(exit_block);
        builder.seal_block(fallback_block);
        builder.finalize();

        module.define_function(func_id, &mut context).ok()?;
        module.clear_context(&mut context);
        module.finalize_definitions().ok()?;
        let entry = module.get_finalized_function(func_id);
        let exit_pc = match guard {
            LinearIntLoopGuard::HeadRegReg { exit_pc, .. }
            | LinearIntLoopGuard::HeadRegImm { exit_pc, .. }
            | LinearIntLoopGuard::TailRegReg { exit_pc, .. }
            | LinearIntLoopGuard::TailRegImm { exit_pc, .. } => exit_pc,
        };

        Some(NativeCompiledTrace::LinearIntJmpLoop {
            entry: unsafe { std::mem::transmute(entry) },
            exit_pc,
        })
    }
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
}

fn slot_addr(builder: &mut FunctionBuilder<'_>, base_ptr: Value, reg: u32) -> Value {
    builder
        .ins()
        .iadd_imm(base_ptr, i64::from(reg).saturating_mul(LUA_VALUE_SIZE))
}

fn emit_integer_guard(
    builder: &mut FunctionBuilder<'_>,
    slot_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
) {
    let mem = MemFlags::new();
    let tt = builder.ins().load(types::I8, mem, slot_ptr, LUA_VALUE_TT_OFFSET);
    let is_integer = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMINT as i64);
    let next_block = builder.create_block();
    builder.def_var(hits_var, current_hits);
    builder
        .ins()
        .brif(is_integer, next_block, &[], bail_block, &[]);
    builder.switch_to_block(next_block);
    builder.seal_block(next_block);
}

fn emit_linear_int_loop_guard(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    fallback_block: Block,
    exit_block: Block,
    continue_block: Block,
    guard: LinearIntLoopGuard,
) {
    let mem = MemFlags::new();
    let (op, lhs_val, rhs_val, continue_when) = match guard {
        LinearIntLoopGuard::HeadRegReg { op, lhs, rhs, continue_when, .. }
        | LinearIntLoopGuard::TailRegReg { op, lhs, rhs, continue_when, .. } => {
            let lhs_ptr = slot_addr(builder, base_ptr, lhs);
            let rhs_ptr = slot_addr(builder, base_ptr, rhs);
            emit_integer_guard(builder, lhs_ptr, hits_var, current_hits, fallback_block);
            emit_integer_guard(builder, rhs_ptr, hits_var, current_hits, fallback_block);
            let lhs_val = builder.ins().load(types::I64, mem, lhs_ptr, LUA_VALUE_VALUE_OFFSET);
            let rhs_val = builder.ins().load(types::I64, mem, rhs_ptr, LUA_VALUE_VALUE_OFFSET);
            (op, lhs_val, rhs_val, continue_when)
        }
        LinearIntLoopGuard::HeadRegImm { op, reg, imm, continue_when, .. }
        | LinearIntLoopGuard::TailRegImm { op, reg, imm, continue_when, .. } => {
            let reg_ptr = slot_addr(builder, base_ptr, reg);
            emit_integer_guard(builder, reg_ptr, hits_var, current_hits, fallback_block);
            let lhs_val = builder.ins().load(types::I64, mem, reg_ptr, LUA_VALUE_VALUE_OFFSET);
            let rhs_val = builder.ins().iconst(types::I64, i64::from(imm));
            (op, lhs_val, rhs_val, continue_when)
        }
    };

    let cond = match op {
        LinearIntGuardOp::Eq => builder.ins().icmp(IntCC::Equal, lhs_val, rhs_val),
        LinearIntGuardOp::Lt => builder.ins().icmp(IntCC::SignedLessThan, lhs_val, rhs_val),
        LinearIntGuardOp::Le => builder.ins().icmp(IntCC::SignedLessThanOrEqual, lhs_val, rhs_val),
        LinearIntGuardOp::Gt => builder.ins().icmp(IntCC::SignedGreaterThan, lhs_val, rhs_val),
        LinearIntGuardOp::Ge => builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, lhs_val, rhs_val),
    };

    builder.def_var(hits_var, current_hits);
    if continue_when {
        builder.ins().brif(cond, continue_block, &[], exit_block, &[]);
    } else {
        builder.ins().brif(cond, exit_block, &[], continue_block, &[]);
    }
}

fn emit_linear_int_step(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    step: LinearIntStep,
) {
    let mem = MemFlags::new();

    match step {
        LinearIntStep::Move { dst, src } => {
            let src_ptr = slot_addr(builder, base_ptr, src);
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            emit_integer_guard(builder, src_ptr, hits_var, current_hits, bail_block);
            let src_val = builder.ins().load(types::I64, mem, src_ptr, LUA_VALUE_VALUE_OFFSET);
            let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
            builder.ins().store(mem, src_val, dst_ptr, LUA_VALUE_VALUE_OFFSET);
            builder.ins().store(mem, int_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
        }
        LinearIntStep::LoadI { dst, imm } => {
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            let dst_val = builder.ins().iconst(types::I64, i64::from(imm));
            let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
            builder.ins().store(mem, dst_val, dst_ptr, LUA_VALUE_VALUE_OFFSET);
            builder.ins().store(mem, int_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
        }
        LinearIntStep::Add { dst, lhs, rhs } => {
            emit_binary_int_op(builder, base_ptr, hits_var, current_hits, bail_block, dst, lhs, rhs, |b, l, r| {
                b.ins().iadd(l, r)
            });
        }
        LinearIntStep::AddI { dst, src, imm } => {
            let src_ptr = slot_addr(builder, base_ptr, src);
            let dst_ptr = slot_addr(builder, base_ptr, dst);
            emit_integer_guard(builder, src_ptr, hits_var, current_hits, bail_block);
            let src_val = builder.ins().load(types::I64, mem, src_ptr, LUA_VALUE_VALUE_OFFSET);
            let imm_val = builder.ins().iconst(types::I64, i64::from(imm));
            let result = builder.ins().iadd(src_val, imm_val);
            let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
            builder.ins().store(mem, result, dst_ptr, LUA_VALUE_VALUE_OFFSET);
            builder.ins().store(mem, int_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
        }
        LinearIntStep::Sub { dst, lhs, rhs } => {
            emit_binary_int_op(builder, base_ptr, hits_var, current_hits, bail_block, dst, lhs, rhs, |b, l, r| {
                b.ins().isub(l, r)
            });
        }
        LinearIntStep::Mul { dst, lhs, rhs } => {
            emit_binary_int_op(builder, base_ptr, hits_var, current_hits, bail_block, dst, lhs, rhs, |b, l, r| {
                b.ins().imul(l, r)
            });
        }
    }
}

fn emit_binary_int_op<F>(
    builder: &mut FunctionBuilder<'_>,
    base_ptr: Value,
    hits_var: Variable,
    current_hits: Value,
    bail_block: Block,
    dst: u32,
    lhs: u32,
    rhs: u32,
    op: F,
) where
    F: Fn(&mut FunctionBuilder<'_>, Value, Value) -> Value,
{
    let mem = MemFlags::new();
    let lhs_ptr = slot_addr(builder, base_ptr, lhs);
    let rhs_ptr = slot_addr(builder, base_ptr, rhs);
    let dst_ptr = slot_addr(builder, base_ptr, dst);
    emit_integer_guard(builder, lhs_ptr, hits_var, current_hits, bail_block);
    emit_integer_guard(builder, rhs_ptr, hits_var, current_hits, bail_block);
    let lhs_val = builder.ins().load(types::I64, mem, lhs_ptr, LUA_VALUE_VALUE_OFFSET);
    let rhs_val = builder.ins().load(types::I64, mem, rhs_ptr, LUA_VALUE_VALUE_OFFSET);
    let result = op(builder, lhs_val, rhs_val);
    let int_tag = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
    builder.ins().store(mem, result, dst_ptr, LUA_VALUE_VALUE_OFFSET);
    builder.ins().store(mem, int_tag, dst_ptr, LUA_VALUE_TT_OFFSET);
}

fn encode_trace_result(builder: &mut FunctionBuilder<'_>, hits: Value, completed: bool) -> Value {
    let shifted = builder.ins().ishl_imm(hits, 1);
    if completed {
        builder.ins().bor_imm(shifted, 1)
    } else {
        shifted
    }
}