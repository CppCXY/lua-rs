use crate::{lua_value::LuaProto, Instruction};

use crate::lua_vm::jit::ir::{TraceIr, TraceIrGuard, TraceIrGuardKind, TraceIrInst, TraceIrInstKind};
use crate::lua_vm::jit::lowering::{LoweredExit, LoweredTrace};
use super::model::{
    CompiledTraceExecutor, LinearIntGuardOp, LinearIntLoopGuard, LinearIntStep,
    NumericBinaryOp, NumericIfElseCond, NumericJmpLoopGuard, NumericOperand, NumericStep,
};
use crate::lua_vm::jit::trace_recorder::TraceArtifact;

include!("compile_shared.rs");
include!("compile_loops.rs");
include!("compile_branches.rs");
include!("compile_iterators.rs");

pub(super) fn compile_executor(
    artifact: &TraceArtifact,
    ir: &TraceIr,
    lowered_trace: &LoweredTrace,
) -> Option<CompiledTraceExecutor> {
    if let Some(executor) = compile_terminal_return(ir) {
        return Some(executor);
    }

    if let Some(executor) = compile_linear_int_forloop(ir) {
        return Some(executor);
    }

    if let Some(executor) = compile_guarded_numeric_forloop(ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_guarded_numeric_ifelse_forloop(artifact, ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_numeric_ifelse_forloop(ir) {
        return Some(executor);
    }

    if let Some(executor) = compile_numeric_forloop(ir) {
        return Some(executor);
    }

    if let Some(executor) = compile_linear_int_jmp_loop(ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_numeric_table_shift_jmp_loop(ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_numeric_table_scan_jmp_loop(ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_numeric_jmp_loop(ir, lowered_trace) {
        return Some(executor);
    }

    if let Some(executor) = compile_generic_for_builtin_add(ir) {
        return Some(executor);
    }

    if let Some(executor) = compile_next_while_builtin_add(ir) {
        return Some(executor);
    }

    None
}

fn compile_terminal_return(ir: &TraceIr) -> Option<CompiledTraceExecutor> {
    if !ir.guards.is_empty() {
        return None;
    }

    let [inst] = ir.insts.as_slice() else {
        return None;
    };

    match inst.opcode {
        crate::OpCode::Return => {
            let instruction = Instruction::from_u32(inst.raw_instruction);
            if instruction.get_k() {
                return None;
            }

            match instruction.get_b() {
                0 => None,
                1 => Some(CompiledTraceExecutor::Return0),
                2 => Some(CompiledTraceExecutor::Return1 {
                    src_reg: instruction.get_a(),
                }),
                b => Some(CompiledTraceExecutor::Return {
                    start_reg: instruction.get_a(),
                    result_count: b.saturating_sub(1) as u8,
                }),
            }
        }
        crate::OpCode::Return0 => Some(CompiledTraceExecutor::Return0),
        crate::OpCode::Return1 => {
            let instruction = Instruction::from_u32(inst.raw_instruction);
            Some(CompiledTraceExecutor::Return1 {
                src_reg: instruction.get_a(),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::{Instruction, OpCode};
    use crate::lua_vm::jit::backend::compile::compile_executor;
    use crate::lua_vm::jit::backend::model::CompiledTraceExecutor;
    use crate::lua_vm::jit::helper_plan::HelperPlan;
    use crate::lua_vm::jit::ir::TraceIr;
    use crate::lua_vm::jit::lowering::LoweredTrace;
    use crate::lua_vm::jit::trace_recorder::{TraceArtifact, TraceOp, TraceSeed};

    #[test]
    fn compiles_terminal_return0_trace() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 7,
                root_chunk_addr: 0x55,
                instruction_budget: 1,
            },
            ops: vec![TraceOp {
                pc: 7,
                instruction: Instruction::create_abc(OpCode::Return0, 0, 0, 0),
                opcode: OpCode::Return0,
            }],
            exits: vec![],
            loop_tail_pc: 7,
        };
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            compile_executor(&artifact, &ir, &lowered),
            Some(CompiledTraceExecutor::Return0)
        );
    }

    #[test]
    fn compiles_terminal_return1_trace() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 9,
                root_chunk_addr: 0x56,
                instruction_budget: 1,
            },
            ops: vec![TraceOp {
                pc: 9,
                instruction: Instruction::create_abc(OpCode::Return1, 4, 0, 0),
                opcode: OpCode::Return1,
            }],
            exits: vec![],
            loop_tail_pc: 9,
        };
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            compile_executor(&artifact, &ir, &lowered),
            Some(CompiledTraceExecutor::Return1 { src_reg: 4 })
        );
    }

    #[test]
    fn compiles_fixed_arity_return_trace() {
        let artifact = TraceArtifact {
            seed: TraceSeed {
                start_pc: 11,
                root_chunk_addr: 0x57,
                instruction_budget: 1,
            },
            ops: vec![TraceOp {
                pc: 11,
                instruction: Instruction::create_abck(OpCode::Return, 5, 4, 0, false),
                opcode: OpCode::Return,
            }],
            exits: vec![],
            loop_tail_pc: 11,
        };
        let ir = TraceIr::lower(&artifact);
        let helper_plan = HelperPlan::lower(&ir);
        let lowered = LoweredTrace::lower(&artifact, &ir, &helper_plan);

        assert_eq!(
            compile_executor(&artifact, &ir, &lowered),
            Some(CompiledTraceExecutor::Return {
                start_reg: 5,
                result_count: 3,
            })
        );
    }
}
