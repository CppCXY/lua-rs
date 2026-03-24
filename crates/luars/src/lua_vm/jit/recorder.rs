use crate::{Chunk, Instruction, OpCode};

use super::{
    JitPolicy, RecordingRequest, TraceAbortReason, TraceExit, TraceExitAction, TraceExitKind,
    TraceFallback, TraceGuard, TraceGuardKind, TraceGuardMode, TraceGuardOperands, TraceId,
    TraceInstruction, TracePlan, TraceSnapshot, TraceSnapshotKind,
};

pub struct TraceRecorder<'a> {
    policy: JitPolicy,
    chunk: &'a Chunk,
    request: RecordingRequest,
}

impl<'a> TraceRecorder<'a> {
    pub fn new(policy: JitPolicy, chunk: &'a Chunk, request: RecordingRequest) -> Self {
        Self {
            policy,
            chunk,
            request,
        }
    }

    pub fn record(&self, trace_id: TraceId) -> Result<TracePlan, TraceAbortReason> {
        if self.request.start_pc >= self.chunk.code.len() {
            return Err(TraceAbortReason::InvalidAnchor);
        }

        let mut instructions = Vec::new();
        let mut live_regs = Vec::new();
        let mut snapshots = vec![TraceSnapshot {
            kind: TraceSnapshotKind::Entry,
            pc: self.request.start_pc,
            resume_pc: self.request.current_pc,
            base: self.request.base,
            frame_depth: self.request.frame_depth,
            live_regs: Vec::new(),
        }];
        let mut guards = Vec::new();
        let mut exits = Vec::new();
        let mut pc = self.request.start_pc;
        let max_len = self.policy.max_trace_instructions as usize;

        for _ in 0..max_len {
            let instr = *self
                .chunk
                .code
                .get(pc)
                .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
            let opcode = instr.get_opcode();
            let fallback = self.paired_fallback(pc, opcode)?;
            instructions.push(TraceInstruction {
                pc,
                opcode,
                line: self.chunk.line_info.get(pc).copied(),
                fallback,
            });
            Self::collect_live_regs(&mut live_regs, instr, opcode);
            self.record_type_guards(pc, instr, opcode, &mut snapshots, &mut guards, &mut exits);

            match opcode {
                OpCode::Jmp => {
                    let target = Self::jump_target(pc, instr)?;
                    if instr.get_sj() < 0 && target == self.request.anchor_pc {
                        Self::finalize_live_regs(&mut snapshots, &live_regs);
                        return Ok(TracePlan {
                            id: trace_id,
                            chunk_key: self.request.chunk_key,
                            anchor_pc: self.request.anchor_pc,
                            end_pc: pc,
                            anchor_kind: self.request.anchor_kind,
                            instructions,
                            snapshots,
                            guards,
                            exits,
                        });
                    }
                    if target > pc {
                        pc = target;
                        continue;
                    }
                    return Err(TraceAbortReason::UnsupportedControlFlow);
                }
                OpCode::ForLoop => {
                    let target = Self::for_loop_target(pc, instr)?;
                    if target == self.request.anchor_pc {
                        let exit_snapshot_index = snapshots.len();
                        snapshots.push(TraceSnapshot {
                            kind: TraceSnapshotKind::SideExit,
                            pc,
                            resume_pc: pc + 1,
                            base: self.request.base,
                            frame_depth: self.request.frame_depth,
                            live_regs: Vec::new(),
                        });
                        exits.push(TraceExit {
                            kind: TraceExitKind::LoopExit,
                            source_pc: pc,
                            target_pc: pc + 1,
                            snapshot_index: exit_snapshot_index,
                            side_trace: None,
                            actions: Vec::new(),
                        });
                        Self::finalize_live_regs(&mut snapshots, &live_regs);
                        return Ok(TracePlan {
                            id: trace_id,
                            chunk_key: self.request.chunk_key,
                            anchor_pc: self.request.anchor_pc,
                            end_pc: pc,
                            anchor_kind: self.request.anchor_kind,
                            instructions,
                            snapshots,
                            guards,
                            exits,
                        });
                    }
                    return Err(TraceAbortReason::UnsupportedControlFlow);
                }
                OpCode::Eq
                | OpCode::EqK
                | OpCode::EqI
                | OpCode::Lt
                | OpCode::LtI
                | OpCode::LeI
                | OpCode::GtI
                | OpCode::GeI
                | OpCode::Le
                | OpCode::Test
                | OpCode::TestSet => {
                    if let Some(plan) = self.try_finish_tail_control_loop(
                        trace_id,
                        &instructions,
                        &live_regs,
                        &mut snapshots,
                        &mut guards,
                        &mut exits,
                        pc,
                        instr,
                        opcode,
                    )? {
                        return Ok(plan);
                    }
                    let (next_pc, guard, exit) =
                        self.record_conditional_guard(pc, instr, opcode, &mut snapshots)?;
                    guards.push(guard);
                    exits.push(exit);
                    pc = next_pc;
                }
                _ if Self::is_supported_linear_opcode(opcode) => {
                    pc += if fallback.is_some() { 2 } else { 1 };
                }
                _ if Self::is_side_effect_boundary(opcode) => {
                    return Err(TraceAbortReason::SideEffectBoundary);
                }
                _ => {
                    return Err(TraceAbortReason::UnsupportedOpcode);
                }
            }
        }

        Err(TraceAbortReason::TraceTooLong)
    }

    fn try_finish_tail_control_loop(
        &self,
        trace_id: TraceId,
        instructions: &[TraceInstruction],
        live_regs: &[u8],
        snapshots: &mut Vec<TraceSnapshot>,
        guards: &mut Vec<TraceGuard>,
        exits: &mut Vec<TraceExit>,
        pc: usize,
        instr: Instruction,
        opcode: OpCode,
    ) -> Result<Option<TracePlan>, TraceAbortReason> {
        let jmp_pc = pc + 1;
        let Some(jmp) = self.chunk.code.get(jmp_pc).copied() else {
            return Ok(None);
        };
        if jmp.get_opcode() != OpCode::Jmp || jmp.get_sj() >= 0 {
            return Ok(None);
        }

        let target = Self::jump_target(jmp_pc, jmp)?;
        if target != self.request.anchor_pc {
            return Ok(None);
        }

        let exit_snapshot_index = snapshots.len();
        snapshots.push(TraceSnapshot {
            kind: TraceSnapshotKind::SideExit,
            pc,
            resume_pc: jmp_pc + 1,
            base: self.request.base,
            frame_depth: self.request.frame_depth,
            live_regs: Vec::new(),
        });

        let (kind, operands, continue_when, actions) = Self::guard_metadata(instr, opcode)?;
        guards.push(TraceGuard {
            pc,
            mode: TraceGuardMode::Control,
            kind,
            operands,
            continue_when: !continue_when,
            exit_snapshot_index,
        });
        exits.push(TraceExit {
            kind: TraceExitKind::GuardExit,
            source_pc: pc,
            target_pc: jmp_pc + 1,
            snapshot_index: exit_snapshot_index,
            side_trace: None,
            actions,
        });

        Self::finalize_live_regs(snapshots, live_regs);
        Ok(Some(TracePlan {
            id: trace_id,
            chunk_key: self.request.chunk_key,
            anchor_pc: self.request.anchor_pc,
            end_pc: pc,
            anchor_kind: self.request.anchor_kind,
            instructions: instructions.to_vec(),
            snapshots: snapshots.clone(),
            guards: guards.clone(),
            exits: exits.clone(),
        }))
    }

    fn record_conditional_guard(
        &self,
        pc: usize,
        instr: Instruction,
        opcode: OpCode,
        snapshots: &mut Vec<TraceSnapshot>,
    ) -> Result<(usize, TraceGuard, TraceExit), TraceAbortReason> {
        let jmp_pc = pc + 1;
        let jmp = *self
            .chunk
            .code
            .get(jmp_pc)
            .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
        if jmp.get_opcode() != OpCode::Jmp {
            return Err(TraceAbortReason::UnsupportedControlFlow);
        }

        let exit_target = Self::jump_target(jmp_pc, jmp)?;
        let continue_pc = jmp_pc + 1;
        let exit_snapshot_index = snapshots.len();
        snapshots.push(TraceSnapshot {
            kind: TraceSnapshotKind::SideExit,
            pc,
            resume_pc: exit_target,
            base: self.request.base,
            frame_depth: self.request.frame_depth,
            live_regs: Vec::new(),
        });

        let (kind, operands, continue_when, actions) = Self::guard_metadata(instr, opcode)?;
        let guard = TraceGuard {
            pc,
            mode: TraceGuardMode::Control,
            kind,
            operands,
            continue_when,
            exit_snapshot_index,
        };
        let exit = TraceExit {
            kind: TraceExitKind::GuardExit,
            source_pc: pc,
            target_pc: exit_target,
            snapshot_index: exit_snapshot_index,
            side_trace: None,
            actions,
        };

        Ok((continue_pc, guard, exit))
    }

    fn guard_metadata(
        instr: Instruction,
        opcode: OpCode,
    ) -> Result<
        (
            TraceGuardKind,
            TraceGuardOperands,
            bool,
            Vec<TraceExitAction>,
        ),
        TraceAbortReason,
    > {
        let metadata = match opcode {
            OpCode::Eq => (
                TraceGuardKind::Eq,
                TraceGuardOperands::Registers {
                    lhs: instr.get_a() as u8,
                    rhs: instr.get_b() as u8,
                },
                !instr.get_k(),
                Vec::new(),
            ),
            OpCode::EqK => (
                TraceGuardKind::Eq,
                TraceGuardOperands::RegisterConstant {
                    reg: instr.get_a() as u8,
                    constant_index: instr.get_b() as usize,
                },
                !instr.get_k(),
                Vec::new(),
            ),
            OpCode::EqI => (
                TraceGuardKind::Eq,
                TraceGuardOperands::RegisterImmediate {
                    reg: instr.get_a() as u8,
                    imm: instr.get_sb() as i64,
                },
                !instr.get_k(),
                Vec::new(),
            ),
            OpCode::Lt => (
                TraceGuardKind::Lt,
                TraceGuardOperands::Registers {
                    lhs: instr.get_a() as u8,
                    rhs: instr.get_b() as u8,
                },
                !instr.get_k(),
                Vec::new(),
            ),
            OpCode::LtI => (
                TraceGuardKind::Lt,
                TraceGuardOperands::RegisterImmediate {
                    reg: instr.get_a() as u8,
                    imm: instr.get_sb() as i64,
                },
                !instr.get_k(),
                Vec::new(),
            ),
            OpCode::Le => (
                TraceGuardKind::Le,
                TraceGuardOperands::Registers {
                    lhs: instr.get_a() as u8,
                    rhs: instr.get_b() as u8,
                },
                !instr.get_k(),
                Vec::new(),
            ),
            OpCode::LeI => (
                TraceGuardKind::Le,
                TraceGuardOperands::RegisterImmediate {
                    reg: instr.get_a() as u8,
                    imm: instr.get_sb() as i64,
                },
                !instr.get_k(),
                Vec::new(),
            ),
            OpCode::GtI => (
                TraceGuardKind::Lt,
                TraceGuardOperands::ImmediateRegister {
                    imm: instr.get_sb() as i64,
                    reg: instr.get_a() as u8,
                },
                !instr.get_k(),
                Vec::new(),
            ),
            OpCode::GeI => (
                TraceGuardKind::Le,
                TraceGuardOperands::ImmediateRegister {
                    imm: instr.get_sb() as i64,
                    reg: instr.get_a() as u8,
                },
                !instr.get_k(),
                Vec::new(),
            ),
            OpCode::Test => (
                TraceGuardKind::Truthy,
                TraceGuardOperands::Register {
                    reg: instr.get_a() as u8,
                },
                !instr.get_k(),
                Vec::new(),
            ),
            OpCode::TestSet => (
                TraceGuardKind::Falsey,
                TraceGuardOperands::Register {
                    reg: instr.get_b() as u8,
                },
                instr.get_k(),
                vec![TraceExitAction::CopyReg {
                    dst: instr.get_a() as u8,
                    src: instr.get_b() as u8,
                }],
            ),
            _ => return Err(TraceAbortReason::UnsupportedOpcode),
        };

        Ok(metadata)
    }

    fn record_type_guards(
        &self,
        pc: usize,
        instr: Instruction,
        opcode: OpCode,
        snapshots: &mut Vec<TraceSnapshot>,
        guards: &mut Vec<TraceGuard>,
        exits: &mut Vec<TraceExit>,
    ) {
        match opcode {
            OpCode::AddI => {
                self.push_precondition_guard(
                    pc,
                    TraceGuardKind::IsNumber,
                    TraceGuardOperands::Register {
                        reg: instr.get_b() as u8,
                    },
                    snapshots,
                    guards,
                    exits,
                );
            }
            OpCode::AddK
            | OpCode::SubK
            | OpCode::MulK
            | OpCode::ModK
            | OpCode::PowK
            | OpCode::DivK
            | OpCode::IDivK => {
                self.push_precondition_guard(
                    pc,
                    TraceGuardKind::IsNumber,
                    TraceGuardOperands::Register {
                        reg: instr.get_b() as u8,
                    },
                    snapshots,
                    guards,
                    exits,
                );
            }
            OpCode::BAndK
            | OpCode::BOrK
            | OpCode::BXorK
            | OpCode::ShlI
            | OpCode::ShrI
            | OpCode::BNot => {
                self.push_precondition_guard(
                    pc,
                    TraceGuardKind::IsIntegerLike,
                    TraceGuardOperands::Register {
                        reg: instr.get_b() as u8,
                    },
                    snapshots,
                    guards,
                    exits,
                );
            }
            OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::Div
            | OpCode::IDiv => {
                for reg in [instr.get_b() as u8, instr.get_c() as u8] {
                    self.push_precondition_guard(
                        pc,
                        TraceGuardKind::IsNumber,
                        TraceGuardOperands::Register { reg },
                        snapshots,
                        guards,
                        exits,
                    );
                }
            }
            OpCode::BAnd | OpCode::BOr | OpCode::BXor | OpCode::Shl | OpCode::Shr => {
                for reg in [instr.get_b() as u8, instr.get_c() as u8] {
                    self.push_precondition_guard(
                        pc,
                        TraceGuardKind::IsIntegerLike,
                        TraceGuardOperands::Register { reg },
                        snapshots,
                        guards,
                        exits,
                    );
                }
            }
            OpCode::Unm => {
                self.push_precondition_guard(
                    pc,
                    TraceGuardKind::IsNumber,
                    TraceGuardOperands::Register {
                        reg: instr.get_b() as u8,
                    },
                    snapshots,
                    guards,
                    exits,
                );
            }
            OpCode::Eq => {
                self.push_precondition_guard(
                    pc,
                    TraceGuardKind::IsEqSafeComparable,
                    TraceGuardOperands::Registers {
                        lhs: instr.get_a() as u8,
                        rhs: instr.get_b() as u8,
                    },
                    snapshots,
                    guards,
                    exits,
                );
            }
            OpCode::LtI | OpCode::LeI | OpCode::GtI | OpCode::GeI => {
                self.push_precondition_guard(
                    pc,
                    TraceGuardKind::IsNumber,
                    TraceGuardOperands::Register {
                        reg: instr.get_a() as u8,
                    },
                    snapshots,
                    guards,
                    exits,
                );
            }
            OpCode::Lt | OpCode::Le => {
                self.push_precondition_guard(
                    pc,
                    TraceGuardKind::IsComparableLtLe,
                    TraceGuardOperands::Registers {
                        lhs: instr.get_a() as u8,
                        rhs: instr.get_b() as u8,
                    },
                    snapshots,
                    guards,
                    exits,
                );
            }
            OpCode::ForLoop => {
                for reg in [
                    instr.get_a() as u8,
                    instr.get_a() as u8 + 1,
                    instr.get_a() as u8 + 2,
                ] {
                    self.push_precondition_guard(
                        pc,
                        TraceGuardKind::IsNumber,
                        TraceGuardOperands::Register { reg },
                        snapshots,
                        guards,
                        exits,
                    );
                }
            }
            _ => {}
        }
    }

    fn push_precondition_guard(
        &self,
        pc: usize,
        kind: TraceGuardKind,
        operands: TraceGuardOperands,
        snapshots: &mut Vec<TraceSnapshot>,
        guards: &mut Vec<TraceGuard>,
        exits: &mut Vec<TraceExit>,
    ) {
        let exit_snapshot_index = snapshots.len();
        snapshots.push(TraceSnapshot {
            kind: TraceSnapshotKind::SideExit,
            pc,
            resume_pc: pc,
            base: self.request.base,
            frame_depth: self.request.frame_depth,
            live_regs: Vec::new(),
        });
        exits.push(TraceExit {
            kind: TraceExitKind::GuardExit,
            source_pc: pc,
            target_pc: pc,
            snapshot_index: exit_snapshot_index,
            side_trace: None,
            actions: Vec::new(),
        });
        guards.push(TraceGuard {
            pc,
            mode: TraceGuardMode::Precondition,
            kind,
            operands,
            continue_when: true,
            exit_snapshot_index,
        });
    }

    fn finalize_live_regs(snapshots: &mut [TraceSnapshot], live_regs: &[u8]) {
        for snapshot in snapshots {
            snapshot.live_regs = live_regs.to_vec();
        }
    }

    fn paired_fallback(
        &self,
        pc: usize,
        opcode: OpCode,
    ) -> Result<Option<TraceFallback>, TraceAbortReason> {
        let Some(fallback_opcode) = Self::expected_fallback_opcode(opcode) else {
            return Ok(None);
        };
        let Some(fallback_instr) = self.chunk.code.get(pc + 1).copied() else {
            return Ok(None);
        };
        if fallback_instr.get_opcode() != fallback_opcode {
            return Ok(None);
        }

        let fallback = match fallback_opcode {
            OpCode::MmBin => TraceFallback::MmBin {
                tm: unsafe {
                    crate::lua_vm::TmKind::from_u8_unchecked(fallback_instr.get_c() as u8)
                },
            },
            OpCode::MmBinI => TraceFallback::MmBinI {
                imm: fallback_instr.get_sb() as i64,
                tm: unsafe {
                    crate::lua_vm::TmKind::from_u8_unchecked(fallback_instr.get_c() as u8)
                },
                flip: fallback_instr.get_k(),
            },
            OpCode::MmBinK => TraceFallback::MmBinK {
                constant_index: fallback_instr.get_b() as usize,
                tm: unsafe {
                    crate::lua_vm::TmKind::from_u8_unchecked(fallback_instr.get_c() as u8)
                },
                flip: fallback_instr.get_k(),
            },
            _ => return Err(TraceAbortReason::UnsupportedOpcode),
        };

        Ok(Some(fallback))
    }

    fn expected_fallback_opcode(opcode: OpCode) -> Option<OpCode> {
        match opcode {
            OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::Div
            | OpCode::IDiv => Some(OpCode::MmBin),
            OpCode::AddI | OpCode::ShlI | OpCode::ShrI => Some(OpCode::MmBinI),
            OpCode::AddK
            | OpCode::SubK
            | OpCode::MulK
            | OpCode::ModK
            | OpCode::PowK
            | OpCode::DivK
            | OpCode::IDivK
            | OpCode::BAndK
            | OpCode::BOrK
            | OpCode::BXorK => Some(OpCode::MmBinK),
            _ => None,
        }
    }

    fn jump_target(pc: usize, instr: Instruction) -> Result<usize, TraceAbortReason> {
        let next_pc = pc
            .checked_add(1)
            .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
        let target = next_pc as isize + instr.get_sj() as isize;
        if target < 0 {
            return Err(TraceAbortReason::UnsupportedControlFlow);
        }
        Ok(target as usize)
    }

    fn for_loop_target(pc: usize, instr: Instruction) -> Result<usize, TraceAbortReason> {
        let next_pc = pc
            .checked_add(1)
            .ok_or(TraceAbortReason::UnsupportedControlFlow)?;
        next_pc
            .checked_sub(instr.get_bx() as usize)
            .ok_or(TraceAbortReason::UnsupportedControlFlow)
    }

    fn is_supported_linear_opcode(opcode: OpCode) -> bool {
        matches!(
            opcode,
            OpCode::Move
                | OpCode::LoadI
                | OpCode::LoadF
                | OpCode::LoadK
                | OpCode::LoadFalse
                | OpCode::LoadTrue
                | OpCode::LoadNil
                | OpCode::AddI
                | OpCode::AddK
                | OpCode::SubK
                | OpCode::MulK
                | OpCode::ModK
                | OpCode::PowK
                | OpCode::DivK
                | OpCode::IDivK
                | OpCode::BAndK
                | OpCode::BOrK
                | OpCode::BXorK
                | OpCode::ShlI
                | OpCode::ShrI
                | OpCode::Add
                | OpCode::Sub
                | OpCode::Mul
                | OpCode::Mod
                | OpCode::Pow
                | OpCode::Div
                | OpCode::IDiv
                | OpCode::BAnd
                | OpCode::BOr
                | OpCode::BXor
                | OpCode::Shl
                | OpCode::Shr
                | OpCode::Unm
                | OpCode::BNot
                | OpCode::Not
        )
    }

    fn collect_live_regs(live_regs: &mut Vec<u8>, instr: Instruction, opcode: OpCode) {
        match opcode {
            OpCode::Move => {
                Self::push_reg(live_regs, instr.get_a());
                Self::push_reg(live_regs, instr.get_b());
            }
            OpCode::Eq | OpCode::Lt | OpCode::Le => {
                Self::push_reg(live_regs, instr.get_a());
                Self::push_reg(live_regs, instr.get_b());
            }
            OpCode::EqK | OpCode::EqI | OpCode::LtI | OpCode::LeI | OpCode::GtI | OpCode::GeI => {
                Self::push_reg(live_regs, instr.get_a());
            }
            OpCode::Test => {
                Self::push_reg(live_regs, instr.get_a());
            }
            OpCode::TestSet => {
                Self::push_reg(live_regs, instr.get_a());
                Self::push_reg(live_regs, instr.get_b());
            }
            OpCode::LoadI
            | OpCode::LoadF
            | OpCode::LoadK
            | OpCode::LoadFalse
            | OpCode::LoadTrue => {
                Self::push_reg(live_regs, instr.get_a());
            }
            OpCode::LoadNil => {
                let start = instr.get_a();
                let count = instr.get_b();
                for reg in start..=start + count {
                    Self::push_reg(live_regs, reg);
                }
            }
            OpCode::AddI
            | OpCode::AddK
            | OpCode::SubK
            | OpCode::MulK
            | OpCode::ModK
            | OpCode::PowK
            | OpCode::DivK
            | OpCode::IDivK
            | OpCode::BAndK
            | OpCode::BOrK
            | OpCode::BXorK
            | OpCode::ShlI
            | OpCode::ShrI
            | OpCode::Unm
            | OpCode::BNot
            | OpCode::Not => {
                Self::push_reg(live_regs, instr.get_a());
                Self::push_reg(live_regs, instr.get_b());
            }
            OpCode::Add
            | OpCode::Sub
            | OpCode::Mul
            | OpCode::Mod
            | OpCode::Pow
            | OpCode::Div
            | OpCode::IDiv
            | OpCode::BAnd
            | OpCode::BOr
            | OpCode::BXor
            | OpCode::Shl
            | OpCode::Shr => {
                Self::push_reg(live_regs, instr.get_a());
                Self::push_reg(live_regs, instr.get_b());
                Self::push_reg(live_regs, instr.get_c());
            }
            OpCode::Jmp => {}
            OpCode::ForLoop => {
                let a = instr.get_a();
                Self::push_reg(live_regs, a);
                Self::push_reg(live_regs, a + 1);
                Self::push_reg(live_regs, a + 2);
            }
            _ => {}
        }
    }

    fn push_reg(live_regs: &mut Vec<u8>, reg: u32) {
        let reg = reg as u8;
        if !live_regs.contains(&reg) {
            live_regs.push(reg);
        }
    }

    fn is_side_effect_boundary(opcode: OpCode) -> bool {
        matches!(
            opcode,
            OpCode::GetUpval
                | OpCode::SetUpval
                | OpCode::GetTabUp
                | OpCode::GetTable
                | OpCode::GetI
                | OpCode::GetField
                | OpCode::SetTabUp
                | OpCode::SetTable
                | OpCode::SetI
                | OpCode::SetField
                | OpCode::NewTable
                | OpCode::Self_
                | OpCode::MmBin
                | OpCode::MmBinI
                | OpCode::MmBinK
                | OpCode::Len
                | OpCode::Concat
                | OpCode::Close
                | OpCode::Tbc
                | OpCode::Call
                | OpCode::TailCall
                | OpCode::Return
                | OpCode::Return0
                | OpCode::Return1
                | OpCode::ForPrep
                | OpCode::TForPrep
                | OpCode::TForCall
                | OpCode::TForLoop
                | OpCode::SetList
                | OpCode::Closure
                | OpCode::Vararg
                | OpCode::GetVarg
                | OpCode::ErrNNil
                | OpCode::VarargPrep
                | OpCode::LoadKX
                | OpCode::LFalseSkip
                | OpCode::ExtraArg
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{Chunk, Instruction};

    use super::*;
    use crate::lua_vm::jit::{TraceAnchorKind, TraceGuardMode, TraceId};

    fn recording_request(anchor_pc: usize) -> RecordingRequest {
        RecordingRequest {
            chunk_key: 0x1234,
            anchor_pc,
            start_pc: anchor_pc,
            current_pc: anchor_pc,
            base: 8,
            frame_depth: 2,
            anchor_kind: TraceAnchorKind::LoopBackedge,
            parent_side_trace: None,
        }
    }

    #[test]
    fn records_simple_linear_loop_trace() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_asbx(OpCode::LoadI, 0, 1),
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];
        chunk.line_info = vec![10, 11, 12];

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, recording_request(1));
        let plan = recorder.record(TraceId(7)).expect("trace should record");

        assert_eq!(plan.id, TraceId(7));
        assert_eq!(plan.anchor_pc, 1);
        assert_eq!(plan.end_pc, 2);
        assert_eq!(plan.instructions.len(), 2);
        assert_eq!(plan.instructions[0].opcode, OpCode::Add);
        assert_eq!(plan.instructions[0].line, Some(11));
        assert_eq!(plan.instructions[0].fallback, None);
        assert_eq!(plan.instructions[1].opcode, OpCode::Jmp);
        assert_eq!(plan.snapshots.len(), 3);
        assert_eq!(plan.snapshots[0].kind, TraceSnapshotKind::Entry);
        assert_eq!(plan.snapshots[0].base, 8);
        assert_eq!(plan.snapshots[0].resume_pc, 1);
        assert_eq!(plan.snapshots[0].live_regs, &[0, 1]);
        assert_eq!(plan.guards.len(), 2);
        assert!(
            plan.guards
                .iter()
                .all(|guard| guard.mode == TraceGuardMode::Precondition)
        );
        assert_eq!(plan.exits.len(), 2);
        assert!(
            plan.exits
                .iter()
                .all(|exit| exit.kind == TraceExitKind::GuardExit)
        );
    }

    #[test]
    fn records_add_with_mmbin_companion() {
        let chunk = Chunk {
            code: vec![
                Instruction::create_abc(OpCode::Add, 0, 0, 1),
                Instruction::create_abc(OpCode::MmBin, 0, 0, crate::lua_vm::TmKind::Add as u32),
                Instruction::create_sj(OpCode::Jmp, -3),
            ],
            ..Chunk::new()
        };

        let trace = TraceRecorder::new(
            JitPolicy::default(),
            &chunk,
            RecordingRequest {
                chunk_key: &chunk as *const Chunk as usize,
                anchor_pc: 0,
                start_pc: 0,
                current_pc: 0,
                base: 0,
                frame_depth: 0,
                anchor_kind: TraceAnchorKind::LoopBackedge,
                parent_side_trace: None,
            },
        )
        .record(TraceId(12))
        .expect("add trace should record across mmbin companion");

        assert_eq!(trace.instructions.len(), 2);
        assert_eq!(trace.instructions[0].opcode, OpCode::Add);
        assert_eq!(
            trace.instructions[0].fallback,
            Some(TraceFallback::MmBin {
                tm: crate::lua_vm::TmKind::Add,
            })
        );
        assert_eq!(trace.instructions[1].opcode, OpCode::Jmp);
    }

    #[test]
    fn aborts_on_side_effect_boundary() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::GetTable, 0, 1, 2),
            Instruction::create_sj(OpCode::Jmp, -2),
        ];

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, recording_request(0));
        let err = recorder.record(TraceId(1)).expect_err("trace should abort");

        assert_eq!(err, TraceAbortReason::SideEffectBoundary);
    }

    #[test]
    fn records_eq_guard_and_exit() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Eq, 1, 2, 0),
            Instruction::create_sj(OpCode::Jmp, 1),
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -4),
        ];

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, recording_request(0));
        let plan = recorder
            .record(TraceId(11))
            .expect("eq guard trace should record");

        assert_eq!(plan.guards.len(), 4);
        assert!(plan.guards.iter().any(|guard| {
            guard.mode == TraceGuardMode::Precondition
                && guard.kind == TraceGuardKind::IsEqSafeComparable
                && guard.operands == TraceGuardOperands::Registers { lhs: 1, rhs: 2 }
        }));
        assert!(plan.guards.iter().any(|guard| {
            guard.mode == TraceGuardMode::Control
                && guard.kind == TraceGuardKind::Eq
                && guard.operands == TraceGuardOperands::Registers { lhs: 1, rhs: 2 }
                && guard.continue_when
        }));
        assert!(plan.guards.iter().any(|guard| {
            guard.mode == TraceGuardMode::Precondition
                && guard.kind == TraceGuardKind::IsNumber
                && guard.operands == TraceGuardOperands::Register { reg: 0 }
        }));
        assert!(plan.guards.iter().any(|guard| {
            guard.mode == TraceGuardMode::Precondition
                && guard.kind == TraceGuardKind::IsNumber
                && guard.operands == TraceGuardOperands::Register { reg: 1 }
        }));
        assert_eq!(plan.exits.len(), 4);
        assert!(plan.exits.iter().any(|exit| exit.target_pc == 0));
        assert!(plan.exits.iter().any(|exit| exit.target_pc == 3));
    }

    #[test]
    fn records_branch_loop_with_forward_merge_jump() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::EqI, 1, 127, 0, false),
            Instruction::create_sj(OpCode::Jmp, 3),
            Instruction::create_abc(OpCode::AddI, 0, 0, 127 + 1),
            Instruction::create_abck(
                OpCode::MmBinI,
                0,
                127 + 1,
                crate::lua_vm::TmKind::Add as u32,
                false,
            ),
            Instruction::create_sj(OpCode::Jmp, 2),
            Instruction::create_abc(OpCode::AddI, 0, 0, 127 - 1),
            Instruction::create_abck(
                OpCode::MmBinI,
                0,
                127 + 1,
                crate::lua_vm::TmKind::Sub as u32,
                false,
            ),
            Instruction::create_sj(OpCode::Jmp, -8),
        ];

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, recording_request(0));
        let plan = recorder
            .record(TraceId(21))
            .expect("branch loop trace should record");

        assert_eq!(plan.instructions.len(), 4);
        assert_eq!(plan.instructions[0].pc, 0);
        assert_eq!(plan.instructions[1].pc, 2);
        assert_eq!(plan.instructions[2].pc, 4);
        assert_eq!(plan.instructions[2].opcode, OpCode::Jmp);
        assert_eq!(plan.instructions[3].pc, 7);
        assert_eq!(plan.instructions[3].opcode, OpCode::Jmp);
        assert!(plan.exits.iter().any(|exit| exit.target_pc == 5));
    }

    #[test]
    fn records_eqk_guard_and_exit() {
        let mut chunk = Chunk::new();
        chunk.constants = vec![crate::LuaValue::integer(7)];
        chunk.code = vec![
            Instruction::create_abck(OpCode::EqK, 1, 0, 0, false),
            Instruction::create_sj(OpCode::Jmp, 1),
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -4),
        ];

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, recording_request(0));
        let plan = recorder
            .record(TraceId(13))
            .expect("eqk guard trace should record");

        assert!(plan.guards.iter().any(|guard| {
            guard.mode == TraceGuardMode::Control
                && guard.kind == TraceGuardKind::Eq
                && guard.operands
                    == TraceGuardOperands::RegisterConstant {
                        reg: 1,
                        constant_index: 0,
                    }
                && guard.continue_when
        }));
    }

    #[test]
    fn records_gti_guard_with_immediate_register_order() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::GtI, 1, 127 + 5, 0, false),
            Instruction::create_sj(OpCode::Jmp, 1),
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -4),
        ];

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, recording_request(0));
        let plan = recorder
            .record(TraceId(14))
            .expect("gti guard trace should record");

        assert!(plan.guards.iter().any(|guard| {
            guard.mode == TraceGuardMode::Precondition
                && guard.kind == TraceGuardKind::IsNumber
                && guard.operands == TraceGuardOperands::Register { reg: 1 }
        }));
        assert!(plan.guards.iter().any(|guard| {
            guard.mode == TraceGuardMode::Control
                && guard.kind == TraceGuardKind::Lt
                && guard.operands == TraceGuardOperands::ImmediateRegister { imm: 5, reg: 1 }
                && guard.continue_when
        }));
    }

    #[test]
    fn records_tail_control_loop_trace() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_abc(OpCode::AddI, 1, 1, 127 + 1),
            Instruction::create_abck(OpCode::Le, 1, 2, 0, false),
            Instruction::create_sj(OpCode::Jmp, -4),
        ];

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, recording_request(0));
        let plan = recorder
            .record(TraceId(17))
            .expect("tail-control loop trace should record");

        assert_eq!(plan.instructions.len(), 3);
        assert_eq!(plan.instructions[0].opcode, OpCode::Add);
        assert_eq!(plan.instructions[1].opcode, OpCode::AddI);
        assert_eq!(plan.instructions[2].opcode, OpCode::Le);
        assert!(plan.guards.iter().any(|guard| {
            guard.mode == TraceGuardMode::Control
                && guard.kind == TraceGuardKind::Le
                && guard.operands == TraceGuardOperands::Registers { lhs: 1, rhs: 2 }
                && !guard.continue_when
        }));
        assert!(plan.exits.iter().any(|exit| {
            exit.kind == TraceExitKind::GuardExit && exit.source_pc == 2 && exit.target_pc == 4
        }));
    }

    #[test]
    fn records_testset_guard_exit_action() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abck(OpCode::TestSet, 1, 2, 0, false),
            Instruction::create_sj(OpCode::Jmp, 1),
            Instruction::create_abc(OpCode::Add, 0, 0, 1),
            Instruction::create_sj(OpCode::Jmp, -4),
        ];

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, recording_request(0));
        let plan = recorder
            .record(TraceId(12))
            .expect("testset guard trace should record");

        assert_eq!(plan.guards.len(), 3);
        assert!(plan.guards.iter().any(|guard| {
            guard.mode == TraceGuardMode::Control
                && guard.kind == TraceGuardKind::Falsey
                && guard.operands == TraceGuardOperands::Register { reg: 2 }
                && !guard.continue_when
        }));
        assert!(
            plan.guards
                .iter()
                .filter(|guard| guard.mode == TraceGuardMode::Precondition)
                .count()
                == 2
        );
        assert!(
            plan.exits
                .iter()
                .any(|exit| exit.actions == [TraceExitAction::CopyReg { dst: 1, src: 2 }])
        );
    }

    #[test]
    fn records_for_loop_exit_snapshot() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::Add, 3, 3, 4),
            Instruction::create_abx(OpCode::ForLoop, 0, 2),
        ];

        let request = RecordingRequest {
            chunk_key: 0x88,
            anchor_pc: 0,
            start_pc: 0,
            current_pc: 0,
            base: 16,
            frame_depth: 3,
            anchor_kind: TraceAnchorKind::ForLoop,
            parent_side_trace: None,
        };

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, request);
        let plan = recorder
            .record(TraceId(9))
            .expect("for-loop trace should record");

        assert_eq!(plan.exits.len(), 6);
        assert!(plan.exits.iter().any(|exit| {
            exit.kind == TraceExitKind::LoopExit
                && exit.source_pc == 1
                && exit.target_pc == 2
                && exit.actions.is_empty()
        }));
        assert_eq!(plan.snapshots.len(), 7);
        assert!(plan.snapshots.iter().any(|snapshot| {
            snapshot.kind == TraceSnapshotKind::SideExit
                && snapshot.resume_pc == 2
                && snapshot.live_regs == [3, 4, 0, 1, 2]
        }));
        assert_eq!(plan.guards.len(), 5);
        assert!(
            plan.guards
                .iter()
                .all(|guard| guard.mode == TraceGuardMode::Precondition)
        );
    }

    #[test]
    fn records_addi_with_mmbini_companion() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::AddI, 0, 0, 127 + 1),
            Instruction::create_abck(
                OpCode::MmBinI,
                0,
                127 + 1,
                crate::lua_vm::TmKind::Add as u32,
                false,
            ),
            Instruction::create_sj(OpCode::Jmp, -3),
        ];

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, recording_request(0));
        let plan = recorder
            .record(TraceId(15))
            .expect("addi trace should record across mmbini companion");

        assert_eq!(plan.instructions.len(), 2);
        assert_eq!(plan.instructions[0].opcode, OpCode::AddI);
        assert_eq!(
            plan.instructions[0].fallback,
            Some(TraceFallback::MmBinI {
                imm: 1,
                tm: crate::lua_vm::TmKind::Add,
                flip: false,
            })
        );

        assert_eq!(plan.instructions[1].opcode, OpCode::Jmp);
    }

    #[test]
    fn records_addk_with_mmbink_companion() {
        let mut chunk = Chunk::new();
        chunk.code = vec![
            Instruction::create_abc(OpCode::AddK, 0, 0, 3),
            Instruction::create_abck(
                OpCode::MmBinK,
                0,
                3,
                crate::lua_vm::TmKind::Add as u32,
                false,
            ),
            Instruction::create_sj(OpCode::Jmp, -3),
        ];

        let recorder = TraceRecorder::new(JitPolicy::default(), &chunk, recording_request(0));
        let plan = recorder
            .record(TraceId(16))
            .expect("addk trace should record across mmbink companion");

        assert_eq!(plan.instructions.len(), 2);
        assert_eq!(
            plan.instructions[0].fallback,
            Some(TraceFallback::MmBinK {
                constant_index: 3,
                tm: crate::lua_vm::TmKind::Add,
                flip: false,
            })
        );
    }
}
