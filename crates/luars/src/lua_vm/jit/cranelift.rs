use std::collections::{BTreeMap, BTreeSet};

use cranelift_codegen::{
    Context,
    ir::{AbiParam, FuncRef, InstBuilder, MemFlags, condcodes::IntCC, types},
    settings,
    settings::Configurable,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module, default_libcall_names};

use crate::{
    Chunk, LuaState,
    lua_value::{LUA_VFALSE, LUA_VNIL, LUA_VNUMFLT, LUA_VNUMINT, LUA_VTRUE},
    lua_vm::execute::helper::{lua_fmod, lua_idiv, lua_imod, luai_numpow},
};

use super::{
    CompiledTraceArtifact, JitPolicy, LoweredTraceBackend, TraceAbortReason, TraceBackend,
    TraceBackendError, TraceBackendKind, TraceCompilationUnit, TraceArtifact,
    artifact::CraneliftTraceArtifact,
    replay::{CompiledTraceIterationOutcome, execute_compiled_trace_iteration_from_step},
};

const ITERATION_CONTINUE_CODE: isize = -1;
const LUA_VALUE_SIZE: i32 = std::mem::size_of::<crate::LuaValue>() as i32;
const LUA_VALUE_VALUE_OFFSET: i32 = std::mem::offset_of!(crate::LuaValue, value) as i32;
const LUA_VALUE_TT_OFFSET: i32 = std::mem::offset_of!(crate::LuaValue, tt) as i32;

#[derive(Clone, Copy)]
struct RuntimeHelpers {
    int_idiv: FuncRef,
    int_imod: FuncRef,
    float_mod: FuncRef,
    float_pow: FuncRef,
}

#[derive(Clone, Copy)]
enum FastIntegerOp {
    Move { dst: u8, src: u8 },
    LoadI { dst: u8, value: i64 },
    AddI { dst: u8, src: u8, imm: i64 },
    Add { dst: u8, lhs: u8, rhs: u8 },
    Sub { dst: u8, lhs: u8, rhs: u8 },
    Mul { dst: u8, lhs: u8, rhs: u8 },
    AddK { dst: u8, src: u8, imm: i64 },
    SubK { dst: u8, src: u8, imm: i64 },
    MulK { dst: u8, src: u8, imm: i64 },
}

struct FastIntegerForLoopTrace {
    regs: Vec<u8>,
    ops: Vec<FastIntegerOp>,
    count_reg: u8,
    step_reg: u8,
    idx_reg: u8,
    exit_pc: usize,
}

#[derive(Clone, Copy)]
enum FastIntegerControlKind {
    LtRegs { lhs: u8, rhs: u8 },
    LeRegs { lhs: u8, rhs: u8 },
    LtImmRhs { reg: u8, imm: i64 },
    LeImmRhs { reg: u8, imm: i64 },
    LtImmLhs { imm: i64, reg: u8 },
    LeImmLhs { imm: i64, reg: u8 },
}

struct FastIntegerLoopbackTrace {
    regs: Vec<u8>,
    ops: Vec<FastIntegerOp>,
    control_kind: FastIntegerControlKind,
    continue_when: bool,
    exit_pc: usize,
    control_at_tail: bool,
}

type TraceEntryFn = unsafe extern "C" fn(
    *mut LuaState,
    *const Chunk,
    *const CraneliftTraceArtifact,
    usize,
    usize,
) -> isize;

#[derive(Debug, Default)]
pub struct CraneliftTraceBackend;

impl TraceBackend for CraneliftTraceBackend {
    fn name(&self) -> &'static str {
        "cranelift"
    }

    fn kind(&self) -> TraceBackendKind {
        TraceBackendKind::Cranelift
    }

    fn compile(
        &self,
        unit: &TraceCompilationUnit,
    ) -> Result<Option<TraceArtifact>, TraceBackendError> {
        let compiled = build_compiled_trace(unit)?;
        let entry = compile_trace_entry(unit)?;

        Ok(Some(TraceArtifact::Cranelift(CraneliftTraceArtifact {
            compiled,
            entry,
        })))
    }
}

fn build_compiled_trace(
    unit: &TraceCompilationUnit,
) -> Result<CompiledTraceArtifact, TraceBackendError> {
    let artifact = LoweredTraceBackend
        .compile(unit)?
        .ok_or(TraceBackendError::UnsupportedTrace)?;
    match artifact {
        TraceArtifact::Compiled(compiled) => Ok(compiled),
        _ => Err(TraceBackendError::UnsupportedTrace),
    }
}

fn compile_trace_entry(unit: &TraceCompilationUnit) -> Result<usize, TraceBackendError> {
    let trace_chunk = unsafe { &*(unit.plan.chunk_key as *const Chunk) };
    let mut flag_builder = settings::builder();
    flag_builder
        .set("opt_level", "speed")
        .map_err(|_| TraceBackendError::CompileFailed)?;
    let isa_builder = cranelift_native::builder().map_err(|_| TraceBackendError::CompileFailed)?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|_| TraceBackendError::CompileFailed)?;
    let call_conv = isa.default_call_conv();

    let mut jit_builder = JITBuilder::with_isa(isa, default_libcall_names());
    jit_builder.symbol(
        "luars_jit_execute_compiled_iteration_from_step",
        luars_jit_execute_compiled_iteration_from_step as *const u8,
    );
    jit_builder.symbol(
        "luars_jit_stack_base_ptr",
        luars_jit_stack_base_ptr as *const u8,
    );
    jit_builder.symbol("luars_jit_lua_idiv", luars_jit_lua_idiv as *const u8);
    jit_builder.symbol("luars_jit_lua_imod", luars_jit_lua_imod as *const u8);
    jit_builder.symbol("luars_jit_lua_fmod", luars_jit_lua_fmod as *const u8);
    jit_builder.symbol("luars_jit_lua_pow", luars_jit_lua_pow as *const u8);
    let mut module = JITModule::new(jit_builder);
    let pointer_type = module.target_config().pointer_type();

    let mut ctx = Context::new();
    ctx.func.signature.call_conv = call_conv;
    for _ in 0..5 {
        ctx.func.signature.params.push(AbiParam::new(pointer_type));
    }
    ctx.func.signature.returns.push(AbiParam::new(pointer_type));

    let mut helper_sig = module.make_signature();
    helper_sig.call_conv = call_conv;
    for _ in 0..5 {
        helper_sig.params.push(AbiParam::new(pointer_type));
    }
    helper_sig.returns.push(AbiParam::new(pointer_type));
    let helper_id = module
        .declare_function(
            "luars_jit_execute_compiled_iteration_from_step",
            Linkage::Import,
            &helper_sig,
        )
        .map_err(|_| TraceBackendError::CompileFailed)?;

    let mut stack_sig = module.make_signature();
    stack_sig.call_conv = call_conv;
    stack_sig.params.push(AbiParam::new(pointer_type));
    stack_sig.params.push(AbiParam::new(pointer_type));
    stack_sig.returns.push(AbiParam::new(pointer_type));
    let stack_id = module
        .declare_function("luars_jit_stack_base_ptr", Linkage::Import, &stack_sig)
        .map_err(|_| TraceBackendError::CompileFailed)?;

    let mut int_binop_sig = module.make_signature();
    int_binop_sig.call_conv = call_conv;
    int_binop_sig.params.push(AbiParam::new(types::I64));
    int_binop_sig.params.push(AbiParam::new(types::I64));
    int_binop_sig.returns.push(AbiParam::new(types::I64));
    let int_idiv_id = module
        .declare_function("luars_jit_lua_idiv", Linkage::Import, &int_binop_sig)
        .map_err(|_| TraceBackendError::CompileFailed)?;
    let int_imod_id = module
        .declare_function("luars_jit_lua_imod", Linkage::Import, &int_binop_sig)
        .map_err(|_| TraceBackendError::CompileFailed)?;

    let mut float_binop_sig = module.make_signature();
    float_binop_sig.call_conv = call_conv;
    float_binop_sig.params.push(AbiParam::new(types::F64));
    float_binop_sig.params.push(AbiParam::new(types::F64));
    float_binop_sig.returns.push(AbiParam::new(types::F64));
    let float_mod_id = module
        .declare_function("luars_jit_lua_fmod", Linkage::Import, &float_binop_sig)
        .map_err(|_| TraceBackendError::CompileFailed)?;
    let float_pow_id = module
        .declare_function("luars_jit_lua_pow", Linkage::Import, &float_binop_sig)
        .map_err(|_| TraceBackendError::CompileFailed)?;

    let func_name = format!("luars_trace_clif_{}", unit.plan.id.0);
    let func_id = module
        .declare_function(&func_name, Linkage::Local, &ctx.func.signature)
        .map_err(|_| TraceBackendError::CompileFailed)?;

    let mut fb_ctx = FunctionBuilderContext::new();
    {
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
        if unit.plan.instructions.is_empty() {
            return Err(TraceBackendError::UnsupportedTrace);
        }

        let entry_block = builder.create_block();
        let loop_block = builder.create_block();
        let continue_block = builder.create_block();
        let done_block = builder.create_block();
        let remaining_var = builder.declare_var(pointer_type);
        let stack_var = builder.declare_var(pointer_type);
        let helpers = RuntimeHelpers {
            int_idiv: module.declare_func_in_func(int_idiv_id, &mut builder.func),
            int_imod: module.declare_func_in_func(int_imod_id, &mut builder.func),
            float_mod: module.declare_func_in_func(float_mod_id, &mut builder.func),
            float_pow: module.declare_func_in_func(float_pow_id, &mut builder.func),
        };
        let step_blocks = (0..unit.plan.instructions.len())
            .map(|_| builder.create_block())
            .collect::<Vec<_>>();
        let fallback_blocks = (0..unit.plan.instructions.len())
            .map(|_| builder.create_block())
            .collect::<Vec<_>>();
        let fast_integer_forloop = match_fast_integer_forloop_trace(unit, trace_chunk)?;
        let fast_integer_loopback = match_fast_integer_loopback_trace(unit, trace_chunk)?;

        builder.append_block_params_for_function_params(entry_block);

        builder.switch_to_block(entry_block);
        let params = builder.block_params(entry_block).to_vec();
        let lua_state = params[0];
        let chunk = params[1];
        let artifact = params[2];
        let base = params[3];
        let replay_budget = params[4];
        let stack_callee = module.declare_func_in_func(stack_id, &mut builder.func);
        let stack_call = builder.ins().call(stack_callee, &[lua_state, base]);
        let stack_ptr = builder.inst_results(stack_call)[0];
        builder.def_var(remaining_var, replay_budget);
        builder.def_var(stack_var, stack_ptr);

        if let Some(fast_trace) = fast_integer_forloop {
            emit_fast_integer_forloop_entry(
                &mut builder,
                stack_ptr,
                replay_budget,
                loop_block,
                fast_trace,
                unit.plan.anchor_pc,
            );
        } else if let Some(fast_trace) = fast_integer_loopback {
            emit_fast_integer_loopback_entry(
                &mut builder,
                stack_ptr,
                replay_budget,
                loop_block,
                fast_trace,
                unit.plan.anchor_pc,
            );
        } else {
            builder.ins().jump(loop_block, &[]);
        }

        builder.switch_to_block(loop_block);
        builder.ins().jump(step_blocks[0], &[]);

        for (index, step) in unit.plan.instructions.iter().enumerate() {
            builder.switch_to_block(step_blocks[index]);
            let stack_ptr = builder.use_var(stack_var);
            let instr = *trace_chunk
                .code
                .get(step.pc)
                .ok_or(TraceBackendError::UnsupportedTrace)?;

            let (guards_supported, has_control_guard) = emit_step_guards(
                &mut builder,
                stack_ptr,
                trace_chunk,
                &unit.lowered.guards,
                step.pc,
                fallback_blocks[index],
            )?;
            if !guards_supported {
                builder.ins().jump(fallback_blocks[index], &[]);
                continue;
            }

            if has_control_guard {
                jump_next_step(&mut builder, &step_blocks, index, &continue_block);
            } else if !emit_step_fast_path(
                &mut builder,
                stack_ptr,
                trace_chunk,
                &step_blocks,
                &fallback_blocks,
                &continue_block,
                helpers,
                index,
                step.opcode,
                instr,
                step.pc,
                unit.plan.anchor_pc,
            )? {
                builder.ins().jump(fallback_blocks[index], &[]);
            }

            builder.switch_to_block(fallback_blocks[index]);
            let callee = module.declare_func_in_func(helper_id, &mut builder.func);
            let step_index = builder.ins().iconst(pointer_type, index as i64);
            let call = builder
                .ins()
                .call(callee, &[lua_state, chunk, artifact, base, step_index]);
            let result = builder.inst_results(call)[0];
            let is_continue = builder
                .ins()
                .icmp_imm(IntCC::Equal, result, ITERATION_CONTINUE_CODE as i64);
            let helper_return_block = builder.create_block();
            builder
                .ins()
                .brif(is_continue, continue_block, &[], helper_return_block, &[]);

            builder.switch_to_block(helper_return_block);
            builder.ins().return_(&[result]);
        }

        builder.switch_to_block(continue_block);
        let remaining = builder.use_var(remaining_var);
        let decremented = builder.ins().iadd_imm(remaining, -1);
        builder.def_var(remaining_var, decremented);
        let is_zero = builder
            .ins()
            .icmp_imm(IntCC::Equal, decremented, 0);
        builder.ins().brif(is_zero, done_block, &[], loop_block, &[]);

        builder.switch_to_block(done_block);
        let done_value = builder.ins().iconst(pointer_type, unit.plan.anchor_pc as i64);
        builder.ins().return_(&[done_value]);
        builder.seal_all_blocks();
        builder.finalize();
    }

    module
        .define_function(func_id, &mut ctx)
        .map_err(|_| TraceBackendError::CompileFailed)?;
    module.clear_context(&mut ctx);
    module
        .finalize_definitions()
        .map_err(|_| TraceBackendError::CompileFailed)?;
    let code = module.get_finalized_function(func_id);
    let _module = Box::leak(Box::new(module));

    Ok(code as usize)
}

fn invert_bool(
    builder: &mut FunctionBuilder,
    value: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    builder.ins().icmp_imm(IntCC::Equal, value, 0)
}

fn match_fast_integer_forloop_trace(
    unit: &TraceCompilationUnit,
    trace_chunk: &Chunk,
) -> Result<Option<FastIntegerForLoopTrace>, TraceBackendError> {
    if unit.plan.anchor_kind != super::TraceAnchorKind::ForLoop {
        return Ok(None);
    }
    let Some(last_step) = unit.plan.instructions.last() else {
        return Ok(None);
    };
    if last_step.opcode != crate::OpCode::ForLoop {
        return Ok(None);
    }

    if unit
        .lowered
        .guards
        .iter()
        .any(|guard| guard.mode == super::TraceGuardMode::Control)
    {
        return Ok(None);
    }

    let forloop_instr = *trace_chunk
        .code
        .get(last_step.pc)
        .ok_or(TraceBackendError::UnsupportedTrace)?;

    let loop_base = forloop_instr.get_a() as u8;
    let count_reg = loop_base;
    let step_reg = loop_base + 1;
    let idx_reg = loop_base + 2;

    let Some((mut regs, ops)) = collect_fast_integer_body_ops(
        trace_chunk,
        &unit.plan.instructions[..unit.plan.instructions.len().saturating_sub(1)],
    )?
    else {
        return Ok(None);
    };
    regs.extend([count_reg, step_reg, idx_reg]);

    if !guards_match_fast_integer_regs(&unit.lowered.guards, &regs, None) {
        return Ok(None);
    }

    let exit_pc = unit
        .plan
        .exits
        .iter()
        .find(|exit| exit.kind == super::TraceExitKind::LoopExit && exit.source_pc == last_step.pc)
        .map(|exit| exit.target_pc)
        .unwrap_or(last_step.pc + 1);

    Ok(Some(FastIntegerForLoopTrace {
        regs: regs.into_iter().collect(),
        ops,
        count_reg,
        step_reg,
        idx_reg,
        exit_pc,
    }))
}

fn match_fast_integer_loopback_trace(
    unit: &TraceCompilationUnit,
    trace_chunk: &Chunk,
) -> Result<Option<FastIntegerLoopbackTrace>, TraceBackendError> {
    if unit.plan.anchor_kind != super::TraceAnchorKind::LoopBackedge {
        return Ok(None);
    }
    let Some(first_step) = unit.plan.instructions.first() else {
        return Ok(None);
    };
    let Some(last_step) = unit.plan.instructions.last() else {
        return Ok(None);
    };
    if first_step.pc == last_step.pc {
        return Ok(None);
    }

    let control_guards = unit
        .lowered
        .guards
        .iter()
        .filter(|guard| guard.mode == super::TraceGuardMode::Control)
        .collect::<Vec<_>>();
    if control_guards.len() != 1 {
        return Ok(None);
    }
    let control_guard = control_guards[0];
    let Some(control_kind) = match_fast_integer_control_kind(*control_guard) else {
        return Ok(None);
    };

    let (body_steps, control_at_tail) = if control_guard.pc == first_step.pc {
        if last_step.opcode != crate::OpCode::Jmp {
            return Ok(None);
        }
        let jmp_instr = *trace_chunk
            .code
            .get(last_step.pc)
            .ok_or(TraceBackendError::UnsupportedTrace)?;
        let target = step_target(jmp_instr, last_step.pc, unit.plan.anchor_pc)?;
        if target != unit.plan.anchor_pc {
            return Ok(None);
        }
        (
            &unit.plan.instructions[1..unit.plan.instructions.len().saturating_sub(1)],
            false,
        )
    } else if control_guard.pc == last_step.pc {
        (&unit.plan.instructions[..unit.plan.instructions.len().saturating_sub(1)], true)
    } else {
        return Ok(None);
    };

    let Some((mut regs, ops)) = collect_fast_integer_body_ops(
        trace_chunk,
        body_steps,
    )?
    else {
        return Ok(None);
    };
    extend_integer_control_regs(&mut regs, control_kind);

    if !guards_match_fast_integer_regs(&unit.lowered.guards, &regs, Some(control_guard.pc)) {
        return Ok(None);
    }

    let exit_pc = unit
        .plan
        .exits
        .iter()
        .find(|exit| {
            exit.kind == super::TraceExitKind::GuardExit && exit.source_pc == control_guard.pc
        })
        .map(|exit| exit.target_pc)
        .ok_or(TraceBackendError::UnsupportedTrace)?;

    Ok(Some(FastIntegerLoopbackTrace {
        regs: regs.into_iter().collect(),
        ops,
        control_kind,
        continue_when: control_guard.continue_when,
        exit_pc,
        control_at_tail,
    }))
}

fn collect_fast_integer_body_ops(
    trace_chunk: &Chunk,
    steps: &[super::TraceInstruction],
) -> Result<Option<(BTreeSet<u8>, Vec<FastIntegerOp>)>, TraceBackendError> {
    let mut regs = BTreeSet::new();
    let mut ops = Vec::new();

    for step in steps {
        let instr = *trace_chunk
            .code
            .get(step.pc)
            .ok_or(TraceBackendError::UnsupportedTrace)?;
        match step.opcode {
            crate::OpCode::Move => {
                let dst = instr.get_a() as u8;
                let src = instr.get_b() as u8;
                regs.insert(dst);
                regs.insert(src);
                ops.push(FastIntegerOp::Move { dst, src });
            }
            crate::OpCode::LoadI => {
                let dst = instr.get_a() as u8;
                regs.insert(dst);
                ops.push(FastIntegerOp::LoadI { dst, value: instr.get_sbx() as i64 });
            }
            crate::OpCode::AddI => {
                let dst = instr.get_a() as u8;
                let src = instr.get_b() as u8;
                regs.insert(dst);
                regs.insert(src);
                ops.push(FastIntegerOp::AddI { dst, src, imm: instr.get_sc() as i64 });
            }
            crate::OpCode::Add | crate::OpCode::Sub | crate::OpCode::Mul => {
                let dst = instr.get_a() as u8;
                let lhs = instr.get_b() as u8;
                let rhs = instr.get_c() as u8;
                regs.insert(dst);
                regs.insert(lhs);
                regs.insert(rhs);
                match step.opcode {
                    crate::OpCode::Add => ops.push(FastIntegerOp::Add { dst, lhs, rhs }),
                    crate::OpCode::Sub => ops.push(FastIntegerOp::Sub { dst, lhs, rhs }),
                    crate::OpCode::Mul => ops.push(FastIntegerOp::Mul { dst, lhs, rhs }),
                    _ => unreachable!(),
                }
            }
            crate::OpCode::AddK | crate::OpCode::SubK | crate::OpCode::MulK => {
                let dst = instr.get_a() as u8;
                let src = instr.get_b() as u8;
                let constant = trace_chunk
                    .constants
                    .get(instr.get_c() as usize)
                    .and_then(crate::LuaValue::as_integer_strict);
                let Some(imm) = constant else {
                    return Ok(None);
                };
                regs.insert(dst);
                regs.insert(src);
                match step.opcode {
                    crate::OpCode::AddK => ops.push(FastIntegerOp::AddK { dst, src, imm }),
                    crate::OpCode::SubK => ops.push(FastIntegerOp::SubK { dst, src, imm }),
                    crate::OpCode::MulK => ops.push(FastIntegerOp::MulK { dst, src, imm }),
                    _ => unreachable!(),
                }
            }
            _ => return Ok(None),
        }
    }

    Ok(Some((regs, ops)))
}

fn guards_match_fast_integer_regs(
    guards: &[super::TraceGuard],
    regs: &BTreeSet<u8>,
    control_pc: Option<usize>,
) -> bool {
    guards.iter().all(|guard| match (guard.kind, guard.operands) {
        (
            super::TraceGuardKind::IsNumber | super::TraceGuardKind::IsIntegerLike,
            super::TraceGuardOperands::Register { reg },
        ) if guard.mode == super::TraceGuardMode::Precondition && regs.contains(&reg) => true,
        (
            super::TraceGuardKind::IsComparableLtLe | super::TraceGuardKind::IsEqSafeComparable,
            super::TraceGuardOperands::Registers { lhs, rhs },
        ) if guard.mode == super::TraceGuardMode::Precondition
            && regs.contains(&lhs)
            && regs.contains(&rhs) => true,
        _ => control_pc.is_some_and(|pc| guard.mode == super::TraceGuardMode::Control && guard.pc == pc),
    })
}

fn emit_fast_integer_forloop_entry(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    replay_budget: cranelift_codegen::ir::Value,
    generic_loop_block: cranelift_codegen::ir::Block,
    fast_trace: FastIntegerForLoopTrace,
    anchor_pc: usize,
) {
    let reg_ptrs = fast_trace
        .regs
        .iter()
        .copied()
        .map(|reg| (reg, stack_slot_ptr(builder, stack_ptr, reg)))
        .collect::<BTreeMap<_, _>>();

    let mut all_int = None;
    for ptr in reg_ptrs.values().copied() {
        let tt = builder
            .ins()
            .load(types::I8, MemFlags::trusted(), ptr, LUA_VALUE_TT_OFFSET);
        let is_int = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMINT as i64);
        all_int = Some(match all_int {
            Some(acc) => builder.ins().band(acc, is_int),
            None => is_int,
        });
    }
    let Some(all_int) = all_int else {
        builder.ins().jump(generic_loop_block, &[]);
        return;
    };

    let fast_init_block = builder.create_block();
    builder.ins().brif(
        all_int,
        fast_init_block,
        &[],
        generic_loop_block,
        &[],
    );

    let reg_vars = fast_trace
        .regs
        .iter()
        .copied()
        .map(|reg| (reg, builder.declare_var(types::I64)))
        .collect::<BTreeMap<u8, Variable>>();
    let remaining_var = builder.declare_var(types::I64);

    let fast_loop_block = builder.create_block();
    let fast_continue_block = builder.create_block();
    let fast_anchor_return_block = builder.create_block();
    let fast_loop_exit_block = builder.create_block();

    builder.switch_to_block(fast_init_block);
    for (reg, ptr) in &reg_ptrs {
        let value = builder
            .ins()
            .load(types::I64, MemFlags::trusted(), *ptr, LUA_VALUE_VALUE_OFFSET);
        builder.def_var(reg_vars[reg], value);
    }
    builder.def_var(remaining_var, replay_budget);
    builder.ins().jump(fast_loop_block, &[]);

    builder.switch_to_block(fast_loop_block);
    for op in &fast_trace.ops {
        emit_fast_integer_op(builder, &reg_vars, *op);
    }
    let count = builder.use_var(reg_vars[&fast_trace.count_reg]);
    let should_continue = builder.ins().icmp_imm(IntCC::SignedGreaterThan, count, 0);
    builder.ins().brif(
        should_continue,
        fast_continue_block,
        &[],
        fast_loop_exit_block,
        &[],
    );

    builder.switch_to_block(fast_continue_block);
    let count = builder.use_var(reg_vars[&fast_trace.count_reg]);
    let step = builder.use_var(reg_vars[&fast_trace.step_reg]);
    let idx = builder.use_var(reg_vars[&fast_trace.idx_reg]);
    let remaining = builder.use_var(remaining_var);
    let next_count = builder.ins().iadd_imm(count, -1);
    let next_idx = builder.ins().iadd(idx, step);
    let next_remaining = builder.ins().iadd_imm(remaining, -1);
    builder.def_var(reg_vars[&fast_trace.count_reg], next_count);
    builder.def_var(reg_vars[&fast_trace.idx_reg], next_idx);
    builder.def_var(remaining_var, next_remaining);
    let budget_empty = builder.ins().icmp_imm(IntCC::Equal, next_remaining, 0);
    builder.ins().brif(
        budget_empty,
        fast_anchor_return_block,
        &[],
        fast_loop_block,
        &[],
    );

    builder.switch_to_block(fast_anchor_return_block);
    spill_fast_integer_forloop_state(builder, &reg_ptrs, &reg_vars, &fast_trace.regs);
    let done_value = builder.ins().iconst(types::I64, anchor_pc as i64);
    builder.ins().return_(&[done_value]);

    builder.switch_to_block(fast_loop_exit_block);
    spill_fast_integer_forloop_state(builder, &reg_ptrs, &reg_vars, &fast_trace.regs);
    let exit_value = builder.ins().iconst(types::I64, fast_trace.exit_pc as i64);
    builder.ins().return_(&[exit_value]);
}

fn emit_fast_integer_loopback_entry(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    replay_budget: cranelift_codegen::ir::Value,
    generic_loop_block: cranelift_codegen::ir::Block,
    fast_trace: FastIntegerLoopbackTrace,
    anchor_pc: usize,
) {
    let reg_ptrs = fast_trace
        .regs
        .iter()
        .copied()
        .map(|reg| (reg, stack_slot_ptr(builder, stack_ptr, reg)))
        .collect::<BTreeMap<_, _>>();

    let mut all_int = None;
    for ptr in reg_ptrs.values().copied() {
        let tt = builder
            .ins()
            .load(types::I8, MemFlags::trusted(), ptr, LUA_VALUE_TT_OFFSET);
        let is_int = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMINT as i64);
        all_int = Some(match all_int {
            Some(acc) => builder.ins().band(acc, is_int),
            None => is_int,
        });
    }
    let Some(all_int) = all_int else {
        builder.ins().jump(generic_loop_block, &[]);
        return;
    };

    let fast_init_block = builder.create_block();
    builder.ins().brif(all_int, fast_init_block, &[], generic_loop_block, &[]);

    let reg_vars = fast_trace
        .regs
        .iter()
        .copied()
        .map(|reg| (reg, builder.declare_var(types::I64)))
        .collect::<BTreeMap<u8, Variable>>();
    let remaining_var = builder.declare_var(types::I64);

    let fast_loop_block = builder.create_block();
    let fast_body_block = builder.create_block();
    let fast_exit_block = builder.create_block();
    let fast_anchor_return_block = builder.create_block();

    builder.switch_to_block(fast_init_block);
    for (reg, ptr) in &reg_ptrs {
        let value = builder
            .ins()
            .load(types::I64, MemFlags::trusted(), *ptr, LUA_VALUE_VALUE_OFFSET);
        builder.def_var(reg_vars[reg], value);
    }
    builder.def_var(remaining_var, replay_budget);
    builder.ins().jump(fast_loop_block, &[]);

    builder.switch_to_block(fast_loop_block);
    if fast_trace.control_at_tail {
        builder.ins().jump(fast_body_block, &[]);
    } else {
        let condition = emit_fast_integer_control_condition(builder, &reg_vars, fast_trace.control_kind);
        let should_continue = if fast_trace.continue_when {
            condition
        } else {
            builder.ins().bnot(condition)
        };
        builder.ins().brif(
            should_continue,
            fast_body_block,
            &[],
            fast_exit_block,
            &[],
        );
    }

    builder.switch_to_block(fast_body_block);
    for op in &fast_trace.ops {
        emit_fast_integer_op(builder, &reg_vars, *op);
    }
    if fast_trace.control_at_tail {
        let condition = emit_fast_integer_control_condition(builder, &reg_vars, fast_trace.control_kind);
        let should_continue = if fast_trace.continue_when {
            condition
        } else {
            invert_bool(builder, condition)
        };
        let tail_continue_block = builder.create_block();
        builder.ins().brif(
            should_continue,
            tail_continue_block,
            &[],
            fast_exit_block,
            &[],
        );
        builder.switch_to_block(tail_continue_block);
    }
    let remaining = builder.use_var(remaining_var);
    let next_remaining = builder.ins().iadd_imm(remaining, -1);
    builder.def_var(remaining_var, next_remaining);
    let budget_empty = builder.ins().icmp_imm(IntCC::Equal, next_remaining, 0);
    builder.ins().brif(
        budget_empty,
        fast_anchor_return_block,
        &[],
        fast_loop_block,
        &[],
    );

    builder.switch_to_block(fast_anchor_return_block);
    spill_fast_integer_forloop_state(builder, &reg_ptrs, &reg_vars, &fast_trace.regs);
    let done_value = builder.ins().iconst(types::I64, anchor_pc as i64);
    builder.ins().return_(&[done_value]);

    builder.switch_to_block(fast_exit_block);
    spill_fast_integer_forloop_state(builder, &reg_ptrs, &reg_vars, &fast_trace.regs);
    let exit_value = builder.ins().iconst(types::I64, fast_trace.exit_pc as i64);
    builder.ins().return_(&[exit_value]);
}

fn match_fast_integer_control_kind(guard: super::TraceGuard) -> Option<FastIntegerControlKind> {
    match (guard.kind, guard.operands) {
        (super::TraceGuardKind::Lt, super::TraceGuardOperands::Registers { lhs, rhs }) => {
            Some(FastIntegerControlKind::LtRegs { lhs, rhs })
        }
        (super::TraceGuardKind::Le, super::TraceGuardOperands::Registers { lhs, rhs }) => {
            Some(FastIntegerControlKind::LeRegs { lhs, rhs })
        }
        (super::TraceGuardKind::Lt, super::TraceGuardOperands::RegisterImmediate { reg, imm }) => {
            Some(FastIntegerControlKind::LtImmRhs { reg, imm })
        }
        (super::TraceGuardKind::Le, super::TraceGuardOperands::RegisterImmediate { reg, imm }) => {
            Some(FastIntegerControlKind::LeImmRhs { reg, imm })
        }
        (super::TraceGuardKind::Lt, super::TraceGuardOperands::ImmediateRegister { imm, reg }) => {
            Some(FastIntegerControlKind::LtImmLhs { imm, reg })
        }
        (super::TraceGuardKind::Le, super::TraceGuardOperands::ImmediateRegister { imm, reg }) => {
            Some(FastIntegerControlKind::LeImmLhs { imm, reg })
        }
        _ => None,
    }
}

fn extend_integer_control_regs(regs: &mut BTreeSet<u8>, control_kind: FastIntegerControlKind) {
    match control_kind {
        FastIntegerControlKind::LtRegs { lhs, rhs }
        | FastIntegerControlKind::LeRegs { lhs, rhs } => {
            regs.insert(lhs);
            regs.insert(rhs);
        }
        FastIntegerControlKind::LtImmRhs { reg, .. }
        | FastIntegerControlKind::LeImmRhs { reg, .. }
        | FastIntegerControlKind::LtImmLhs { reg, .. }
        | FastIntegerControlKind::LeImmLhs { reg, .. } => {
            regs.insert(reg);
        }
    }
}

fn emit_fast_integer_control_condition(
    builder: &mut FunctionBuilder,
    reg_vars: &BTreeMap<u8, Variable>,
    control_kind: FastIntegerControlKind,
) -> cranelift_codegen::ir::Value {
    match control_kind {
        FastIntegerControlKind::LtRegs { lhs, rhs } => {
            let lhs = builder.use_var(reg_vars[&lhs]);
            let rhs = builder.use_var(reg_vars[&rhs]);
            builder.ins().icmp(IntCC::SignedLessThan, lhs, rhs)
        }
        FastIntegerControlKind::LeRegs { lhs, rhs } => {
            let lhs = builder.use_var(reg_vars[&lhs]);
            let rhs = builder.use_var(reg_vars[&rhs]);
            builder.ins().icmp(IntCC::SignedLessThanOrEqual, lhs, rhs)
        }
        FastIntegerControlKind::LtImmRhs { reg, imm } => {
            let value = builder.use_var(reg_vars[&reg]);
            builder.ins().icmp_imm(IntCC::SignedLessThan, value, imm)
        }
        FastIntegerControlKind::LeImmRhs { reg, imm } => {
            let value = builder.use_var(reg_vars[&reg]);
            builder.ins().icmp_imm(IntCC::SignedLessThanOrEqual, value, imm)
        }
        FastIntegerControlKind::LtImmLhs { imm, reg } => {
            let value = builder.use_var(reg_vars[&reg]);
            let lhs = builder.ins().iconst(types::I64, imm);
            builder.ins().icmp(IntCC::SignedLessThan, lhs, value)
        }
        FastIntegerControlKind::LeImmLhs { imm, reg } => {
            let value = builder.use_var(reg_vars[&reg]);
            let lhs = builder.ins().iconst(types::I64, imm);
            builder.ins().icmp(IntCC::SignedLessThanOrEqual, lhs, value)
        }
    }
}

fn emit_fast_integer_op(
    builder: &mut FunctionBuilder,
    reg_vars: &BTreeMap<u8, Variable>,
    op: FastIntegerOp,
) {
    match op {
        FastIntegerOp::Move { dst, src } => {
            let value = builder.use_var(reg_vars[&src]);
            builder.def_var(reg_vars[&dst], value);
        }
        FastIntegerOp::LoadI { dst, value } => {
            let value = builder.ins().iconst(types::I64, value);
            builder.def_var(reg_vars[&dst], value);
        }
        FastIntegerOp::AddI { dst, src, imm } => {
            let value = builder.use_var(reg_vars[&src]);
            let result = builder.ins().iadd_imm(value, imm);
            builder.def_var(reg_vars[&dst], result);
        }
        FastIntegerOp::Add { dst, lhs, rhs } => {
            let lhs = builder.use_var(reg_vars[&lhs]);
            let rhs = builder.use_var(reg_vars[&rhs]);
            let result = builder.ins().iadd(lhs, rhs);
            builder.def_var(reg_vars[&dst], result);
        }
        FastIntegerOp::Sub { dst, lhs, rhs } => {
            let lhs = builder.use_var(reg_vars[&lhs]);
            let rhs = builder.use_var(reg_vars[&rhs]);
            let result = builder.ins().isub(lhs, rhs);
            builder.def_var(reg_vars[&dst], result);
        }
        FastIntegerOp::Mul { dst, lhs, rhs } => {
            let lhs = builder.use_var(reg_vars[&lhs]);
            let rhs = builder.use_var(reg_vars[&rhs]);
            let result = builder.ins().imul(lhs, rhs);
            builder.def_var(reg_vars[&dst], result);
        }
        FastIntegerOp::AddK { dst, src, imm } => {
            let value = builder.use_var(reg_vars[&src]);
            let result = builder.ins().iadd_imm(value, imm);
            builder.def_var(reg_vars[&dst], result);
        }
        FastIntegerOp::SubK { dst, src, imm } => {
            let value = builder.use_var(reg_vars[&src]);
            let result = builder.ins().iadd_imm(value, -imm);
            builder.def_var(reg_vars[&dst], result);
        }
        FastIntegerOp::MulK { dst, src, imm } => {
            let value = builder.use_var(reg_vars[&src]);
            let imm = builder.ins().iconst(types::I64, imm);
            let result = builder.ins().imul(value, imm);
            builder.def_var(reg_vars[&dst], result);
        }
    }
}

fn spill_fast_integer_forloop_state(
    builder: &mut FunctionBuilder,
    reg_ptrs: &BTreeMap<u8, cranelift_codegen::ir::Value>,
    reg_vars: &BTreeMap<u8, Variable>,
    regs: &[u8],
) {
    for reg in regs {
        let value = builder.use_var(reg_vars[reg]);
        write_integer_slot(builder, reg_ptrs[reg], value);
    }
}

pub(crate) fn execute_cranelift_trace(
    lua_state: &mut LuaState,
    chunk: &Chunk,
    artifact: &CraneliftTraceArtifact,
    base: usize,
    policy: JitPolicy,
) -> Result<usize, TraceAbortReason> {
    let replay_budget = policy.max_trace_replays.max(1) as usize;
    let entry: TraceEntryFn = unsafe { std::mem::transmute(artifact.entry) };
    let result = unsafe {
        entry(
            lua_state as *mut LuaState,
            chunk as *const Chunk,
            artifact as *const CraneliftTraceArtifact,
            base,
            replay_budget,
        )
    };
    if result >= 0 {
        Ok(result as usize)
    } else {
        Err(decode_abort(result))
    }
}

extern "C" fn luars_jit_execute_compiled_iteration_from_step(
    lua_state: *mut LuaState,
    chunk: *const Chunk,
    artifact: *const CraneliftTraceArtifact,
    base: usize,
    start_step: usize,
) -> isize {
    let lua_state = unsafe { &mut *lua_state };
    let chunk = unsafe { &*chunk };
    let artifact = unsafe { &*artifact };

    match execute_compiled_trace_iteration_from_step(
        lua_state,
        chunk,
        &artifact.compiled,
        base,
        start_step,
    ) {
        Ok(CompiledTraceIterationOutcome::LoopContinue) => ITERATION_CONTINUE_CODE,
        Ok(CompiledTraceIterationOutcome::ReturnPc(next_pc)) => next_pc as isize,
        Err(reason) => encode_abort(reason),
    }
}

extern "C" fn luars_jit_stack_base_ptr(lua_state: *mut LuaState, base: usize) -> *mut crate::LuaValue {
    let lua_state = unsafe { &mut *lua_state };
    unsafe { lua_state.stack_mut().as_mut_ptr().add(base) }
}

extern "C" fn luars_jit_lua_idiv(lhs: i64, rhs: i64) -> i64 {
    lua_idiv(lhs, rhs)
}

extern "C" fn luars_jit_lua_imod(lhs: i64, rhs: i64) -> i64 {
    lua_imod(lhs, rhs)
}

extern "C" fn luars_jit_lua_fmod(lhs: f64, rhs: f64) -> f64 {
    lua_fmod(lhs, rhs)
}

extern "C" fn luars_jit_lua_pow(lhs: f64, rhs: f64) -> f64 {
    luai_numpow(lhs, rhs)
}

fn emit_step_guards(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    trace_chunk: &Chunk,
    guards: &[super::TraceGuard],
    pc: usize,
    fallback_block: cranelift_codegen::ir::Block,
) -> Result<(bool, bool), TraceBackendError> {
    let mut has_control_guard = false;
    for guard in guards.iter().filter(|guard| guard.pc == pc) {
        has_control_guard |= guard.mode == super::TraceGuardMode::Control;
        let condition = match (guard.kind, guard.operands) {
            (super::TraceGuardKind::IsNumber, super::TraceGuardOperands::Register { reg })
                if guard.mode == super::TraceGuardMode::Precondition =>
            {
                let slot_ptr = stack_slot_ptr(builder, stack_ptr, reg);
                let tt = builder
                    .ins()
                    .load(types::I8, MemFlags::trusted(), slot_ptr, LUA_VALUE_TT_OFFSET);
                let is_int = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMINT as i64);
                let is_float = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMFLT as i64);
                builder.ins().bor(is_int, is_float)
            }
            (
                super::TraceGuardKind::IsComparableLtLe,
                super::TraceGuardOperands::Registers { lhs, rhs },
            ) if guard.mode == super::TraceGuardMode::Precondition => emit_both_numeric_slots(
                builder,
                stack_ptr,
                lhs,
                rhs,
            ),
            (
                super::TraceGuardKind::IsEqSafeComparable,
                super::TraceGuardOperands::Registers { lhs, rhs },
            ) if guard.mode == super::TraceGuardMode::Precondition => emit_eq_safe_comparable_pair(
                builder,
                stack_ptr,
                lhs,
                rhs,
            ),
            (super::TraceGuardKind::Truthy, super::TraceGuardOperands::Register { reg })
            | (super::TraceGuardKind::Falsey, super::TraceGuardOperands::Register { reg })
                if guard.mode == super::TraceGuardMode::Control =>
            {
                let slot_ptr = stack_slot_ptr(builder, stack_ptr, reg);
                let tt = builder
                    .ins()
                    .load(types::I8, MemFlags::trusted(), slot_ptr, LUA_VALUE_TT_OFFSET);
                let is_nil = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNIL as i64);
                let is_false = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VFALSE as i64);
                let is_falsey = builder.ins().bor(is_nil, is_false);
                match guard.kind {
                    super::TraceGuardKind::Truthy => builder.ins().bnot(is_falsey),
                    super::TraceGuardKind::Falsey => is_falsey,
                    _ => unreachable!(),
                }
            }
            (
                super::TraceGuardKind::Eq,
                super::TraceGuardOperands::Registers { lhs, rhs },
            ) if guard.mode == super::TraceGuardMode::Control => {
                emit_eq_registers(builder, stack_ptr, lhs, rhs, fallback_block)
            }
            (
                super::TraceGuardKind::Eq,
                super::TraceGuardOperands::RegisterConstant {
                    reg,
                    constant_index,
                },
            ) if guard.mode == super::TraceGuardMode::Control => {
                let Some(constant) = trace_chunk.constants.get(constant_index) else {
                    return Ok((false, false));
                };
                emit_eq_register_constant(builder, stack_ptr, reg, constant, fallback_block)
            }
            (
                super::TraceGuardKind::Eq,
                super::TraceGuardOperands::RegisterImmediate { reg, imm },
            ) if guard.mode == super::TraceGuardMode::Control => {
                let slot_ptr = stack_slot_ptr(builder, stack_ptr, reg);
                let tt = builder
                    .ins()
                    .load(types::I8, MemFlags::trusted(), slot_ptr, LUA_VALUE_TT_OFFSET);
                let value = emit_load_number_slot(builder, slot_ptr, tt, fallback_block);
                let immediate = builder.ins().f64const(imm as f64);
                builder
                    .ins()
                    .fcmp(cranelift_codegen::ir::condcodes::FloatCC::Equal, value, immediate)
            }
            (
                super::TraceGuardKind::Lt,
                super::TraceGuardOperands::Registers { lhs, rhs },
            ) if guard.mode == super::TraceGuardMode::Control => emit_numeric_guard_compare_registers(
                builder,
                stack_ptr,
                lhs,
                rhs,
                cranelift_codegen::ir::condcodes::FloatCC::LessThan,
                fallback_block,
            ),
            (
                super::TraceGuardKind::Lt,
                super::TraceGuardOperands::RegisterImmediate { reg, imm },
            ) if guard.mode == super::TraceGuardMode::Control => emit_numeric_guard_compare_immediate_rhs(
                builder,
                stack_ptr,
                reg,
                imm,
                cranelift_codegen::ir::condcodes::FloatCC::LessThan,
                fallback_block,
            ),
            (
                super::TraceGuardKind::Le,
                super::TraceGuardOperands::Registers { lhs, rhs },
            ) if guard.mode == super::TraceGuardMode::Control => emit_numeric_guard_compare_registers(
                builder,
                stack_ptr,
                lhs,
                rhs,
                cranelift_codegen::ir::condcodes::FloatCC::LessThanOrEqual,
                fallback_block,
            ),
            (
                super::TraceGuardKind::Le,
                super::TraceGuardOperands::RegisterImmediate { reg, imm },
            ) if guard.mode == super::TraceGuardMode::Control => emit_numeric_guard_compare_immediate_rhs(
                builder,
                stack_ptr,
                reg,
                imm,
                cranelift_codegen::ir::condcodes::FloatCC::LessThanOrEqual,
                fallback_block,
            ),
            (
                super::TraceGuardKind::Lt,
                super::TraceGuardOperands::ImmediateRegister { imm, reg },
            ) if guard.mode == super::TraceGuardMode::Control => emit_numeric_guard_compare_immediate_lhs(
                builder,
                stack_ptr,
                imm,
                reg,
                cranelift_codegen::ir::condcodes::FloatCC::LessThan,
                fallback_block,
            ),
            (
                super::TraceGuardKind::Le,
                super::TraceGuardOperands::ImmediateRegister { imm, reg },
            ) if guard.mode == super::TraceGuardMode::Control => emit_numeric_guard_compare_immediate_lhs(
                builder,
                stack_ptr,
                imm,
                reg,
                cranelift_codegen::ir::condcodes::FloatCC::LessThanOrEqual,
                fallback_block,
            ),
            _ => return Ok((false, false)),
        };

        let should_continue = if guard.continue_when {
            condition
        } else {
            invert_bool(builder, condition)
        };
        let next_block = builder.create_block();
        builder
            .ins()
            .brif(should_continue, next_block, &[], fallback_block, &[]);
        builder.switch_to_block(next_block);
    }
    Ok((true, has_control_guard))
}

fn emit_step_fast_path(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    trace_chunk: &Chunk,
    step_blocks: &[cranelift_codegen::ir::Block],
    fallback_blocks: &[cranelift_codegen::ir::Block],
    continue_block: &cranelift_codegen::ir::Block,
    helpers: RuntimeHelpers,
    index: usize,
    opcode: crate::OpCode,
    instr: crate::Instruction,
    pc: usize,
    anchor_pc: usize,
) -> Result<bool, TraceBackendError> {
    match opcode {
        crate::OpCode::Move => {
            let dst = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
            let src = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
            copy_slot(builder, src, dst);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::LoadI => {
            let dst = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
            let value = builder.ins().iconst(types::I64, instr.get_sbx() as i64);
            write_integer_slot(builder, dst, value);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::LoadF => {
            let dst = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
            let value = builder.ins().f64const(instr.get_sbx() as f64);
            write_float_slot(builder, dst, value);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::LoadFalse => {
            let dst = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
            write_tagged_zero_slot(builder, dst, LUA_VFALSE);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::LoadTrue => {
            let dst = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
            write_tagged_zero_slot(builder, dst, LUA_VTRUE);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::LoadNil => {
            for reg in instr.get_a()..=instr.get_a() + instr.get_b() {
                let dst = stack_slot_ptr(builder, stack_ptr, reg as u8);
                write_tagged_zero_slot(builder, dst, LUA_VNIL);
            }
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::AddI => {
            emit_numeric_addi(builder, stack_ptr, instr, fallback_blocks[index]);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::AddK
        | crate::OpCode::SubK
        | crate::OpCode::MulK
        | crate::OpCode::PowK
        | crate::OpCode::DivK
        | crate::OpCode::IDivK
        | crate::OpCode::ModK => {
            let Some(constant) = trace_chunk.constants.get(instr.get_c() as usize) else {
                return Ok(false);
            };
            if let Some(integer) = constant.as_integer_strict() {
                emit_numeric_binary_k_integer(
                    builder,
                    stack_ptr,
                    helpers,
                    opcode,
                    instr,
                    integer,
                    fallback_blocks[index],
                );
                jump_next_step(builder, step_blocks, index, continue_block);
                return Ok(true);
            }
            if let Some(number) = constant.as_float() {
                emit_numeric_binary_k_float(
                    builder,
                    stack_ptr,
                    helpers,
                    opcode,
                    instr,
                    number,
                    fallback_blocks[index],
                );
                jump_next_step(builder, step_blocks, index, continue_block);
                return Ok(true);
            }
            Ok(false)
        }
        crate::OpCode::BAndK | crate::OpCode::BOrK | crate::OpCode::BXorK => {
            let Some(constant) = trace_chunk.constants.get(instr.get_c() as usize) else {
                return Ok(false);
            };
            let Some(integer) = constant.as_integer_strict() else {
                return Ok(false);
            };
            emit_integer_binary_k(builder, stack_ptr, opcode, instr, integer, fallback_blocks[index]);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::Add
        | crate::OpCode::Sub
        | crate::OpCode::Mul
        | crate::OpCode::Pow
        | crate::OpCode::Div
        | crate::OpCode::IDiv
        | crate::OpCode::Mod
        | crate::OpCode::BAnd
        | crate::OpCode::BOr
        | crate::OpCode::BXor => {
            emit_numeric_binary_rr(builder, stack_ptr, helpers, opcode, instr, fallback_blocks[index]);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::Shl | crate::OpCode::Shr => {
            emit_integer_shift_rr(builder, stack_ptr, opcode, instr, fallback_blocks[index]);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::ShlI | crate::OpCode::ShrI => {
            emit_integer_shift_i(builder, stack_ptr, opcode, instr, fallback_blocks[index]);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::Unm => {
            emit_numeric_unm(builder, stack_ptr, instr, fallback_blocks[index]);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::BNot => {
            emit_integer_bnot(builder, stack_ptr, instr, fallback_blocks[index]);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::Not => {
            emit_not(builder, stack_ptr, instr);
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::EqK
        | crate::OpCode::EqI
        | crate::OpCode::LtI
        | crate::OpCode::LeI
        | crate::OpCode::GtI
        | crate::OpCode::GeI
        | crate::OpCode::Test
        | crate::OpCode::TestSet => {
            jump_next_step(builder, step_blocks, index, continue_block);
            Ok(true)
        }
        crate::OpCode::Jmp => {
            step_target(instr, pc, anchor_pc)?;
            builder.ins().jump(*continue_block, &[]);
            Ok(true)
        }
        crate::OpCode::ForLoop => {
            for_loop_target(instr, pc, anchor_pc)?;
            emit_integer_for_loop(builder, stack_ptr, instr, *continue_block, fallback_blocks[index]);
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn emit_numeric_addi(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    instr: crate::Instruction,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let src = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let dst = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    let tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), src, LUA_VALUE_TT_OFFSET);
    let is_int = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMINT as i64);
    let int_block = builder.create_block();
    let float_check_block = builder.create_block();
    let done_block = builder.create_block();
    builder
        .ins()
        .brif(is_int, int_block, &[], float_check_block, &[]);

    builder.switch_to_block(int_block);
    let int_value = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), src, LUA_VALUE_VALUE_OFFSET);
    let int_result = builder.ins().iadd_imm(int_value, instr.get_sc() as i64);
    write_integer_slot(builder, dst, int_result);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(float_check_block);
    let is_float = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMFLT as i64);
    let float_block = builder.create_block();
    builder
        .ins()
        .brif(is_float, float_block, &[], fallback_block, &[]);

    builder.switch_to_block(float_block);
    let float_value = builder
        .ins()
        .load(types::F64, MemFlags::trusted(), src, LUA_VALUE_VALUE_OFFSET);
    let immediate = builder.ins().f64const(instr.get_sc() as f64);
    let float_result = builder.ins().fadd(float_value, immediate);
    write_float_slot(builder, dst, float_result);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(done_block);
}

fn emit_integer_binary(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    opcode: crate::OpCode,
    instr: crate::Instruction,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let lhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let rhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_c() as u8);
    let dst_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    guard_integer_slot(builder, lhs_ptr, fallback_block);
    guard_integer_slot(builder, rhs_ptr, fallback_block);
    let lhs = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), lhs_ptr, LUA_VALUE_VALUE_OFFSET);
    let rhs = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), rhs_ptr, LUA_VALUE_VALUE_OFFSET);
    let result = match opcode {
        crate::OpCode::Add => builder.ins().iadd(lhs, rhs),
        crate::OpCode::Sub => builder.ins().isub(lhs, rhs),
        crate::OpCode::Mul => builder.ins().imul(lhs, rhs),
        crate::OpCode::BAnd => builder.ins().band(lhs, rhs),
        crate::OpCode::BOr => builder.ins().bor(lhs, rhs),
        crate::OpCode::BXor => builder.ins().bxor(lhs, rhs),
        _ => unreachable!(),
    };
    write_integer_slot(builder, dst_ptr, result);
}

fn emit_numeric_binary_rr(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    helpers: RuntimeHelpers,
    opcode: crate::OpCode,
    instr: crate::Instruction,
    fallback_block: cranelift_codegen::ir::Block,
) {
    match opcode {
        crate::OpCode::Add | crate::OpCode::Sub | crate::OpCode::Mul => {
            let lhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
            let rhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_c() as u8);
            let dst_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
            let lhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
            let rhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), rhs_ptr, LUA_VALUE_TT_OFFSET);
            let lhs_is_int = builder.ins().icmp_imm(IntCC::Equal, lhs_tt, LUA_VNUMINT as i64);
            let rhs_is_int = builder.ins().icmp_imm(IntCC::Equal, rhs_tt, LUA_VNUMINT as i64);
            let both_int = builder.ins().band(lhs_is_int, rhs_is_int);
            let int_block = builder.create_block();
            let float_check_block = builder.create_block();
            let done_block = builder.create_block();

            builder
                .ins()
                .brif(both_int, int_block, &[], float_check_block, &[]);

            builder.switch_to_block(int_block);
            emit_integer_binary(builder, stack_ptr, opcode, instr, fallback_block);
            builder.ins().jump(done_block, &[]);

            builder.switch_to_block(float_check_block);
            let lhs = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
            let rhs = emit_load_number_slot(builder, rhs_ptr, rhs_tt, fallback_block);
            let result = match opcode {
                crate::OpCode::Add => builder.ins().fadd(lhs, rhs),
                crate::OpCode::Sub => builder.ins().fsub(lhs, rhs),
                crate::OpCode::Mul => builder.ins().fmul(lhs, rhs),
                _ => unreachable!(),
            };
            write_float_slot(builder, dst_ptr, result);
            builder.ins().jump(done_block, &[]);

            builder.switch_to_block(done_block);
        }
        crate::OpCode::Pow => {
            let lhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
            let rhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_c() as u8);
            let dst_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
            let lhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
            let rhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), rhs_ptr, LUA_VALUE_TT_OFFSET);
            let lhs_num = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
            let rhs_num = emit_load_number_slot(builder, rhs_ptr, rhs_tt, fallback_block);
            let call = builder.ins().call(helpers.float_pow, &[lhs_num, rhs_num]);
            let result = builder.inst_results(call)[0];
            write_float_slot(builder, dst_ptr, result);
        }
        crate::OpCode::Div | crate::OpCode::IDiv | crate::OpCode::Mod => {
            let lhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
            let rhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_c() as u8);
            let dst_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
            let lhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
            let rhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), rhs_ptr, LUA_VALUE_TT_OFFSET);
            let lhs_is_int = builder.ins().icmp_imm(IntCC::Equal, lhs_tt, LUA_VNUMINT as i64);
            let rhs_is_int = builder.ins().icmp_imm(IntCC::Equal, rhs_tt, LUA_VNUMINT as i64);
            let both_int = builder.ins().band(lhs_is_int, rhs_is_int);
            let int_block = builder.create_block();
            let number_check_block = builder.create_block();
            let done_block = builder.create_block();

            builder
                .ins()
                .brif(both_int, int_block, &[], number_check_block, &[]);

            builder.switch_to_block(int_block);
            match opcode {
                crate::OpCode::Div => {
                    let lhs = builder
                        .ins()
                        .load(types::I64, MemFlags::trusted(), lhs_ptr, LUA_VALUE_VALUE_OFFSET);
                    let rhs = builder
                        .ins()
                        .load(types::I64, MemFlags::trusted(), rhs_ptr, LUA_VALUE_VALUE_OFFSET);
                    let lhs_num = builder.ins().fcvt_from_sint(types::F64, lhs);
                    let rhs_num = builder.ins().fcvt_from_sint(types::F64, rhs);
                    let result = builder.ins().fdiv(lhs_num, rhs_num);
                    write_float_slot(builder, dst_ptr, result);
                }
                crate::OpCode::IDiv | crate::OpCode::Mod => {
                    let rhs = builder
                        .ins()
                        .load(types::I64, MemFlags::trusted(), rhs_ptr, LUA_VALUE_VALUE_OFFSET);
                    let rhs_is_zero = builder.ins().icmp_imm(IntCC::Equal, rhs, 0);
                    let nonzero_block = builder.create_block();
                    builder
                        .ins()
                        .brif(rhs_is_zero, fallback_block, &[], nonzero_block, &[]);
                    builder.switch_to_block(nonzero_block);

                    let lhs = builder
                        .ins()
                        .load(types::I64, MemFlags::trusted(), lhs_ptr, LUA_VALUE_VALUE_OFFSET);
                    let result = match opcode {
                        crate::OpCode::IDiv => {
                            let call = builder.ins().call(helpers.int_idiv, &[lhs, rhs]);
                            builder.inst_results(call)[0]
                        }
                        crate::OpCode::Mod => {
                            let call = builder.ins().call(helpers.int_imod, &[lhs, rhs]);
                            builder.inst_results(call)[0]
                        }
                        _ => unreachable!(),
                    };
                    write_integer_slot(builder, dst_ptr, result);
                }
                _ => unreachable!(),
            }
            builder.ins().jump(done_block, &[]);

            builder.switch_to_block(number_check_block);
            let lhs_num = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
            let rhs_num = emit_load_number_slot(builder, rhs_ptr, rhs_tt, fallback_block);
            let result = match opcode {
                crate::OpCode::Div => builder.ins().fdiv(lhs_num, rhs_num),
                crate::OpCode::IDiv => {
                    let div = builder.ins().fdiv(lhs_num, rhs_num);
                    builder.ins().floor(div)
                }
                crate::OpCode::Mod => {
                    let call = builder.ins().call(helpers.float_mod, &[lhs_num, rhs_num]);
                    builder.inst_results(call)[0]
                }
                _ => unreachable!(),
            };
            write_float_slot(builder, dst_ptr, result);
            builder.ins().jump(done_block, &[]);

            builder.switch_to_block(done_block);
        }
        _ => emit_integer_binary(builder, stack_ptr, opcode, instr, fallback_block),
    }
}

fn emit_integer_binary_k(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    opcode: crate::OpCode,
    instr: crate::Instruction,
    constant: i64,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let lhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let dst_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    guard_integer_slot(builder, lhs_ptr, fallback_block);
    let lhs = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), lhs_ptr, LUA_VALUE_VALUE_OFFSET);
    let rhs = builder.ins().iconst(types::I64, constant);
    let result = match opcode {
        crate::OpCode::BAndK => builder.ins().band(lhs, rhs),
        crate::OpCode::BOrK => builder.ins().bor(lhs, rhs),
        crate::OpCode::BXorK => builder.ins().bxor(lhs, rhs),
        _ => unreachable!(),
    };
    write_integer_slot(builder, dst_ptr, result);
}

fn emit_numeric_binary_k_integer(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    helpers: RuntimeHelpers,
    opcode: crate::OpCode,
    instr: crate::Instruction,
    constant: i64,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let lhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let dst_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    match opcode {
        crate::OpCode::AddK | crate::OpCode::SubK | crate::OpCode::MulK => {
            guard_integer_slot(builder, lhs_ptr, fallback_block);
            let lhs = builder
                .ins()
                .load(types::I64, MemFlags::trusted(), lhs_ptr, LUA_VALUE_VALUE_OFFSET);
            let rhs = builder.ins().iconst(types::I64, constant);
            let result = match opcode {
                crate::OpCode::AddK => builder.ins().iadd(lhs, rhs),
                crate::OpCode::SubK => builder.ins().isub(lhs, rhs),
                crate::OpCode::MulK => builder.ins().imul(lhs, rhs),
                _ => unreachable!(),
            };
            write_integer_slot(builder, dst_ptr, result);
        }
        crate::OpCode::PowK => {
            let lhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
            let lhs_num = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
            let rhs = builder.ins().f64const(constant as f64);
            let call = builder.ins().call(helpers.float_pow, &[lhs_num, rhs]);
            let result = builder.inst_results(call)[0];
            write_float_slot(builder, dst_ptr, result);
        }
        crate::OpCode::DivK => {
            let lhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
            let lhs_num = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
            let rhs = builder.ins().f64const(constant as f64);
            let result = builder.ins().fdiv(lhs_num, rhs);
            write_float_slot(builder, dst_ptr, result);
        }
        crate::OpCode::IDivK | crate::OpCode::ModK => {
            let lhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
            let lhs_is_int = builder.ins().icmp_imm(IntCC::Equal, lhs_tt, LUA_VNUMINT as i64);
            let int_block = builder.create_block();
            let number_block = builder.create_block();
            let done_block = builder.create_block();
            builder
                .ins()
                .brif(lhs_is_int, int_block, &[], number_block, &[]);

            builder.switch_to_block(int_block);
            if constant == 0 {
                builder.ins().jump(fallback_block, &[]);
            } else {
                let lhs = builder
                    .ins()
                    .load(types::I64, MemFlags::trusted(), lhs_ptr, LUA_VALUE_VALUE_OFFSET);
                let rhs = builder.ins().iconst(types::I64, constant);
                let result = match opcode {
                    crate::OpCode::IDivK => {
                        let call = builder.ins().call(helpers.int_idiv, &[lhs, rhs]);
                        builder.inst_results(call)[0]
                    }
                    crate::OpCode::ModK => {
                        let call = builder.ins().call(helpers.int_imod, &[lhs, rhs]);
                        builder.inst_results(call)[0]
                    }
                    _ => unreachable!(),
                };
                write_integer_slot(builder, dst_ptr, result);
                builder.ins().jump(done_block, &[]);
            }

            builder.switch_to_block(number_block);
            let lhs_num = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
            let rhs = builder.ins().f64const(constant as f64);
            let result = match opcode {
                crate::OpCode::IDivK => {
                    let div = builder.ins().fdiv(lhs_num, rhs);
                    builder.ins().floor(div)
                }
                crate::OpCode::ModK => {
                    let call = builder.ins().call(helpers.float_mod, &[lhs_num, rhs]);
                    builder.inst_results(call)[0]
                }
                _ => unreachable!(),
            };
            write_float_slot(builder, dst_ptr, result);
            builder.ins().jump(done_block, &[]);

            builder.switch_to_block(done_block);
        }
        _ => unreachable!(),
    }
}

fn emit_numeric_binary_k_float(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    helpers: RuntimeHelpers,
    opcode: crate::OpCode,
    instr: crate::Instruction,
    constant: f64,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let lhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let dst_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    match opcode {
        crate::OpCode::AddK | crate::OpCode::SubK | crate::OpCode::MulK => {
            let lhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
            let lhs = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
            let rhs = builder.ins().f64const(constant);
            let result = match opcode {
                crate::OpCode::AddK => builder.ins().fadd(lhs, rhs),
                crate::OpCode::SubK => builder.ins().fsub(lhs, rhs),
                crate::OpCode::MulK => builder.ins().fmul(lhs, rhs),
                _ => unreachable!(),
            };
            write_float_slot(builder, dst_ptr, result);
        }
        crate::OpCode::PowK => {
            let lhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
            let lhs_num = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
            let rhs = builder.ins().f64const(constant);
            let call = builder.ins().call(helpers.float_pow, &[lhs_num, rhs]);
            let result = builder.inst_results(call)[0];
            write_float_slot(builder, dst_ptr, result);
        }
        crate::OpCode::DivK | crate::OpCode::IDivK | crate::OpCode::ModK => {
            let lhs_tt = builder
                .ins()
                .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
            let lhs_num = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
            let rhs = builder.ins().f64const(constant);
            let result = match opcode {
                crate::OpCode::DivK => builder.ins().fdiv(lhs_num, rhs),
                crate::OpCode::IDivK => {
                    let div = builder.ins().fdiv(lhs_num, rhs);
                    builder.ins().floor(div)
                }
                crate::OpCode::ModK => {
                    let call = builder.ins().call(helpers.float_mod, &[lhs_num, rhs]);
                    builder.inst_results(call)[0]
                }
                _ => unreachable!(),
            };
            write_float_slot(builder, dst_ptr, result);
        }
        _ => unreachable!(),
    }
}

fn emit_load_number_slot(
    builder: &mut FunctionBuilder,
    slot_ptr: cranelift_codegen::ir::Value,
    tt: cranelift_codegen::ir::Value,
    fallback_block: cranelift_codegen::ir::Block,
) -> cranelift_codegen::ir::Value {
    let result_var = builder.declare_var(types::F64);
    let int_block = builder.create_block();
    let float_block = builder.create_block();
    let done_block = builder.create_block();
    let is_int = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMINT as i64);
    builder
        .ins()
        .brif(is_int, int_block, &[], float_block, &[]);

    builder.switch_to_block(int_block);
    let int_value = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), slot_ptr, LUA_VALUE_VALUE_OFFSET);
    let float_value = builder.ins().fcvt_from_sint(types::F64, int_value);
    builder.def_var(result_var, float_value);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(float_block);
    let is_float = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMFLT as i64);
    let float_value_block = builder.create_block();
    builder
        .ins()
        .brif(is_float, float_value_block, &[], fallback_block, &[]);

    builder.switch_to_block(float_value_block);
    let float_value = builder
        .ins()
        .load(types::F64, MemFlags::trusted(), slot_ptr, LUA_VALUE_VALUE_OFFSET);
    builder.def_var(result_var, float_value);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(done_block);
    builder.use_var(result_var)
}

fn emit_numeric_guard_compare_immediate_rhs(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    reg: u8,
    imm: i64,
    cc: cranelift_codegen::ir::condcodes::FloatCC,
    fallback_block: cranelift_codegen::ir::Block,
) -> cranelift_codegen::ir::Value {
    let slot_ptr = stack_slot_ptr(builder, stack_ptr, reg);
    let tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), slot_ptr, LUA_VALUE_TT_OFFSET);
    let lhs = emit_load_number_slot(builder, slot_ptr, tt, fallback_block);
    let rhs = builder.ins().f64const(imm as f64);
    builder.ins().fcmp(cc, lhs, rhs)
}

fn emit_numeric_guard_compare_immediate_lhs(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    imm: i64,
    reg: u8,
    cc: cranelift_codegen::ir::condcodes::FloatCC,
    fallback_block: cranelift_codegen::ir::Block,
) -> cranelift_codegen::ir::Value {
    let slot_ptr = stack_slot_ptr(builder, stack_ptr, reg);
    let tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), slot_ptr, LUA_VALUE_TT_OFFSET);
    let lhs = builder.ins().f64const(imm as f64);
    let rhs = emit_load_number_slot(builder, slot_ptr, tt, fallback_block);
    builder.ins().fcmp(cc, lhs, rhs)
}

fn emit_numeric_guard_compare_registers(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    lhs_reg: u8,
    rhs_reg: u8,
    cc: cranelift_codegen::ir::condcodes::FloatCC,
    fallback_block: cranelift_codegen::ir::Block,
) -> cranelift_codegen::ir::Value {
    let lhs_ptr = stack_slot_ptr(builder, stack_ptr, lhs_reg);
    let rhs_ptr = stack_slot_ptr(builder, stack_ptr, rhs_reg);
    let lhs_tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
    let rhs_tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), rhs_ptr, LUA_VALUE_TT_OFFSET);
    let lhs = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
    let rhs = emit_load_number_slot(builder, rhs_ptr, rhs_tt, fallback_block);
    builder.ins().fcmp(cc, lhs, rhs)
}

fn emit_both_numeric_slots(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    lhs_reg: u8,
    rhs_reg: u8,
) -> cranelift_codegen::ir::Value {
    let lhs_ptr = stack_slot_ptr(builder, stack_ptr, lhs_reg);
    let rhs_ptr = stack_slot_ptr(builder, stack_ptr, rhs_reg);
    let lhs_tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
    let rhs_tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), rhs_ptr, LUA_VALUE_TT_OFFSET);
    let lhs_is_int = builder.ins().icmp_imm(IntCC::Equal, lhs_tt, LUA_VNUMINT as i64);
    let lhs_is_float = builder.ins().icmp_imm(IntCC::Equal, lhs_tt, LUA_VNUMFLT as i64);
    let rhs_is_int = builder.ins().icmp_imm(IntCC::Equal, rhs_tt, LUA_VNUMINT as i64);
    let rhs_is_float = builder.ins().icmp_imm(IntCC::Equal, rhs_tt, LUA_VNUMFLT as i64);
    let lhs_is_number = builder.ins().bor(lhs_is_int, lhs_is_float);
    let rhs_is_number = builder.ins().bor(rhs_is_int, rhs_is_float);
    builder.ins().band(lhs_is_number, rhs_is_number)
}

fn emit_eq_safe_comparable_pair(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    lhs_reg: u8,
    rhs_reg: u8,
) -> cranelift_codegen::ir::Value {
    let lhs_ptr = stack_slot_ptr(builder, stack_ptr, lhs_reg);
    let rhs_ptr = stack_slot_ptr(builder, stack_ptr, rhs_reg);
    let lhs_tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
    let rhs_tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), rhs_ptr, LUA_VALUE_TT_OFFSET);
    let lhs_safe = emit_eq_safe_tag(builder, lhs_tt);
    let rhs_safe = emit_eq_safe_tag(builder, rhs_tt);
    builder.ins().band(lhs_safe, rhs_safe)
}

fn emit_eq_registers(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    lhs_reg: u8,
    rhs_reg: u8,
    fallback_block: cranelift_codegen::ir::Block,
) -> cranelift_codegen::ir::Value {
    let lhs_ptr = stack_slot_ptr(builder, stack_ptr, lhs_reg);
    let rhs_ptr = stack_slot_ptr(builder, stack_ptr, rhs_reg);
    let lhs_tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), lhs_ptr, LUA_VALUE_TT_OFFSET);
    let rhs_tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), rhs_ptr, LUA_VALUE_TT_OFFSET);

    let result_var = builder.declare_var(types::I8);
    let numeric_check_block = builder.create_block();
    let non_numeric_block = builder.create_block();
    let done_block = builder.create_block();

    let lhs_is_int = builder.ins().icmp_imm(IntCC::Equal, lhs_tt, LUA_VNUMINT as i64);
    let lhs_is_float = builder.ins().icmp_imm(IntCC::Equal, lhs_tt, LUA_VNUMFLT as i64);
    let rhs_is_int = builder.ins().icmp_imm(IntCC::Equal, rhs_tt, LUA_VNUMINT as i64);
    let rhs_is_float = builder.ins().icmp_imm(IntCC::Equal, rhs_tt, LUA_VNUMFLT as i64);
    let lhs_is_number = builder.ins().bor(lhs_is_int, lhs_is_float);
    let rhs_is_number = builder.ins().bor(rhs_is_int, rhs_is_float);
    let both_numeric = builder.ins().band(lhs_is_number, rhs_is_number);
    builder.ins().brif(
        both_numeric,
        numeric_check_block,
        &[],
        non_numeric_block,
        &[],
    );

    builder.switch_to_block(numeric_check_block);
    let lhs = emit_load_number_slot(builder, lhs_ptr, lhs_tt, fallback_block);
    let rhs = emit_load_number_slot(builder, rhs_ptr, rhs_tt, fallback_block);
    let numeric_eq = builder
        .ins()
        .fcmp(cranelift_codegen::ir::condcodes::FloatCC::Equal, lhs, rhs);
    let one = builder.ins().iconst(types::I8, 1);
    let zero = builder.ins().iconst(types::I8, 0);
    let numeric_eq_i8 = builder.ins().select(numeric_eq, one, zero);
    builder.def_var(result_var, numeric_eq_i8);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(non_numeric_block);
    let tags_equal = builder.ins().icmp(IntCC::Equal, lhs_tt, rhs_tt);
    let safe_pair = emit_eq_safe_tag_pair(builder, lhs_tt, rhs_tt);
    let non_numeric_eq = builder.ins().band(tags_equal, safe_pair);
    let one = builder.ins().iconst(types::I8, 1);
    let zero = builder.ins().iconst(types::I8, 0);
    let non_numeric_eq_i8 = builder.ins().select(non_numeric_eq, one, zero);
    builder.def_var(result_var, non_numeric_eq_i8);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(done_block);
    let result = builder.use_var(result_var);
    builder.ins().icmp_imm(IntCC::NotEqual, result, 0)
}

fn emit_eq_safe_tag_pair(
    builder: &mut FunctionBuilder,
    lhs_tt: cranelift_codegen::ir::Value,
    rhs_tt: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let lhs_safe = emit_eq_safe_tag(builder, lhs_tt);
    let rhs_safe = emit_eq_safe_tag(builder, rhs_tt);
    builder.ins().band(lhs_safe, rhs_safe)
}

fn emit_eq_safe_tag(
    builder: &mut FunctionBuilder,
    tt: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let is_nil = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNIL as i64);
    let is_false = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VFALSE as i64);
    let is_true = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VTRUE as i64);
    let is_int = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMINT as i64);
    let is_float = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMFLT as i64);
    let is_bool = builder.ins().bor(is_false, is_true);
    let is_number = builder.ins().bor(is_int, is_float);
    let nil_or_bool = builder.ins().bor(is_nil, is_bool);
    builder.ins().bor(nil_or_bool, is_number)
}

fn emit_eq_register_constant(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    reg: u8,
    constant: &crate::LuaValue,
    fallback_block: cranelift_codegen::ir::Block,
) -> cranelift_codegen::ir::Value {
    let slot_ptr = stack_slot_ptr(builder, stack_ptr, reg);
    let tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), slot_ptr, LUA_VALUE_TT_OFFSET);

    if constant.is_nil() {
        return builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNIL as i64);
    }
    if constant.as_boolean() == Some(false) {
        return builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VFALSE as i64);
    }
    if constant.as_boolean() == Some(true) {
        return builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VTRUE as i64);
    }
    if let Some(number) = constant.as_float() {
        let lhs = emit_load_number_slot(builder, slot_ptr, tt, fallback_block);
        let rhs = builder.ins().f64const(number);
        return builder
            .ins()
            .fcmp(cranelift_codegen::ir::condcodes::FloatCC::Equal, lhs, rhs);
    }

    builder.ins().jump(fallback_block, &[]);
    builder.ins().iconst(types::I8, 0)
}

fn emit_integer_for_loop(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    instr: crate::Instruction,
    continue_block: cranelift_codegen::ir::Block,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let a = instr.get_a() as u8;
    let count_ptr = stack_slot_ptr(builder, stack_ptr, a);
    let step_ptr = stack_slot_ptr(builder, stack_ptr, a + 1);
    let idx_ptr = stack_slot_ptr(builder, stack_ptr, a + 2);

    guard_integer_slot(builder, count_ptr, fallback_block);
    guard_integer_slot(builder, step_ptr, fallback_block);
    guard_integer_slot(builder, idx_ptr, fallback_block);

    let count = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), count_ptr, LUA_VALUE_VALUE_OFFSET);
    let should_continue = builder.ins().icmp_imm(IntCC::SignedGreaterThan, count, 0);
    let body_block = builder.create_block();
    builder
        .ins()
        .brif(should_continue, body_block, &[], fallback_block, &[]);

    builder.switch_to_block(body_block);
    let step = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), step_ptr, LUA_VALUE_VALUE_OFFSET);
    let idx = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), idx_ptr, LUA_VALUE_VALUE_OFFSET);
    let next_count = builder.ins().iadd_imm(count, -1);
    let next_idx = builder.ins().iadd(idx, step);
    write_integer_slot(builder, count_ptr, next_count);
    write_integer_slot(builder, idx_ptr, next_idx);
    builder.ins().jump(continue_block, &[]);
}

fn emit_integer_shift_rr(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    opcode: crate::OpCode,
    instr: crate::Instruction,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let lhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let rhs_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_c() as u8);
    let dst_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    guard_integer_slot(builder, lhs_ptr, fallback_block);
    guard_integer_slot(builder, rhs_ptr, fallback_block);
    let lhs = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), lhs_ptr, LUA_VALUE_VALUE_OFFSET);
    let rhs = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), rhs_ptr, LUA_VALUE_VALUE_OFFSET);
    let result = match opcode {
        crate::OpCode::Shl => emit_lua_shiftl(builder, lhs, rhs),
        crate::OpCode::Shr => emit_lua_shiftr(builder, lhs, rhs),
        _ => unreachable!(),
    };
    write_integer_slot(builder, dst_ptr, result);
}

fn emit_integer_shift_i(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    opcode: crate::OpCode,
    instr: crate::Instruction,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let src_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let dst_ptr = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    guard_integer_slot(builder, src_ptr, fallback_block);
    let src = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), src_ptr, LUA_VALUE_VALUE_OFFSET);
    let immediate = builder.ins().iconst(types::I64, instr.get_sc() as i64);
    let result = match opcode {
        crate::OpCode::ShlI => emit_lua_shiftl(builder, immediate, src),
        crate::OpCode::ShrI => emit_lua_shiftr(builder, src, immediate),
        _ => unreachable!(),
    };
    write_integer_slot(builder, dst_ptr, result);
}

fn emit_integer_unm(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    instr: crate::Instruction,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let src = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let dst = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    guard_integer_slot(builder, src, fallback_block);
    let value = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), src, LUA_VALUE_VALUE_OFFSET);
    let zero = builder.ins().iconst(types::I64, 0);
    let result = builder.ins().isub(zero, value);
    write_integer_slot(builder, dst, result);
}

fn emit_numeric_unm(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    instr: crate::Instruction,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let src = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let dst = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    let src_tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), src, LUA_VALUE_TT_OFFSET);
    let is_int = builder.ins().icmp_imm(IntCC::Equal, src_tt, LUA_VNUMINT as i64);
    let int_block = builder.create_block();
    let float_check_block = builder.create_block();
    let done_block = builder.create_block();
    builder
        .ins()
        .brif(is_int, int_block, &[], float_check_block, &[]);

    builder.switch_to_block(int_block);
    emit_integer_unm(builder, stack_ptr, instr, fallback_block);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(float_check_block);
    let is_float = builder.ins().icmp_imm(IntCC::Equal, src_tt, LUA_VNUMFLT as i64);
    let float_block = builder.create_block();
    builder
        .ins()
        .brif(is_float, float_block, &[], fallback_block, &[]);

    builder.switch_to_block(float_block);
    let value = builder
        .ins()
        .load(types::F64, MemFlags::trusted(), src, LUA_VALUE_VALUE_OFFSET);
    let negated = builder.ins().fneg(value);
    write_float_slot(builder, dst, negated);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(done_block);
}

fn emit_integer_bnot(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    instr: crate::Instruction,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let src = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let dst = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    guard_integer_slot(builder, src, fallback_block);
    let value = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), src, LUA_VALUE_VALUE_OFFSET);
    let all_bits = builder.ins().iconst(types::I64, -1);
    let result = builder.ins().bxor(value, all_bits);
    write_integer_slot(builder, dst, result);
}

fn emit_not(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    instr: crate::Instruction,
) {
    let src = stack_slot_ptr(builder, stack_ptr, instr.get_b() as u8);
    let dst = stack_slot_ptr(builder, stack_ptr, instr.get_a() as u8);
    let tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), src, LUA_VALUE_TT_OFFSET);
    let is_nil = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNIL as i64);
    let is_false = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VFALSE as i64);
    let result = builder.ins().bor(is_nil, is_false);
    let false_tag = builder.ins().iconst(types::I8, LUA_VFALSE as i64);
    let true_tag = builder.ins().iconst(types::I8, LUA_VTRUE as i64);
    let result_tag = builder.ins().select(result, true_tag, false_tag);
    let zero = builder.ins().iconst(types::I64, 0);
    builder
        .ins()
        .store(MemFlags::trusted(), zero, dst, LUA_VALUE_VALUE_OFFSET);
    builder
        .ins()
        .store(MemFlags::trusted(), result_tag, dst, LUA_VALUE_TT_OFFSET);
}

fn emit_lua_shiftl(
    builder: &mut FunctionBuilder,
    value: cranelift_codegen::ir::Value,
    shift: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let result_var = builder.declare_var(types::I64);
    let negative_block = builder.create_block();
    let non_negative_block = builder.create_block();
    let right_zero_block = builder.create_block();
    let right_shift_block = builder.create_block();
    let left_zero_block = builder.create_block();
    let left_shift_block = builder.create_block();
    let done_block = builder.create_block();

    let is_negative = builder.ins().icmp_imm(IntCC::SignedLessThan, shift, 0);
    builder
        .ins()
        .brif(is_negative, negative_block, &[], non_negative_block, &[]);

    builder.switch_to_block(negative_block);
    let neg_shift = builder.ins().ineg(shift);
    let right_overflow = builder
        .ins()
        .icmp_imm(IntCC::SignedGreaterThanOrEqual, neg_shift, 64);
    builder
        .ins()
        .brif(right_overflow, right_zero_block, &[], right_shift_block, &[]);

    builder.switch_to_block(right_zero_block);
    let zero = builder.ins().iconst(types::I64, 0);
    builder.def_var(result_var, zero);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(right_shift_block);
    let shifted_right = builder.ins().ushr(value, neg_shift);
    builder.def_var(result_var, shifted_right);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(non_negative_block);
    let left_overflow = builder
        .ins()
        .icmp_imm(IntCC::SignedGreaterThanOrEqual, shift, 64);
    builder
        .ins()
        .brif(left_overflow, left_zero_block, &[], left_shift_block, &[]);

    builder.switch_to_block(left_zero_block);
    let zero = builder.ins().iconst(types::I64, 0);
    builder.def_var(result_var, zero);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(left_shift_block);
    let shifted_left = builder.ins().ishl(value, shift);
    builder.def_var(result_var, shifted_left);
    builder.ins().jump(done_block, &[]);

    builder.switch_to_block(done_block);
    builder.use_var(result_var)
}

fn emit_lua_shiftr(
    builder: &mut FunctionBuilder,
    value: cranelift_codegen::ir::Value,
    shift: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let neg_shift = builder.ins().ineg(shift);
    emit_lua_shiftl(builder, value, neg_shift)
}

fn write_integer_slot(
    builder: &mut FunctionBuilder,
    dst: cranelift_codegen::ir::Value,
    value: cranelift_codegen::ir::Value,
) {
    builder
        .ins()
        .store(MemFlags::trusted(), value, dst, LUA_VALUE_VALUE_OFFSET);
    let tt = builder.ins().iconst(types::I8, LUA_VNUMINT as i64);
    builder
        .ins()
        .store(MemFlags::trusted(), tt, dst, LUA_VALUE_TT_OFFSET);
}

fn write_float_slot(
    builder: &mut FunctionBuilder,
    dst: cranelift_codegen::ir::Value,
    value: cranelift_codegen::ir::Value,
) {
    builder
        .ins()
        .store(MemFlags::trusted(), value, dst, LUA_VALUE_VALUE_OFFSET);
    let tt = builder.ins().iconst(types::I8, LUA_VNUMFLT as i64);
    builder
        .ins()
        .store(MemFlags::trusted(), tt, dst, LUA_VALUE_TT_OFFSET);
}

fn write_tagged_zero_slot(
    builder: &mut FunctionBuilder,
    dst: cranelift_codegen::ir::Value,
    tag: u8,
) {
    let zero = builder.ins().iconst(types::I64, 0);
    let tt = builder.ins().iconst(types::I8, tag as i64);
    builder
        .ins()
        .store(MemFlags::trusted(), zero, dst, LUA_VALUE_VALUE_OFFSET);
    builder
        .ins()
        .store(MemFlags::trusted(), tt, dst, LUA_VALUE_TT_OFFSET);
}

fn copy_slot(
    builder: &mut FunctionBuilder,
    src: cranelift_codegen::ir::Value,
    dst: cranelift_codegen::ir::Value,
) {
    let value = builder
        .ins()
        .load(types::I64, MemFlags::trusted(), src, LUA_VALUE_VALUE_OFFSET);
    let tt = builder
        .ins()
        .load(types::I8, MemFlags::trusted(), src, LUA_VALUE_TT_OFFSET);
    builder
        .ins()
        .store(MemFlags::trusted(), value, dst, LUA_VALUE_VALUE_OFFSET);
    builder
        .ins()
        .store(MemFlags::trusted(), tt, dst, LUA_VALUE_TT_OFFSET);
}

fn guard_integer_slot(
    builder: &mut FunctionBuilder,
    slot_ptr: cranelift_codegen::ir::Value,
    fallback_block: cranelift_codegen::ir::Block,
) {
    let tt = builder.ins().load(types::I8, MemFlags::trusted(), slot_ptr, LUA_VALUE_TT_OFFSET);
    let is_int = builder.ins().icmp_imm(IntCC::Equal, tt, LUA_VNUMINT as i64);
    let next_block = builder.create_block();
    builder.ins().brif(is_int, next_block, &[], fallback_block, &[]);
    builder.switch_to_block(next_block);
}

fn stack_slot_ptr(
    builder: &mut FunctionBuilder,
    stack_ptr: cranelift_codegen::ir::Value,
    reg: u8,
) -> cranelift_codegen::ir::Value {
    builder
        .ins()
        .iadd_imm(stack_ptr, i64::from(reg) * i64::from(LUA_VALUE_SIZE))
}

fn jump_next_step(
    builder: &mut FunctionBuilder,
    step_blocks: &[cranelift_codegen::ir::Block],
    index: usize,
    continue_block: &cranelift_codegen::ir::Block,
) {
    if let Some(next_block) = step_blocks.get(index + 1) {
        builder.ins().jump(*next_block, &[]);
    } else {
        builder.ins().jump(*continue_block, &[]);
    }
}

fn step_target(instr: crate::Instruction, pc: usize, anchor_pc: usize) -> Result<usize, TraceBackendError> {
    let next_pc = pc
        .checked_add(1)
        .ok_or(TraceBackendError::UnsupportedTrace)?;
    let target = next_pc
        .checked_add_signed(instr.get_sj() as isize)
        .ok_or(TraceBackendError::UnsupportedTrace)?;
    if target != anchor_pc {
        return Err(TraceBackendError::UnsupportedTrace);
    }
    Ok(target)
}

fn for_loop_target(
    instr: crate::Instruction,
    pc: usize,
    anchor_pc: usize,
) -> Result<usize, TraceBackendError> {
    let next_pc = pc
        .checked_add(1)
        .ok_or(TraceBackendError::UnsupportedTrace)?;
    let target = next_pc
        .checked_sub(instr.get_bx() as usize)
        .ok_or(TraceBackendError::UnsupportedTrace)?;
    if target != anchor_pc {
        return Err(TraceBackendError::UnsupportedTrace);
    }
    Ok(target)
}

fn encode_abort(reason: TraceAbortReason) -> isize {
    -2 - reason_index(reason) as isize
}

fn decode_abort(code: isize) -> TraceAbortReason {
    match (-2 - code) as usize {
        0 => TraceAbortReason::NotImplemented,
        1 => TraceAbortReason::UnsupportedOpcode,
        2 => TraceAbortReason::UnsupportedControlFlow,
        3 => TraceAbortReason::SideEffectBoundary,
        4 => TraceAbortReason::InvalidAnchor,
        5 => TraceAbortReason::TraceTooLong,
        6 => TraceAbortReason::Blacklisted,
        _ => TraceAbortReason::NotImplemented,
    }
}

fn reason_index(reason: TraceAbortReason) -> usize {
    match reason {
        TraceAbortReason::NotImplemented => 0,
        TraceAbortReason::UnsupportedOpcode => 1,
        TraceAbortReason::UnsupportedControlFlow => 2,
        TraceAbortReason::SideEffectBoundary => 3,
        TraceAbortReason::InvalidAnchor => 4,
        TraceAbortReason::TraceTooLong => 5,
        TraceAbortReason::Blacklisted => 6,
    }
}

#[cfg(test)]
mod tests {
    use crate::{Chunk, Instruction, LuaLanguageLevel, LuaVM, OpCode, SafeOption};

    use super::*;
    use crate::lua_vm::jit::{
        TraceAnchorKind, TraceExit, TraceExitKind, TraceGuard, TraceGuardKind, TraceGuardMode,
        TraceGuardOperands, TraceId, TraceInstruction, TracePlan, TraceSnapshot,
        TraceSnapshotKind,
    };

    fn encode_sc(sc: i32) -> u32 {
        (sc + Instruction::OFFSET_SC) as u32
    }

    fn setup_state(stack_size: usize) -> Box<LuaVM> {
        let mut vm = LuaVM::new(SafeOption::default());
        vm.set_language_level(LuaLanguageLevel::LuaJIT);
        let state = vm.main_state();
        state.grow_stack(stack_size).expect("grow stack");
        state.set_top(stack_size).expect("set top");
        vm
    }

    #[test]
    fn cranelift_backend_executes_simple_trace() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(1);
        state.stack_mut()[1] = crate::LuaValue::integer(2);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];

        let plan = TracePlan {
            id: TraceId(201),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction {
                    pc: 0,
                    opcode: OpCode::Add,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 1,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
            ],
            guards: vec![
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 0 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 1 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
            ],
            exits: vec![TraceExit {
                kind: TraceExitKind::GuardExit,
                source_pc: 0,
                target_pc: 0,
                snapshot_index: 1,
                actions: Vec::new(),
            }],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 3,
            })
            .expect("cranelift trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[0].as_integer_strict(), Some(7));
    }

    #[test]
    fn cranelift_backend_falls_back_for_float_trace_inputs() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::float(1.5);
        state.stack_mut()[1] = crate::LuaValue::float(2.25);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];

        let plan = TracePlan {
            id: TraceId(202),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction {
                    pc: 0,
                    opcode: OpCode::Add,
                    line: None,
                    fallback: None,
                },
                TraceInstruction {
                    pc: 1,
                    opcode: OpCode::Jmp,
                    line: None,
                    fallback: None,
                },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
            ],
            guards: vec![
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 0 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 1 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
            ],
            exits: vec![TraceExit {
                kind: TraceExitKind::GuardExit,
                source_pc: 0,
                target_pc: 0,
                snapshot_index: 1,
                actions: Vec::new(),
            }],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 2,
            })
            .expect("cranelift trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[0].as_float(), Some(6.0));
    }

    #[test]
    fn cranelift_backend_executes_bitwise_and_unary_integer_trace() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(6);
        state.stack_mut()[1] = crate::LuaValue::integer(3);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::BAnd, 2, 0, 1),
            Instruction::create_abc(OpCode::BOr, 3, 0, 1),
            Instruction::create_abc(OpCode::BXor, 4, 0, 1),
            Instruction::create_abc(OpCode::Unm, 5, 0, 0),
            Instruction::create_abc(OpCode::BNot, 6, 1, 0),
            Instruction::create_sj(OpCode::Jmp, -6),
        ];

        let plan = TracePlan {
            id: TraceId(203),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 5,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::BAnd, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::BOr, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::BXor, line: None, fallback: None },
                TraceInstruction { pc: 3, opcode: OpCode::Unm, line: None, fallback: None },
                TraceInstruction { pc: 4, opcode: OpCode::BNot, line: None, fallback: None },
                TraceInstruction { pc: 5, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![],
            guards: vec![],
            exits: vec![],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 2,
            })
            .expect("cranelift bitwise trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[2].as_integer_strict(), Some(2));
        assert_eq!(state.stack()[3].as_integer_strict(), Some(7));
        assert_eq!(state.stack()[4].as_integer_strict(), Some(5));
        assert_eq!(state.stack()[5].as_integer_strict(), Some(-6));
        assert_eq!(state.stack()[6].as_integer_strict(), Some(-4));
    }

    #[test]
    fn cranelift_backend_executes_boolean_and_nil_trace() {
        let mut vm = setup_state(8);
        let state = vm.main_state();

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::LoadTrue, 0, 0, 0),
            Instruction::create_abc(OpCode::Not, 1, 0, 0),
            Instruction::create_abc(OpCode::LoadNil, 2, 1, 0),
            Instruction::create_abc(OpCode::Not, 4, 2, 0),
            Instruction::create_abc(OpCode::Not, 5, 3, 0),
            Instruction::create_sj(OpCode::Jmp, -6),
        ];

        let plan = TracePlan {
            id: TraceId(204),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 5,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::LoadTrue, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::Not, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::LoadNil, line: None, fallback: None },
                TraceInstruction { pc: 3, opcode: OpCode::Not, line: None, fallback: None },
                TraceInstruction { pc: 4, opcode: OpCode::Not, line: None, fallback: None },
                TraceInstruction { pc: 5, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![],
            guards: vec![],
            exits: vec![],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 2,
            })
            .expect("cranelift boolean trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[0].is_boolean(), true);
        assert_eq!(state.stack()[0].tt(), LUA_VTRUE);
        assert_eq!(state.stack()[1].tt(), LUA_VFALSE);
        assert!(state.stack()[2].is_nil());
        assert!(state.stack()[3].is_nil());
        assert_eq!(state.stack()[4].tt(), LUA_VTRUE);
        assert_eq!(state.stack()[5].tt(), LUA_VTRUE);
    }

    #[test]
    fn cranelift_backend_executes_shift_and_constant_bitwise_trace() {
        let mut vm = setup_state(12);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(8);
        state.stack_mut()[1] = crate::LuaValue::integer(2);
        state.stack_mut()[2] = crate::LuaValue::integer(-1);

        let mut chunk = Chunk::new();
        chunk.constants = vec![crate::LuaValue::integer(6)];
        chunk.code = vec![
            Instruction::create_abc(OpCode::Shl, 3, 0, 1),
            Instruction::create_abc(OpCode::Shr, 4, 0, 1),
            Instruction::create_abc(OpCode::ShlI, 5, 2, encode_sc(8)),
            Instruction::create_abc(OpCode::ShrI, 6, 0, encode_sc(1)),
            Instruction::create_abc(OpCode::BAndK, 7, 0, 0),
            Instruction::create_abc(OpCode::BOrK, 8, 0, 0),
            Instruction::create_abc(OpCode::BXorK, 9, 0, 0),
            Instruction::create_sj(OpCode::Jmp, -8),
        ];

        let plan = TracePlan {
            id: TraceId(205),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 7,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::Shl, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::Shr, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::ShlI, line: None, fallback: None },
                TraceInstruction { pc: 3, opcode: OpCode::ShrI, line: None, fallback: None },
                TraceInstruction { pc: 4, opcode: OpCode::BAndK, line: None, fallback: None },
                TraceInstruction { pc: 5, opcode: OpCode::BOrK, line: None, fallback: None },
                TraceInstruction { pc: 6, opcode: OpCode::BXorK, line: None, fallback: None },
                TraceInstruction { pc: 7, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![],
            guards: vec![],
            exits: vec![],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 2,
            })
            .expect("cranelift shift trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[3].as_integer_strict(), Some(32));
        assert_eq!(state.stack()[4].as_integer_strict(), Some(2));
        assert_eq!(state.stack()[5].as_integer_strict(), Some(4));
        assert_eq!(state.stack()[6].as_integer_strict(), Some(4));
        assert_eq!(state.stack()[7].as_integer_strict(), Some(0));
        assert_eq!(state.stack()[8].as_integer_strict(), Some(14));
        assert_eq!(state.stack()[9].as_integer_strict(), Some(14));
    }

    #[test]
    fn cranelift_backend_executes_integer_forloop_trace() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(0);
        state.stack_mut()[1] = crate::LuaValue::integer(2);
        state.stack_mut()[2] = crate::LuaValue::integer(1);
        state.stack_mut()[3] = crate::LuaValue::integer(1);
        state.stack_mut()[4] = crate::LuaValue::integer(1);
        state.stack_mut()[5] = crate::LuaValue::integer(1);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Add, 0, 0, 5),
            Instruction::create_abx(OpCode::ForLoop, 3, 2),
        ];

        let plan = TracePlan {
            id: TraceId(206),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 1,
            anchor_kind: TraceAnchorKind::ForLoop,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::Add, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::ForLoop, line: None, fallback: None },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 3, 4, 5],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 1,
                    resume_pc: 2,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 3, 4, 5],
                },
            ],
            guards: vec![],
            exits: vec![TraceExit {
                kind: TraceExitKind::LoopExit,
                source_pc: 1,
                target_pc: 2,
                snapshot_index: 1,
                actions: Vec::new(),
            }],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 8,
            })
            .expect("cranelift forloop trace should execute");

        assert_eq!(next_pc, 2);
        assert_eq!(state.stack()[0].as_integer_strict(), Some(3));
    }

    #[test]
    fn cranelift_backend_executes_float_mulk_forloop_trace() {
        let mut vm = setup_state(10);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::float(1.0);
        state.stack_mut()[1] = crate::LuaValue::integer(2);
        state.stack_mut()[2] = crate::LuaValue::integer(1);
        state.stack_mut()[3] = crate::LuaValue::integer(1);
        state.stack_mut()[4] = crate::LuaValue::integer(1);
        state.stack_mut()[5] = crate::LuaValue::integer(1);

        let mut chunk = Chunk::new();
        chunk.constants = vec![crate::LuaValue::float(1.5)];
        chunk.code = vec![
            Instruction::create_abc(OpCode::MulK, 0, 0, 0),
            Instruction::create_abx(OpCode::ForLoop, 3, 2),
        ];

        let plan = TracePlan {
            id: TraceId(207),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 1,
            anchor_kind: TraceAnchorKind::ForLoop,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::MulK, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::ForLoop, line: None, fallback: None },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 3, 4, 5],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 1,
                    resume_pc: 2,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 3, 4, 5],
                },
            ],
            guards: vec![],
            exits: vec![TraceExit {
                kind: TraceExitKind::LoopExit,
                source_pc: 1,
                target_pc: 2,
                snapshot_index: 1,
                actions: Vec::new(),
            }],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 8,
            })
            .expect("cranelift float mulk trace should execute");

        assert_eq!(next_pc, 2);
        assert_eq!(state.stack()[0].as_float(), Some(2.25));
    }

    #[test]
    fn cranelift_backend_executes_rr_float_arithmetic_trace() {
        let mut vm = setup_state(10);
        let state = vm.main_state();

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_asbx(OpCode::LoadF, 0, 3),
            Instruction::create_asbx(OpCode::LoadF, 1, 2),
            Instruction::create_abc(OpCode::Add, 2, 0, 1),
            Instruction::create_abc(OpCode::Sub, 3, 0, 1),
            Instruction::create_abc(OpCode::Mul, 4, 0, 1),
            Instruction::create_abc(OpCode::Unm, 5, 1, 0),
            Instruction::create_sj(OpCode::Jmp, -7),
        ];

        let plan = TracePlan {
            id: TraceId(208),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 6,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::LoadF, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::LoadF, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::Add, line: None, fallback: None },
                TraceInstruction { pc: 3, opcode: OpCode::Sub, line: None, fallback: None },
                TraceInstruction { pc: 4, opcode: OpCode::Mul, line: None, fallback: None },
                TraceInstruction { pc: 5, opcode: OpCode::Unm, line: None, fallback: None },
                TraceInstruction { pc: 6, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![],
            guards: vec![],
            exits: vec![],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 2,
            })
            .expect("cranelift rr float trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[2].as_float(), Some(5.0));
        assert_eq!(state.stack()[3].as_float(), Some(1.0));
        assert_eq!(state.stack()[4].as_float(), Some(6.0));
        assert_eq!(state.stack()[5].as_float(), Some(-2.0));
    }

    #[test]
    fn cranelift_backend_executes_rr_div_idiv_mod_trace() {
        let mut vm = setup_state(12);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(7);
        state.stack_mut()[1] = crate::LuaValue::integer(2);
        state.stack_mut()[2] = crate::LuaValue::float(7.5);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Div, 4, 0, 1),
            Instruction::create_abc(OpCode::IDiv, 5, 0, 1),
            Instruction::create_abc(OpCode::Mod, 6, 0, 1),
            Instruction::create_abc(OpCode::Div, 7, 2, 1),
            Instruction::create_abc(OpCode::IDiv, 8, 2, 1),
            Instruction::create_abc(OpCode::Mod, 9, 2, 1),
            Instruction::create_sj(OpCode::Jmp, -7),
        ];

        let plan = TracePlan {
            id: TraceId(209),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 6,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::Div, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::IDiv, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::Mod, line: None, fallback: None },
                TraceInstruction { pc: 3, opcode: OpCode::Div, line: None, fallback: None },
                TraceInstruction { pc: 4, opcode: OpCode::IDiv, line: None, fallback: None },
                TraceInstruction { pc: 5, opcode: OpCode::Mod, line: None, fallback: None },
                TraceInstruction { pc: 6, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![],
            guards: vec![],
            exits: vec![],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 2,
            })
            .expect("cranelift rr div trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[4].as_float(), Some(3.5));
        assert_eq!(state.stack()[5].as_integer_strict(), Some(3));
        assert_eq!(state.stack()[6].as_integer_strict(), Some(1));
        assert_eq!(state.stack()[7].as_float(), Some(3.75));
        assert_eq!(state.stack()[8].as_float(), Some(3.0));
        assert_eq!(state.stack()[9].as_float(), Some(1.5));
    }

    #[test]
    fn cranelift_backend_executes_k_div_idiv_mod_trace() {
        let mut vm = setup_state(12);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(7);
        state.stack_mut()[1] = crate::LuaValue::float(7.5);

        let mut chunk = Chunk::new();
        chunk.constants = vec![crate::LuaValue::integer(2), crate::LuaValue::float(2.5)];
        chunk.code = vec![
            Instruction::create_abc(OpCode::DivK, 2, 0, 0),
            Instruction::create_abc(OpCode::IDivK, 3, 0, 0),
            Instruction::create_abc(OpCode::ModK, 4, 0, 0),
            Instruction::create_abc(OpCode::DivK, 5, 1, 1),
            Instruction::create_abc(OpCode::IDivK, 6, 1, 1),
            Instruction::create_abc(OpCode::ModK, 7, 1, 1),
            Instruction::create_sj(OpCode::Jmp, -7),
        ];

        let plan = TracePlan {
            id: TraceId(210),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 6,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::DivK, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::IDivK, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::ModK, line: None, fallback: None },
                TraceInstruction { pc: 3, opcode: OpCode::DivK, line: None, fallback: None },
                TraceInstruction { pc: 4, opcode: OpCode::IDivK, line: None, fallback: None },
                TraceInstruction { pc: 5, opcode: OpCode::ModK, line: None, fallback: None },
                TraceInstruction { pc: 6, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![],
            guards: vec![],
            exits: vec![],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 2,
            })
            .expect("cranelift k div trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[2].as_float(), Some(3.5));
        assert_eq!(state.stack()[3].as_integer_strict(), Some(3));
        assert_eq!(state.stack()[4].as_integer_strict(), Some(1));
        assert_eq!(state.stack()[5].as_float(), Some(3.0));
        assert_eq!(state.stack()[6].as_float(), Some(3.0));
        assert_eq!(state.stack()[7].as_float(), Some(0.0));
    }

    #[test]
    fn cranelift_backend_executes_rr_pow_and_mixed_numeric_trace() {
        let mut vm = setup_state(12);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(3);
        state.stack_mut()[1] = crate::LuaValue::float(2.5);
        state.stack_mut()[2] = crate::LuaValue::integer(2);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Add, 3, 0, 1),
            Instruction::create_abc(OpCode::Sub, 4, 1, 2),
            Instruction::create_abc(OpCode::Mul, 5, 0, 1),
            Instruction::create_abc(OpCode::Pow, 6, 0, 2),
            Instruction::create_abc(OpCode::Pow, 7, 1, 2),
            Instruction::create_sj(OpCode::Jmp, -6),
        ];

        let plan = TracePlan {
            id: TraceId(211),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 5,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::Add, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::Sub, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::Mul, line: None, fallback: None },
                TraceInstruction { pc: 3, opcode: OpCode::Pow, line: None, fallback: None },
                TraceInstruction { pc: 4, opcode: OpCode::Pow, line: None, fallback: None },
                TraceInstruction { pc: 5, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![],
            guards: vec![],
            exits: vec![],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 2,
            })
            .expect("cranelift rr pow trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[3].as_float(), Some(5.5));
        assert_eq!(state.stack()[4].as_float(), Some(0.5));
        assert_eq!(state.stack()[5].as_float(), Some(7.5));
        assert_eq!(state.stack()[6].as_float(), Some(9.0));
        assert_eq!(state.stack()[7].as_float(), Some(6.25));
    }

    #[test]
    fn cranelift_backend_executes_k_pow_and_mixed_numeric_trace() {
        let mut vm = setup_state(12);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(3);
        state.stack_mut()[1] = crate::LuaValue::float(2.5);

        let mut chunk = Chunk::new();
        chunk.constants = vec![crate::LuaValue::float(2.5), crate::LuaValue::integer(2)];
        chunk.code = vec![
            Instruction::create_abc(OpCode::AddK, 2, 0, 0),
            Instruction::create_abc(OpCode::SubK, 3, 0, 0),
            Instruction::create_abc(OpCode::MulK, 4, 0, 0),
            Instruction::create_abc(OpCode::PowK, 5, 0, 1),
            Instruction::create_abc(OpCode::PowK, 6, 1, 1),
            Instruction::create_sj(OpCode::Jmp, -6),
        ];

        let plan = TracePlan {
            id: TraceId(212),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 5,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::AddK, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::SubK, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::MulK, line: None, fallback: None },
                TraceInstruction { pc: 3, opcode: OpCode::PowK, line: None, fallback: None },
                TraceInstruction { pc: 4, opcode: OpCode::PowK, line: None, fallback: None },
                TraceInstruction { pc: 5, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![],
            guards: vec![],
            exits: vec![],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 2,
            })
            .expect("cranelift k pow trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[2].as_float(), Some(5.5));
        assert_eq!(state.stack()[3].as_float(), Some(0.5));
        assert_eq!(state.stack()[4].as_float(), Some(7.5));
        assert_eq!(state.stack()[5].as_float(), Some(9.0));
        assert_eq!(state.stack()[6].as_float(), Some(6.25));
    }

    #[test]
    fn cranelift_backend_executes_float_addi_trace() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::float(2.5);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::AddI, 1, 0, encode_sc(5)),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];

        let plan = TracePlan {
            id: TraceId(213),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 1,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::AddI, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![],
            guards: vec![],
            exits: vec![],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 2,
            })
            .expect("cranelift float addi trace should execute");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[1].as_float(), Some(7.5));
    }

    #[test]
    fn cranelift_backend_handles_control_guards_and_testset_continue() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(10);
        state.stack_mut()[1] = crate::LuaValue::boolean(true);
        state.stack_mut()[2] = crate::LuaValue::integer(3);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::TestSet, 0, 1, 0, false),
            Instruction::create_sj(OpCode::Jmp, 2),
            Instruction::create_abc(OpCode::Add, 2, 2, 2),
            Instruction::create_sj(OpCode::Jmp, -4),
        ];

        let plan = TracePlan {
            id: TraceId(214),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 3,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::TestSet, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::Add, line: None, fallback: None },
                TraceInstruction { pc: 3, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 4,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
            ],
            guards: vec![TraceGuard {
                pc: 0,
                mode: TraceGuardMode::Control,
                kind: TraceGuardKind::Falsey,
                operands: TraceGuardOperands::Register { reg: 1 },
                continue_when: false,
                exit_snapshot_index: 1,
            }],
            exits: vec![TraceExit {
                kind: TraceExitKind::GuardExit,
                source_pc: 0,
                target_pc: 4,
                snapshot_index: 1,
                actions: vec![crate::lua_vm::jit::TraceExitAction::CopyReg { dst: 0, src: 1 }],
            }],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 1,
            })
            .expect("cranelift control-guard trace should continue");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[2].as_integer_strict(), Some(6));
    }

    #[test]
    fn cranelift_backend_handles_constant_and_immediate_control_guards() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(7);
        state.stack_mut()[1] = crate::LuaValue::integer(9);

        let mut chunk = Chunk::new();
        chunk.constants = vec![crate::LuaValue::integer(7)];
        chunk.code = vec![
            Instruction::create_abck(OpCode::EqK, 0, 0, 0, false),
            Instruction::create_sj(OpCode::Jmp, 3),
            Instruction::create_abck(OpCode::GtI, 1, 127 + 5, 0, false),
            Instruction::create_sj(OpCode::Jmp, 1),
            Instruction::create_abc(OpCode::Add, 1, 1, 0),
            Instruction::create_sj(OpCode::Jmp, -6),
        ];

        let plan = TracePlan {
            id: TraceId(215),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 5,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::EqK, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::GtI, line: None, fallback: None },
                TraceInstruction { pc: 4, opcode: OpCode::Add, line: None, fallback: None },
                TraceInstruction { pc: 5, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 5,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 2,
                    resume_pc: 4,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
            ],
            guards: vec![
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Control,
                    kind: TraceGuardKind::Eq,
                    operands: TraceGuardOperands::RegisterConstant { reg: 0, constant_index: 0 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 2,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 1 },
                    continue_when: true,
                    exit_snapshot_index: 2,
                },
                TraceGuard {
                    pc: 2,
                    mode: TraceGuardMode::Control,
                    kind: TraceGuardKind::Lt,
                    operands: TraceGuardOperands::ImmediateRegister { imm: 5, reg: 1 },
                    continue_when: true,
                    exit_snapshot_index: 2,
                },
            ],
            exits: vec![
                TraceExit {
                    kind: TraceExitKind::GuardExit,
                    source_pc: 0,
                    target_pc: 5,
                    snapshot_index: 1,
                    actions: Vec::new(),
                },
                TraceExit {
                    kind: TraceExitKind::GuardExit,
                    source_pc: 2,
                    target_pc: 4,
                    snapshot_index: 2,
                    actions: Vec::new(),
                },
            ],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 1,
            })
            .expect("cranelift immediate guard trace should continue");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[1].as_integer_strict(), Some(16));
    }

    #[test]
    fn cranelift_backend_handles_register_compare_control_guards() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(0);
        state.stack_mut()[1] = crate::LuaValue::integer(10);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::Lt, 0, 1, 0, false),
            Instruction::create_sj(OpCode::Jmp, 1),
            Instruction::create_abc(OpCode::AddI, 0, 0, encode_sc(1)),
            Instruction::create_sj(OpCode::Jmp, -4),
        ];

        let plan = TracePlan {
            id: TraceId(216),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 3,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::Lt, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::AddI, line: None, fallback: None },
                TraceInstruction { pc: 3, opcode: OpCode::Jmp, line: None, fallback: None },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 3,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 2,
                    resume_pc: 2,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1],
                },
            ],
            guards: vec![
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsComparableLtLe,
                    operands: TraceGuardOperands::Registers { lhs: 0, rhs: 1 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Control,
                    kind: TraceGuardKind::Lt,
                    operands: TraceGuardOperands::Registers { lhs: 0, rhs: 1 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 2,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 0 },
                    continue_when: true,
                    exit_snapshot_index: 2,
                },
            ],
            exits: vec![
                TraceExit {
                    kind: TraceExitKind::GuardExit,
                    source_pc: 0,
                    target_pc: 3,
                    snapshot_index: 1,
                    actions: Vec::new(),
                },
                TraceExit {
                    kind: TraceExitKind::GuardExit,
                    source_pc: 2,
                    target_pc: 2,
                    snapshot_index: 2,
                    actions: Vec::new(),
                },
            ],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 1,
            })
            .expect("cranelift register compare guard trace should continue");

        assert_eq!(next_pc, 0);
        assert_eq!(state.stack()[0].as_integer_strict(), Some(1));
    }

    #[test]
    fn cranelift_backend_executes_tail_control_integer_loop_trace() {
        let mut vm = setup_state(8);
        let state = vm.main_state();
        state.stack_mut()[0] = crate::LuaValue::integer(0);
        state.stack_mut()[1] = crate::LuaValue::integer(0);
        state.stack_mut()[2] = crate::LuaValue::integer(3);

        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_abc(OpCode::AddI, 1, 1, encode_sc(1)),
            Instruction::create_abck(OpCode::Le, 2, 1, 0, false),
            Instruction::create_sj(OpCode::Jmp, -4),
        ];

        let plan = TracePlan {
            id: TraceId(217),
            chunk_key: &chunk as *const Chunk as usize,
            anchor_pc: 0,
            end_pc: 2,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            instructions: vec![
                TraceInstruction { pc: 0, opcode: OpCode::Add, line: None, fallback: None },
                TraceInstruction { pc: 1, opcode: OpCode::AddI, line: None, fallback: None },
                TraceInstruction { pc: 2, opcode: OpCode::Le, line: None, fallback: None },
            ],
            snapshots: vec![
                TraceSnapshot {
                    kind: TraceSnapshotKind::Entry,
                    pc: 0,
                    resume_pc: 0,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1, 2],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 0,
                    resume_pc: 3,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1, 2],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 1,
                    resume_pc: 1,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1, 2],
                },
                TraceSnapshot {
                    kind: TraceSnapshotKind::SideExit,
                    pc: 2,
                    resume_pc: 3,
                    base: 0,
                    frame_depth: 0,
                    live_regs: vec![0, 1, 2],
                },
            ],
            guards: vec![
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 0 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 0,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 1 },
                    continue_when: true,
                    exit_snapshot_index: 1,
                },
                TraceGuard {
                    pc: 1,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsNumber,
                    operands: TraceGuardOperands::Register { reg: 1 },
                    continue_when: true,
                    exit_snapshot_index: 2,
                },
                TraceGuard {
                    pc: 2,
                    mode: TraceGuardMode::Precondition,
                    kind: TraceGuardKind::IsComparableLtLe,
                    operands: TraceGuardOperands::Registers { lhs: 2, rhs: 1 },
                    continue_when: true,
                    exit_snapshot_index: 3,
                },
                TraceGuard {
                    pc: 2,
                    mode: TraceGuardMode::Control,
                    kind: TraceGuardKind::Le,
                    operands: TraceGuardOperands::Registers { lhs: 2, rhs: 1 },
                    continue_when: false,
                    exit_snapshot_index: 3,
                },
            ],
            exits: vec![
                TraceExit {
                    kind: TraceExitKind::GuardExit,
                    source_pc: 0,
                    target_pc: 0,
                    snapshot_index: 1,
                    actions: Vec::new(),
                },
                TraceExit {
                    kind: TraceExitKind::GuardExit,
                    source_pc: 1,
                    target_pc: 1,
                    snapshot_index: 2,
                    actions: Vec::new(),
                },
                TraceExit {
                    kind: TraceExitKind::GuardExit,
                    source_pc: 2,
                    target_pc: 3,
                    snapshot_index: 3,
                    actions: Vec::new(),
                },
            ],
        };

        let backend = CraneliftTraceBackend;
        let artifact = backend
            .compile(&TraceCompilationUnit::new(plan))
            .expect("compile should succeed")
            .expect("backend should produce artifact");

        let next_pc = artifact
            .execute(state, &chunk, 0, JitPolicy {
                hotloop_threshold: 1,
                max_trace_instructions: 16,
                max_trace_replays: 8,
            })
            .expect("cranelift tail-control loop trace should execute");

        assert_eq!(next_pc, 3);
        assert_eq!(state.stack()[0].as_integer_strict(), Some(3));
        assert_eq!(state.stack()[1].as_integer_strict(), Some(3));
    }
}
