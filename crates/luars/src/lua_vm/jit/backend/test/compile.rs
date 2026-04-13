
use crate::lua_vm::jit::backend::compile::{extract_loop_prologue, lower_linear_int_steps_for_native, lower_numeric_lowering_for_native, lower_numeric_steps_for_native, lower_numeric_steps_for_native_with_live_out, run_numeric_midend_passes_with_live_out};
use crate::lua_vm::jit::backend::{LinearIntStep, NumericBinaryOp, NumericOperand, NumericSelfUpdateValueFlow, NumericSelfUpdateValueKind, NumericStep, NumericValueFlowRhs};
use crate::{Instruction, LuaValue};
use crate::lua_vm::jit::backend::model::synthetic_artifact_for_ir;
use crate::lua_vm::jit::helper_plan::HelperPlan;
use crate::lua_vm::jit::ir::{TraceIr, TraceIrInst, TraceIrInstKind, TraceIrOperand};
use crate::lua_vm::jit::lowering::{LoweredSsaTrace, LoweredTrace, TraceValueKind};
use crate::lua_vm::{LuaVM, SafeOption};

fn run_numeric_midend_passes(steps: Vec<NumericStep>) -> Vec<NumericStep> {
    run_numeric_midend_passes_with_live_out(steps, &[])
}

fn lowered_trace_with_constants(constants: Vec<LuaValue>) -> LoweredTrace {
    LoweredTrace {
        root_pc: 0,
        loop_tail_pc: 0,
        snapshots: Vec::new(),
        exits: Vec::new(),
        helper_plan_step_count: 0,
        constants,
        root_register_hints: Vec::new(),
        entry_stable_register_hints: Vec::new(),
        ssa_trace: LoweredSsaTrace::default(),
    }
}

#[test]
fn lower_numeric_steps_consumes_ssa_table_int_rewrites() {
    let ir = TraceIr {
        root_pc: 40,
        loop_tail_pc: 43,
        insts: vec![
            TraceIrInst {
                pc: 40,
                opcode: crate::OpCode::SetTable,
                raw_instruction: Instruction::create_abck(crate::OpCode::SetTable, 3, 7, 8, false)
                    .as_u32(),
                kind: TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(8),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 41,
                opcode: crate::OpCode::GetTable,
                raw_instruction: Instruction::create_abck(crate::OpCode::GetTable, 9, 3, 7, false)
                    .as_u32(),
                kind: TraceIrInstKind::TableAccess,
                reads: vec![TraceIrOperand::Register(3), TraceIrOperand::Register(7)],
                writes: vec![TraceIrOperand::Register(9)],
            },
            TraceIrInst {
                pc: 42,
                opcode: crate::OpCode::SetTable,
                raw_instruction: Instruction::create_abck(crate::OpCode::SetTable, 3, 7, 10, false)
                    .as_u32(),
                kind: TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(10),
                ],
                writes: Vec::new(),
            },
            TraceIrInst {
                pc: 43,
                opcode: crate::OpCode::SetTable,
                raw_instruction: Instruction::create_abck(crate::OpCode::SetTable, 3, 7, 11, false)
                    .as_u32(),
                kind: TraceIrInstKind::TableAccess,
                reads: vec![
                    TraceIrOperand::Register(3),
                    TraceIrOperand::Register(7),
                    TraceIrOperand::Register(11),
                ],
                writes: Vec::new(),
            },
        ],
        guards: Vec::new(),
    };
    let helper_plan = HelperPlan::lower(&ir);
    let artifact = synthetic_artifact_for_ir(&ir);
    let lowered_trace = LoweredTrace::lower(&artifact, &ir, &helper_plan);

    let steps = lower_numeric_steps_for_native(&ir.insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![NumericStep::SetTableInt {
            table: 3,
            index: 7,
            value: 11,
        }]
    );
}

#[test]
fn lower_numeric_steps_with_live_out_keeps_move_only_used_by_live_out() {
    let insts = vec![
        TraceIrInst {
            pc: 0,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 0, 4, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(4)],
            writes: vec![TraceIrOperand::Register(0)],
        },
        TraceIrInst {
            pc: 1,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 1, 10, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(10)],
            writes: vec![TraceIrOperand::Register(1)],
        },
    ];

    let steps = lower_numeric_steps_for_native_with_live_out(
        &insts,
        &lowered_trace_with_constants(Vec::new()),
        &[0, 1],
    )
    .unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Move { dst: 0, src: 4 },
            NumericStep::Move { dst: 1, src: 10 }
        ]
    );
}

#[test]
fn lower_numeric_steps_with_live_out_still_prunes_unrelated_dead_move() {
    let insts = vec![
        TraceIrInst {
            pc: 0,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 0, 4, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(4)],
            writes: vec![TraceIrOperand::Register(0)],
        },
        TraceIrInst {
            pc: 1,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 7, 8, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(8)],
            writes: vec![TraceIrOperand::Register(7)],
        },
    ];

    let steps = lower_numeric_steps_for_native_with_live_out(
        &insts,
        &lowered_trace_with_constants(Vec::new()),
        &[0],
    )
    .unwrap();

    assert_eq!(steps, vec![NumericStep::Move { dst: 0, src: 4 }]);
}

#[test]
fn lower_linear_int_steps_consumes_integer_addk_and_subk_constants() {
    let insts = vec![
        TraceIrInst {
            pc: 10,
            opcode: crate::OpCode::AddK,
            raw_instruction: Instruction::create_abck(crate::OpCode::AddK, 4, 3, 0, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![
                TraceIrOperand::Register(3),
                TraceIrOperand::ConstantIndex(0),
            ],
            writes: vec![TraceIrOperand::Register(4)],
        },
        TraceIrInst {
            pc: 11,
            opcode: crate::OpCode::SubK,
            raw_instruction: Instruction::create_abck(crate::OpCode::SubK, 5, 4, 1, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![
                TraceIrOperand::Register(4),
                TraceIrOperand::ConstantIndex(1),
            ],
            writes: vec![TraceIrOperand::Register(5)],
        },
    ];

    let lowered_trace =
        lowered_trace_with_constants(vec![LuaValue::integer(7), LuaValue::integer(2)]);

    let steps = lower_linear_int_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            LinearIntStep::AddI {
                dst: 4,
                src: 3,
                imm: 7,
            },
            LinearIntStep::SubI {
                dst: 5,
                src: 4,
                imm: 2,
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_rewrite_integer_k_operands_to_immediates() {
    let insts = vec![
        TraceIrInst {
            pc: 20,
            opcode: crate::OpCode::AddK,
            raw_instruction: Instruction::create_abck(crate::OpCode::AddK, 4, 3, 0, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![
                TraceIrOperand::Register(3),
                TraceIrOperand::ConstantIndex(0),
            ],
            writes: vec![TraceIrOperand::Register(4)],
        },
        TraceIrInst {
            pc: 21,
            opcode: crate::OpCode::BAndK,
            raw_instruction: Instruction::create_abck(crate::OpCode::BAndK, 5, 4, 1, false)
                .as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![
                TraceIrOperand::Register(4),
                TraceIrOperand::ConstantIndex(1),
            ],
            writes: vec![TraceIrOperand::Register(5)],
        },
    ];

    let lowered_trace =
        lowered_trace_with_constants(vec![LuaValue::integer(9), LuaValue::integer(3)]);

    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Binary {
                dst: 4,
                lhs: NumericOperand::Reg(3),
                rhs: NumericOperand::ImmI(9),
                op: NumericBinaryOp::Add,
            },
            NumericStep::Binary {
                dst: 5,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::ImmI(3),
                op: NumericBinaryOp::BAnd,
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_normalizes_move_alias_rhs_and_prunes_dead_move() {
    let insts = vec![
        TraceIrInst {
            pc: 24,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 11, 6, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(11)],
        },
        TraceIrInst {
            pc: 25,
            opcode: crate::OpCode::Mul,
            raw_instruction: Instruction::create_abck(crate::OpCode::Mul, 4, 4, 11, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(11)],
            writes: vec![TraceIrOperand::Register(4)],
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![NumericStep::Binary {
            dst: 4,
            lhs: NumericOperand::Reg(4),
            rhs: NumericOperand::Reg(6),
            op: NumericBinaryOp::Mul,
        }]
    );
}

#[test]
fn lower_numeric_steps_normalizes_move_alias_set_upval_source() {
    let insts = vec![
        TraceIrInst {
            pc: 26,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 12, 7, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(7)],
            writes: vec![TraceIrOperand::Register(12)],
        },
        TraceIrInst {
            pc: 27,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 12, 2, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(12)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(steps, vec![NumericStep::SetUpval { src: 7, upvalue: 2 }]);
}

#[test]
fn lower_numeric_steps_forwards_adjacent_affine_addi_temp_move() {
    let insts = vec![
        TraceIrInst {
            pc: 28,
            opcode: crate::OpCode::AddI,
            raw_instruction: Instruction::create_abc(crate::OpCode::AddI, 11, 6, 129).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(11)],
        },
        TraceIrInst {
            pc: 29,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 12, 11, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(11)],
            writes: vec![TraceIrOperand::Register(12)],
        },
        TraceIrInst {
            pc: 30,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 12, 2, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(12)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Binary {
                dst: 12,
                lhs: NumericOperand::Reg(6),
                rhs: NumericOperand::ImmI(2),
                op: NumericBinaryOp::Add,
            },
            NumericStep::SetUpval {
                src: 12,
                upvalue: 2
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_forwards_adjacent_binary_temp_move() {
    let insts = vec![
        TraceIrInst {
            pc: 30,
            opcode: crate::OpCode::Mul,
            raw_instruction: Instruction::create_abck(crate::OpCode::Mul, 11, 4, 6, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(11)],
        },
        TraceIrInst {
            pc: 31,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 12, 11, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(11)],
            writes: vec![TraceIrOperand::Register(12)],
        },
        TraceIrInst {
            pc: 32,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 12, 3, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(12)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Binary {
                dst: 12,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(6),
                op: NumericBinaryOp::Mul,
            },
            NumericStep::SetUpval {
                src: 12,
                upvalue: 3
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_forwards_binary_temp_move_across_unrelated_pure_step() {
    let insts = vec![
        TraceIrInst {
            pc: 33,
            opcode: crate::OpCode::AddI,
            raw_instruction: Instruction::create_abc(crate::OpCode::AddI, 11, 6, 129).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(11)],
        },
        TraceIrInst {
            pc: 34,
            opcode: crate::OpCode::LoadI,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadI, 20, 7).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(20)],
        },
        TraceIrInst {
            pc: 35,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 12, 11, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(11)],
            writes: vec![TraceIrOperand::Register(12)],
        },
        TraceIrInst {
            pc: 36,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 20, 4, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(20)],
            writes: Vec::new(),
        },
        TraceIrInst {
            pc: 37,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 12, 4, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(12)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Binary {
                dst: 12,
                lhs: NumericOperand::Reg(6),
                rhs: NumericOperand::ImmI(2),
                op: NumericBinaryOp::Add,
            },
            NumericStep::LoadI { dst: 20, imm: 7 },
            NumericStep::SetUpval {
                src: 20,
                upvalue: 4
            },
            NumericStep::SetUpval {
                src: 12,
                upvalue: 4
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_prunes_dead_literal_loads() {
    let insts = vec![
        TraceIrInst {
            pc: 37,
            opcode: crate::OpCode::LoadI,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadI, 21, 9).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(21)],
        },
        TraceIrInst {
            pc: 38,
            opcode: crate::OpCode::LoadF,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadF, 22, 3).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(22)],
        },
        TraceIrInst {
            pc: 39,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 22, 5, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(22)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::LoadF { dst: 22, imm: 3 },
            NumericStep::SetUpval {
                src: 22,
                upvalue: 5
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_forwards_binary_through_single_consumer_move_chain() {
    let insts = vec![
        TraceIrInst {
            pc: 40,
            opcode: crate::OpCode::Mul,
            raw_instruction: Instruction::create_abck(crate::OpCode::Mul, 11, 4, 6, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(11)],
        },
        TraceIrInst {
            pc: 41,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 12, 11, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(11)],
            writes: vec![TraceIrOperand::Register(12)],
        },
        TraceIrInst {
            pc: 42,
            opcode: crate::OpCode::LoadI,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadI, 20, 8).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(20)],
        },
        TraceIrInst {
            pc: 43,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 13, 12, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(12)],
            writes: vec![TraceIrOperand::Register(13)],
        },
        TraceIrInst {
            pc: 44,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 20, 6, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(20)],
            writes: Vec::new(),
        },
        TraceIrInst {
            pc: 45,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 13, 6, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(13)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Binary {
                dst: 13,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(6),
                op: NumericBinaryOp::Mul,
            },
            NumericStep::LoadI { dst: 20, imm: 8 },
            NumericStep::SetUpval {
                src: 20,
                upvalue: 6
            },
            NumericStep::SetUpval {
                src: 13,
                upvalue: 6
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_forwards_binary_chain_into_setupval_source_use() {
    let insts = vec![
        TraceIrInst {
            pc: 52,
            opcode: crate::OpCode::Mul,
            raw_instruction: Instruction::create_abck(crate::OpCode::Mul, 11, 4, 6, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(11)],
        },
        TraceIrInst {
            pc: 53,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 12, 11, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(11)],
            writes: vec![TraceIrOperand::Register(12)],
        },
        TraceIrInst {
            pc: 54,
            opcode: crate::OpCode::LoadI,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadI, 20, 9).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(20)],
        },
        TraceIrInst {
            pc: 55,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 13, 12, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(12)],
            writes: vec![TraceIrOperand::Register(13)],
        },
        TraceIrInst {
            pc: 56,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 13, 8, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(13)],
            writes: Vec::new(),
        },
        TraceIrInst {
            pc: 57,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 20, 8, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(20)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Binary {
                dst: 13,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(6),
                op: NumericBinaryOp::Mul,
            },
            NumericStep::LoadI { dst: 20, imm: 9 },
            NumericStep::SetUpval {
                src: 13,
                upvalue: 8
            },
            NumericStep::SetUpval {
                src: 20,
                upvalue: 8
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_forwards_binary_chain_into_settable_value_use() {
    let insts = vec![
        TraceIrInst {
            pc: 58,
            opcode: crate::OpCode::AddI,
            raw_instruction: Instruction::create_abc(crate::OpCode::AddI, 11, 6, 129).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(11)],
        },
        TraceIrInst {
            pc: 59,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 12, 11, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(11)],
            writes: vec![TraceIrOperand::Register(12)],
        },
        TraceIrInst {
            pc: 60,
            opcode: crate::OpCode::LoadI,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadI, 20, 3).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(20)],
        },
        TraceIrInst {
            pc: 61,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 13, 12, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(12)],
            writes: vec![TraceIrOperand::Register(13)],
        },
        TraceIrInst {
            pc: 62,
            opcode: crate::OpCode::SetTable,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetTable, 3, 7, 13, false)
                .as_u32(),
            kind: TraceIrInstKind::TableAccess,
            reads: vec![
                TraceIrOperand::Register(3),
                TraceIrOperand::Register(7),
                TraceIrOperand::Register(13),
            ],
            writes: Vec::new(),
        },
        TraceIrInst {
            pc: 63,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 20, 9, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(20)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Binary {
                dst: 13,
                lhs: NumericOperand::Reg(6),
                rhs: NumericOperand::ImmI(2),
                op: NumericBinaryOp::Add,
            },
            NumericStep::LoadI { dst: 20, imm: 3 },
            NumericStep::SetTableInt {
                table: 3,
                index: 7,
                value: 13,
            },
            NumericStep::SetUpval {
                src: 20,
                upvalue: 9
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_forwards_binary_chain_into_pure_binary_consumer() {
    let insts = vec![
        TraceIrInst {
            pc: 67,
            opcode: crate::OpCode::Mul,
            raw_instruction: Instruction::create_abck(crate::OpCode::Mul, 11, 4, 6, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(11)],
        },
        TraceIrInst {
            pc: 68,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 12, 11, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(11)],
            writes: vec![TraceIrOperand::Register(12)],
        },
        TraceIrInst {
            pc: 69,
            opcode: crate::OpCode::LoadI,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadI, 20, 4).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(20)],
        },
        TraceIrInst {
            pc: 70,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 13, 12, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(12)],
            writes: vec![TraceIrOperand::Register(13)],
        },
        TraceIrInst {
            pc: 71,
            opcode: crate::OpCode::AddI,
            raw_instruction: Instruction::create_abc(crate::OpCode::AddI, 15, 13, 129).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(13)],
            writes: vec![TraceIrOperand::Register(15)],
        },
        TraceIrInst {
            pc: 72,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 20, 11, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(20)],
            writes: Vec::new(),
        },
        TraceIrInst {
            pc: 73,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 15, 11, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(15)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Binary {
                dst: 13,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(6),
                op: NumericBinaryOp::Mul,
            },
            NumericStep::LoadI { dst: 20, imm: 4 },
            NumericStep::Binary {
                dst: 15,
                lhs: NumericOperand::Reg(13),
                rhs: NumericOperand::ImmI(2),
                op: NumericBinaryOp::Add,
            },
            NumericStep::SetUpval {
                src: 20,
                upvalue: 11
            },
            NumericStep::SetUpval {
                src: 15,
                upvalue: 11
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_stabilizes_two_local_pure_consumer_chains() {
    let insts = vec![
        TraceIrInst {
            pc: 74,
            opcode: crate::OpCode::Mul,
            raw_instruction: Instruction::create_abck(crate::OpCode::Mul, 11, 4, 6, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(11)],
        },
        TraceIrInst {
            pc: 75,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 12, 11, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(11)],
            writes: vec![TraceIrOperand::Register(12)],
        },
        TraceIrInst {
            pc: 76,
            opcode: crate::OpCode::AddI,
            raw_instruction: Instruction::create_abc(crate::OpCode::AddI, 15, 12, 129).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(12)],
            writes: vec![TraceIrOperand::Register(15)],
        },
        TraceIrInst {
            pc: 77,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 16, 15, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(15)],
            writes: vec![TraceIrOperand::Register(16)],
        },
        TraceIrInst {
            pc: 78,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 16, 13, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(16)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Binary {
                dst: 12,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(6),
                op: NumericBinaryOp::Mul,
            },
            NumericStep::Binary {
                dst: 15,
                lhs: NumericOperand::Reg(12),
                rhs: NumericOperand::ImmI(2),
                op: NumericBinaryOp::Add,
            },
            NumericStep::SetUpval {
                src: 15,
                upvalue: 13
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_prunes_overwritten_pure_binary_temp() {
    let insts = vec![
        TraceIrInst {
            pc: 46,
            opcode: crate::OpCode::AddI,
            raw_instruction: Instruction::create_abc(crate::OpCode::AddI, 21, 6, 129).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(21)],
        },
        TraceIrInst {
            pc: 47,
            opcode: crate::OpCode::LoadI,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadI, 21, 5).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(21)],
        },
        TraceIrInst {
            pc: 48,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 21, 7, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(21)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::LoadI { dst: 21, imm: 5 },
            NumericStep::SetUpval {
                src: 21,
                upvalue: 7
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_prunes_overwritten_float_materialization() {
    let insts = vec![
        TraceIrInst {
            pc: 64,
            opcode: crate::OpCode::LoadF,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadF, 22, 3).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(22)],
        },
        TraceIrInst {
            pc: 65,
            opcode: crate::OpCode::LoadI,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadI, 22, 5).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(22)],
        },
        TraceIrInst {
            pc: 66,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 22, 10, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(22)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::LoadI { dst: 22, imm: 5 },
            NumericStep::SetUpval {
                src: 22,
                upvalue: 10
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_prunes_overwritten_bool_materialization() {
    let steps = vec![
        NumericStep::LoadBool {
            dst: 23,
            value: true,
        },
        NumericStep::LoadI { dst: 23, imm: 6 },
        NumericStep::SetUpval {
            src: 23,
            upvalue: 12,
        },
    ];

    let steps = run_numeric_midend_passes(steps);

    assert_eq!(
        steps,
        vec![
            NumericStep::LoadI { dst: 23, imm: 6 },
            NumericStep::SetUpval {
                src: 23,
                upvalue: 12
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_keeps_overwritten_non_pure_binary_temp() {
    let insts = vec![
        TraceIrInst {
            pc: 49,
            opcode: crate::OpCode::IDiv,
            raw_instruction: Instruction::create_abck(crate::OpCode::IDiv, 21, 4, 6, false)
                .as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(21)],
        },
        TraceIrInst {
            pc: 50,
            opcode: crate::OpCode::LoadI,
            raw_instruction: Instruction::create_asbx(crate::OpCode::LoadI, 21, 5).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: Vec::new(),
            writes: vec![TraceIrOperand::Register(21)],
        },
        TraceIrInst {
            pc: 51,
            opcode: crate::OpCode::SetUpval,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetUpval, 21, 7, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::UpvalueMutation,
            reads: vec![TraceIrOperand::Register(21)],
            writes: Vec::new(),
        },
    ];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::Binary {
                dst: 21,
                lhs: NumericOperand::Reg(4),
                rhs: NumericOperand::Reg(6),
                op: NumericBinaryOp::IDiv,
            },
            NumericStep::LoadI { dst: 21, imm: 5 },
            NumericStep::SetUpval {
                src: 21,
                upvalue: 7
            },
        ]
    );
}

#[test]
fn lower_linear_int_steps_consumes_integer_bitwise_and_shift_shapes() {
    let insts = vec![
        TraceIrInst {
            pc: 30,
            opcode: crate::OpCode::BAndK,
            raw_instruction: Instruction::create_abck(crate::OpCode::BAndK, 4, 3, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![
                TraceIrOperand::Register(3),
                TraceIrOperand::ConstantIndex(0),
            ],
            writes: vec![TraceIrOperand::Register(4)],
        },
        TraceIrInst {
            pc: 31,
            opcode: crate::OpCode::BOr,
            raw_instruction: Instruction::create_abck(crate::OpCode::BOr, 5, 4, 6, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(5)],
        },
        TraceIrInst {
            pc: 32,
            opcode: crate::OpCode::BXorK,
            raw_instruction: Instruction::create_abck(crate::OpCode::BXorK, 6, 5, 1, false)
                .as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![
                TraceIrOperand::Register(5),
                TraceIrOperand::ConstantIndex(1),
            ],
            writes: vec![TraceIrOperand::Register(6)],
        },
        TraceIrInst {
            pc: 33,
            opcode: crate::OpCode::ShlI,
            raw_instruction: Instruction::create_abc(crate::OpCode::ShlI, 7, 6, 129).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(7)],
        },
        TraceIrInst {
            pc: 34,
            opcode: crate::OpCode::Shr,
            raw_instruction: Instruction::create_abck(crate::OpCode::Shr, 8, 7, 9, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(7), TraceIrOperand::Register(9)],
            writes: vec![TraceIrOperand::Register(8)],
        },
    ];

    let lowered_trace =
        lowered_trace_with_constants(vec![LuaValue::integer(7), LuaValue::integer(3)]);

    let steps = lower_linear_int_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            LinearIntStep::BAndI {
                dst: 4,
                src: 3,
                imm: 7,
            },
            LinearIntStep::BOr {
                dst: 5,
                lhs: 4,
                rhs: 6,
            },
            LinearIntStep::BXorI {
                dst: 6,
                src: 5,
                imm: 3,
            },
            LinearIntStep::ShlI {
                dst: 7,
                imm: 2,
                src: 6,
            },
            LinearIntStep::Shr {
                dst: 8,
                lhs: 7,
                rhs: 9,
            },
        ]
    );
}

#[test]
fn lower_linear_int_steps_consumes_integer_idiv_and_mod_shapes() {
    let insts = vec![
        TraceIrInst {
            pc: 40,
            opcode: crate::OpCode::IDivK,
            raw_instruction: Instruction::create_abc(crate::OpCode::IDivK, 4, 3, 0).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![
                TraceIrOperand::Register(3),
                TraceIrOperand::ConstantIndex(0),
            ],
            writes: vec![TraceIrOperand::Register(4)],
        },
        TraceIrInst {
            pc: 41,
            opcode: crate::OpCode::Mod,
            raw_instruction: Instruction::create_abck(crate::OpCode::Mod, 5, 4, 6, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(5)],
        },
    ];

    let lowered_trace = lowered_trace_with_constants(vec![LuaValue::integer(2)]);

    let steps = lower_linear_int_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            LinearIntStep::IDivI {
                dst: 4,
                src: 3,
                imm: 2,
            },
            LinearIntStep::Mod {
                dst: 5,
                lhs: 4,
                rhs: 6,
            },
        ]
    );
}

#[test]
fn lower_linear_int_steps_consumes_integer_bnot_shape() {
    let insts = vec![TraceIrInst {
        pc: 50,
        opcode: crate::OpCode::BNot,
        raw_instruction: Instruction::create_abc(crate::OpCode::BNot, 4, 3, 0).as_u32(),
        kind: TraceIrInstKind::Arithmetic,
        reads: vec![TraceIrOperand::Register(3)],
        writes: vec![TraceIrOperand::Register(4)],
    }];

    let lowered_trace = lowered_trace_with_constants(Vec::new());
    let steps = lower_linear_int_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(steps, vec![LinearIntStep::BNot { dst: 4, src: 3 }]);
}

#[test]
fn lower_numeric_steps_reports_integer_self_update_value_flow() {
    let insts = vec![TraceIrInst {
        pc: 60,
        opcode: crate::OpCode::AddI,
        raw_instruction: Instruction::create_abc(crate::OpCode::AddI, 4, 4, 130).as_u32(),
        kind: TraceIrInstKind::Arithmetic,
        reads: vec![TraceIrOperand::Register(4)],
        writes: vec![TraceIrOperand::Register(4)],
    }];

    let mut lowered_trace = lowered_trace_with_constants(Vec::new());
    lowered_trace.root_register_hints = vec![crate::lua_vm::jit::lowering::RegisterValueHint {
        reg: 4,
        kind: TraceValueKind::Integer,
    }];
    lowered_trace.entry_stable_register_hints = lowered_trace.root_register_hints.clone();

    let lowering = lower_numeric_lowering_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        lowering.value_state.self_update,
        Some(NumericSelfUpdateValueFlow {
            reg: 4,
            op: NumericBinaryOp::Add,
            kind: NumericSelfUpdateValueKind::Integer,
            rhs: NumericValueFlowRhs::ImmI(3),
        })
    );
}

#[test]
fn lower_numeric_steps_reports_float_self_update_value_flow_after_alias_normalization() {
    let insts = vec![
        TraceIrInst {
            pc: 70,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 6, 8, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(8)],
            writes: vec![TraceIrOperand::Register(6)],
        },
        TraceIrInst {
            pc: 71,
            opcode: crate::OpCode::Mul,
            raw_instruction: Instruction::create_abck(crate::OpCode::Mul, 4, 4, 6, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(4)],
        },
    ];

    let mut lowered_trace = lowered_trace_with_constants(Vec::new());
    lowered_trace.root_register_hints = vec![
        crate::lua_vm::jit::lowering::RegisterValueHint {
            reg: 4,
            kind: TraceValueKind::Float,
        },
        crate::lua_vm::jit::lowering::RegisterValueHint {
            reg: 8,
            kind: TraceValueKind::Integer,
        },
    ];
    lowered_trace.entry_stable_register_hints = lowered_trace.root_register_hints.clone();

    let lowering = lower_numeric_lowering_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        lowering.value_state.self_update,
        Some(NumericSelfUpdateValueFlow {
            reg: 4,
            op: NumericBinaryOp::Mul,
            kind: NumericSelfUpdateValueKind::Float,
            rhs: NumericValueFlowRhs::StableReg {
                reg: 8,
                kind: TraceValueKind::Integer,
            },
        })
    );
}

#[test]
fn lower_numeric_steps_reports_integer_self_update_value_flow_with_residual_step() {
    let insts = vec![
        TraceIrInst {
            pc: 80,
            opcode: crate::OpCode::AddI,
            raw_instruction: Instruction::create_abc(crate::OpCode::AddI, 4, 4, 130).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4)],
            writes: vec![TraceIrOperand::Register(4)],
        },
        TraceIrInst {
            pc: 81,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 11, 9, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(9)],
            writes: vec![TraceIrOperand::Register(11)],
        },
    ];

    let mut lowered_trace = lowered_trace_with_constants(Vec::new());
    lowered_trace.root_register_hints = vec![
        crate::lua_vm::jit::lowering::RegisterValueHint {
            reg: 4,
            kind: TraceValueKind::Integer,
        },
        crate::lua_vm::jit::lowering::RegisterValueHint {
            reg: 9,
            kind: TraceValueKind::Integer,
        },
    ];
    lowered_trace.entry_stable_register_hints = lowered_trace.root_register_hints.clone();

    let lowering = lower_numeric_lowering_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        lowering.value_state.self_update,
        Some(NumericSelfUpdateValueFlow {
            reg: 4,
            op: NumericBinaryOp::Add,
            kind: NumericSelfUpdateValueKind::Integer,
            rhs: NumericValueFlowRhs::ImmI(3),
        })
    );
}

#[test]
fn lower_numeric_steps_reports_float_self_update_value_flow_with_residual_step() {
    let insts = vec![
        TraceIrInst {
            pc: 90,
            opcode: crate::OpCode::Mul,
            raw_instruction: Instruction::create_abck(crate::OpCode::Mul, 4, 4, 8, false).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(4), TraceIrOperand::Register(8)],
            writes: vec![TraceIrOperand::Register(4)],
        },
        TraceIrInst {
            pc: 91,
            opcode: crate::OpCode::Move,
            raw_instruction: Instruction::create_abc(crate::OpCode::Move, 11, 4, 0).as_u32(),
            kind: TraceIrInstKind::LoadMove,
            reads: vec![TraceIrOperand::Register(4)],
            writes: vec![TraceIrOperand::Register(11)],
        },
    ];

    let mut lowered_trace = lowered_trace_with_constants(Vec::new());
    lowered_trace.root_register_hints = vec![
        crate::lua_vm::jit::lowering::RegisterValueHint {
            reg: 4,
            kind: TraceValueKind::Float,
        },
        crate::lua_vm::jit::lowering::RegisterValueHint {
            reg: 8,
            kind: TraceValueKind::Integer,
        },
    ];
    lowered_trace.entry_stable_register_hints = lowered_trace.root_register_hints.clone();

    let lowering = lower_numeric_lowering_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        lowering.value_state.self_update,
        Some(NumericSelfUpdateValueFlow {
            reg: 4,
            op: NumericBinaryOp::Mul,
            kind: NumericSelfUpdateValueKind::Float,
            rhs: NumericValueFlowRhs::StableReg {
                reg: 8,
                kind: TraceValueKind::Integer,
            },
        })
    );
}

#[test]
fn lower_numeric_steps_lowers_short_string_field_access() {
    let mut vm = LuaVM::new(SafeOption::default());
    let key = vm.create_string("value").unwrap();
    let lowered_trace = lowered_trace_with_constants(vec![key]);

    let insts = vec![
        TraceIrInst {
            pc: 100,
            opcode: crate::OpCode::GetField,
            raw_instruction: Instruction::create_abck(crate::OpCode::GetField, 4, 3, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::TableAccess,
            reads: vec![
                TraceIrOperand::Register(3),
                TraceIrOperand::ConstantIndex(0),
            ],
            writes: vec![TraceIrOperand::Register(4)],
        },
        TraceIrInst {
            pc: 101,
            opcode: crate::OpCode::SetField,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetField, 3, 0, 4, false)
                .as_u32(),
            kind: TraceIrInstKind::TableAccess,
            reads: vec![
                TraceIrOperand::Register(3),
                TraceIrOperand::ConstantIndex(0),
                TraceIrOperand::Register(4),
            ],
            writes: Vec::new(),
        },
    ];

    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::GetTableField {
                dst: 4,
                table: 3,
                key: 0,
            },
            NumericStep::SetTableField {
                table: 3,
                key: 0,
                value: 4,
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_lowers_short_string_tabup_field_access() {
    let mut vm = LuaVM::new(SafeOption::default());
    let key = vm.create_string("value").unwrap();
    let lowered_trace = lowered_trace_with_constants(vec![key]);

    let insts = vec![
        TraceIrInst {
            pc: 100,
            opcode: crate::OpCode::GetTabUp,
            raw_instruction: Instruction::create_abck(crate::OpCode::GetTabUp, 4, 0, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::TableAccess,
            reads: vec![TraceIrOperand::Upvalue(0), TraceIrOperand::ConstantIndex(0)],
            writes: vec![TraceIrOperand::Register(4)],
        },
        TraceIrInst {
            pc: 101,
            opcode: crate::OpCode::SetTabUp,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetTabUp, 0, 0, 4, false)
                .as_u32(),
            kind: TraceIrInstKind::TableAccess,
            reads: vec![
                TraceIrOperand::Upvalue(0),
                TraceIrOperand::ConstantIndex(0),
                TraceIrOperand::Register(4),
            ],
            writes: Vec::new(),
        },
    ];

    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    assert_eq!(
        steps,
        vec![
            NumericStep::GetTabUpField {
                dst: 4,
                upvalue: 0,
                key: 0,
            },
            NumericStep::SetTabUpField {
                upvalue: 0,
                key: 0,
                value: 4,
            },
        ]
    );
}

#[test]
fn lower_numeric_steps_forwards_duplicate_get_tabup_field() {
    // Simulates: global_table.value = global_table.value + 1
    // GetTabUp R5, 0, K0("global_table")
    // GetField R6, R5, K1("value")
    // AddI R6, R6, 1
    // MmBinI companion
    // GetTabUp R7, 0, K0("global_table") ← should be forwarded
    // SetField R7, K1, R6
    let mut vm = LuaVM::new(SafeOption::default());
    let globals_key = vm.create_string("global_table").unwrap();
    let value_key = vm.create_string("value").unwrap();
    let lowered_trace = lowered_trace_with_constants(vec![globals_key, value_key]);

    let insts = vec![
        TraceIrInst {
            pc: 100,
            opcode: crate::OpCode::GetTabUp,
            raw_instruction: Instruction::create_abck(crate::OpCode::GetTabUp, 5, 0, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::TableAccess,
            reads: vec![TraceIrOperand::Upvalue(0), TraceIrOperand::ConstantIndex(0)],
            writes: vec![TraceIrOperand::Register(5)],
        },
        TraceIrInst {
            pc: 101,
            opcode: crate::OpCode::GetField,
            raw_instruction: Instruction::create_abck(crate::OpCode::GetField, 6, 5, 1, false)
                .as_u32(),
            kind: TraceIrInstKind::TableAccess,
            reads: vec![
                TraceIrOperand::Register(5),
                TraceIrOperand::ConstantIndex(1),
            ],
            writes: vec![TraceIrOperand::Register(6)],
        },
        TraceIrInst {
            pc: 102,
            opcode: crate::OpCode::AddI,
            raw_instruction: Instruction::create_abc(crate::OpCode::AddI, 6, 6, 128).as_u32(),
            kind: TraceIrInstKind::Arithmetic,
            reads: vec![TraceIrOperand::Register(6)],
            writes: vec![TraceIrOperand::Register(6)],
        },
        TraceIrInst {
            pc: 103,
            opcode: crate::OpCode::MmBinI,
            raw_instruction: Instruction::create_abc(crate::OpCode::MmBinI, 6, 128, 6).as_u32(),
            kind: TraceIrInstKind::MetamethodFallback,
            reads: vec![TraceIrOperand::Register(6)],
            writes: Vec::new(),
        },
        TraceIrInst {
            pc: 104,
            opcode: crate::OpCode::GetTabUp,
            raw_instruction: Instruction::create_abck(crate::OpCode::GetTabUp, 7, 0, 0, false)
                .as_u32(),
            kind: TraceIrInstKind::TableAccess,
            reads: vec![TraceIrOperand::Upvalue(0), TraceIrOperand::ConstantIndex(0)],
            writes: vec![TraceIrOperand::Register(7)],
        },
        TraceIrInst {
            pc: 105,
            opcode: crate::OpCode::SetField,
            raw_instruction: Instruction::create_abck(crate::OpCode::SetField, 7, 1, 6, false)
                .as_u32(),
            kind: TraceIrInstKind::TableAccess,
            reads: vec![
                TraceIrOperand::Register(7),
                TraceIrOperand::ConstantIndex(1),
                TraceIrOperand::Register(6),
            ],
            writes: Vec::new(),
        },
    ];

    let steps = lower_numeric_steps_for_native(&insts, &lowered_trace).unwrap();

    // The second GetTabUp should be forwarded away, and the Move should
    // be eliminated by alias resolution, leaving 4 steps:
    // GetTabUpField, GetTableField, Binary(AddI), SetTableField
    assert_eq!(steps.len(), 4);
    assert!(matches!(
        steps[0],
        NumericStep::GetTabUpField {
            dst: 5,
            upvalue: 0,
            key: 0
        }
    ));
    assert!(matches!(
        steps[1],
        NumericStep::GetTableField {
            dst: 6,
            table: 5,
            key: 1
        }
    ));
    assert!(matches!(
        steps[2],
        NumericStep::Binary {
            dst: 6,
            op: NumericBinaryOp::Add,
            ..
        }
    ));
    // After forwarding + alias: SetTableField should use the original table register
    assert!(matches!(
        steps[3],
        NumericStep::SetTableField {
            table: 5,
            key: 1,
            value: 6
        }
    ));
}

#[test]
fn extract_loop_prologue_hoists_invariant_and_cross_iter_carry() {
    // Starting steps after mid-end: [GetTabUpField(5,0,0), GetTableField(6,5,1), Binary(6,Add), SetTableField(5,1,6)]
    // LICM should hoist GetTabUpField(5,0,0) to prologue.
    // Cross-iteration forwarding should detect SetTableField(5,1,6) → GetTableField(6,5,1)
    // and hoist GetTableField to prologue as well.
    // Result: prologue = [GetTabUpField(5,0,0), GetTableField(6,5,1)],
    //         body    = [Binary(6,Add), SetTableField(5,1,6)]
    let steps = vec![
        NumericStep::GetTabUpField {
            dst: 5,
            upvalue: 0,
            key: 0,
        },
        NumericStep::GetTableField {
            dst: 6,
            table: 5,
            key: 1,
        },
        NumericStep::Binary {
            dst: 6,
            lhs: NumericOperand::Reg(6),
            rhs: NumericOperand::ImmI(1),
            op: NumericBinaryOp::Add,
        },
        NumericStep::SetTableField {
            table: 5,
            key: 1,
            value: 6,
        },
    ];

    let (prologue, body) = extract_loop_prologue(&steps);

    // Prologue should contain the hoisted steps
    assert_eq!(prologue.len(), 2, "prologue: {prologue:?}");
    assert!(matches!(
        prologue[0],
        NumericStep::GetTabUpField {
            dst: 5,
            upvalue: 0,
            key: 0
        }
    ));
    assert!(matches!(
        prologue[1],
        NumericStep::GetTableField {
            dst: 6,
            table: 5,
            key: 1
        }
    ));

    // Body should be just Binary + SetTableField
    assert_eq!(body.len(), 2, "body: {body:?}");
    assert!(matches!(
        body[0],
        NumericStep::Binary {
            dst: 6,
            op: NumericBinaryOp::Add,
            ..
        }
    ));
    assert!(matches!(
        body[1],
        NumericStep::SetTableField {
            table: 5,
            key: 1,
            value: 6
        }
    ));
}
