/*----------------------------------------------------------------------
  New Execute Loop — Raw-Pointer Stack Base Caching

  Core idea: cache `*mut LuaValue` base pointer as a local, only refresh
  after operations that may resize the stack (calls, concat, etc.).
  This eliminates repeated Vec data-pointer reloads on every stack access.
----------------------------------------------------------------------*/

use crate::{
    GcTable,
    lua_value::{LuaValue, LUA_VFALSE, LUA_VTABLE},
    lua_vm::{
        Instruction, LuaError, LuaResult, OpCode, LuaState,
        call_info::call_status::{CIST_C, CIST_PENDING_FINISH},
        execute::helper::{
            self,
            handle_pending_ops,
            psetivalue, psetfltvalue, psetnilvalue,
            pivalue, pfltvalue, pttisinteger, pttisfloat, ptonumberns,
            luai_numpow, lua_idiv, lua_imod, lua_fmod,
            lua_shiftl, lua_shiftr, tointegerns, tointeger, setivalue,
        },
        execute::cold,
        execute::noinline,
        execute::table_ops,
        execute::call::{self, FrameAction},
        execute::return_handler,
        execute::closure_handler::handle_closure,
        execute::closure_vararg_ops,
        execute::metamethod,
        execute::hook::{hook_check_instruction, hook_on_call, hook_on_return},
    },
};

/// Cached frame context — lives on the Rust stack, not behind a pointer.
/// LLVM can allocate these in registers.
struct FrameCtx {
    base: usize,
    pc: usize,
    frame_idx: usize,
    code_ptr: *const Instruction,
    code_len: usize,
    constants_ptr: *const LuaValue,
    /// Raw pointer to stack base. Must be refreshed after any stack resize.
    sp: *mut LuaValue,
}

impl FrameCtx {
    /// Read register R[n] (relative to base)
    #[inline(always)]
    unsafe fn r(&self, n: usize) -> LuaValue {
        unsafe { *self.sp.add(self.base + n) }
    }

    /// Write register R[n]
    #[inline(always)]
    unsafe fn set_r(&self, n: usize, v: LuaValue) {
        unsafe { *self.sp.add(self.base + n) = v; }
    }

    /// Pointer to register R[n]
    #[inline(always)]
    unsafe fn rp(&self, n: usize) -> *mut LuaValue {
        unsafe { self.sp.add(self.base + n) }
    }

    /// Read constant K[n]
    #[inline(always)]
    unsafe fn k(&self, n: usize) -> LuaValue {
        unsafe { *self.constants_ptr.add(n) }
    }

    /// Fetch next instruction and advance pc
    #[inline(always)]
    unsafe fn fetch(&mut self) -> Instruction {
        let instr = unsafe { *self.code_ptr.add(self.pc) };
        self.pc += 1;
        instr
    }

    /// Refresh sp from lua_state (after possible stack resize)
    #[inline(always)]
    fn refresh_sp(&mut self, lua_state: &mut LuaState) {
        self.sp = lua_state.stack_mut().as_mut_ptr();
    }

    /// Refresh base from call_info
    #[inline(always)]
    fn refresh_base(&mut self, lua_state: &LuaState) {
        self.base = lua_state.get_call_info(self.frame_idx).base;
    }
}

pub fn lua_execute_new(lua_state: &mut LuaState, target_depth: usize) -> LuaResult<()> {
    // ===== STARTFUNC: load frame context =====
    'startfunc: loop {
        let current_depth = lua_state.call_depth();
        if current_depth <= target_depth {
            return Ok(());
        }

        let frame_idx = current_depth - 1;

        // Cold path: C frame or pending finish
        let call_status = lua_state.get_call_info(frame_idx).call_status;
        if call_status & (CIST_C | CIST_PENDING_FINISH) != 0
            && handle_pending_ops(lua_state, frame_idx)?
        {
            continue 'startfunc;
        }

        // Load frame context into locals
        let ci = lua_state.get_call_info(frame_idx);
        let chunk_ptr = ci.chunk_ptr;
        let mut chunk = unsafe { &*chunk_ptr };
        let pc_init = ci.pc as usize;

        let mut f = FrameCtx {
            base: ci.base,
            pc: pc_init,
            frame_idx,
            code_ptr: chunk.code.as_ptr(),
            code_len: chunk.code.len(),
            constants_ptr: chunk.constants.as_ptr(),
            sp: lua_state.stack_mut().as_mut_ptr(),
        };

        // oldpc for hooks
        lua_state.oldpc = if pc_init > 0 {
            (pc_init - 1) as u32
        } else if chunk.is_vararg {
            0
        } else {
            u32::MAX
        };

        let mut trap = lua_state.hook_mask != 0;

        // Call hook at function entry
        if f.pc == 0 && trap {
            let hook_mask = lua_state.hook_mask;
            if hook_mask & crate::lua_vm::LUA_MASKCALL != 0 && lua_state.allow_hook {
                hook_on_call(lua_state, hook_mask, call_status, chunk)?;
            }
            if hook_mask & crate::lua_vm::LUA_MASKCOUNT != 0 {
                lua_state.hook_count = lua_state.base_hook_count;
            }
            f.refresh_sp(lua_state);
        }

        // ===== MAINLOOP =====
        loop {
            let instr = unsafe { f.fetch() };

            if trap {
                hook_check_instruction(lua_state, f.pc, chunk, f.frame_idx)?;
                trap = lua_state.hook_mask != 0;
                f.refresh_sp(lua_state);
            }

            match instr.get_opcode() {
                OpCode::Move => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    unsafe { f.set_r(a, f.r(b)) };
                }
                OpCode::LoadI => {
                    let a = instr.get_a() as usize;
                    let sbx = instr.get_sbx();
                    unsafe { f.set_r(a, LuaValue::integer(sbx as i64)) };
                }
                OpCode::LoadF => {
                    let a = instr.get_a() as usize;
                    let sbx = instr.get_sbx();
                    unsafe { f.set_r(a, LuaValue::float(sbx as f64)) };
                }
                OpCode::LoadK => {
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;
                    unsafe { f.set_r(a, f.k(bx)) };
                }
                OpCode::LoadFalse => {
                    let a = instr.get_a() as usize;
                    unsafe { f.set_r(a, LuaValue::boolean(false)) };
                }
                OpCode::LFalseSkip => {
                    let a = instr.get_a() as usize;
                    unsafe { f.set_r(a, LuaValue::boolean(false)) };
                    f.pc += 1;
                }
                OpCode::LoadTrue => {
                    let a = instr.get_a() as usize;
                    unsafe { f.set_r(a, LuaValue::boolean(true)) };
                }
                OpCode::LoadNil => {
                    let a = instr.get_a() as usize;
                    let mut b = instr.get_b() as usize;
                    let mut idx = a;
                    loop {
                        unsafe { psetnilvalue(f.rp(idx)) };
                        if b == 0 { break; }
                        b -= 1;
                        idx += 1;
                    }
                }
                OpCode::Jmp => {
                    let sj = instr.get_sj();
                    f.pc = (f.pc as isize + sj as isize) as usize;
                }

                // ===== ARITHMETIC (register-register) =====
                OpCode::Add => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.rp(c) as *const LuaValue;
                        let ra = f.rp(a);
                        if pttisinteger(v1) && pttisinteger(v2) {
                            psetivalue(ra, pivalue(v1).wrapping_add(pivalue(v2)));
                            f.pc += 1;
                        } else if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, pfltvalue(v1) + pfltvalue(v2));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, n1 + n2);
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::Sub => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.rp(c) as *const LuaValue;
                        let ra = f.rp(a);
                        if pttisinteger(v1) && pttisinteger(v2) {
                            psetivalue(ra, pivalue(v1).wrapping_sub(pivalue(v2)));
                            f.pc += 1;
                        } else if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, pfltvalue(v1) - pfltvalue(v2));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, n1 - n2);
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::Mul => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.rp(c) as *const LuaValue;
                        let ra = f.rp(a);
                        if pttisinteger(v1) && pttisinteger(v2) {
                            psetivalue(ra, pivalue(v1).wrapping_mul(pivalue(v2)));
                            f.pc += 1;
                        } else if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, pfltvalue(v1) * pfltvalue(v2));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, n1 * n2);
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::AddI => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let sc = instr.get_sc();
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let ra = f.rp(a);
                        if pttisinteger(v1) {
                            psetivalue(ra, pivalue(v1).wrapping_add(sc as i64));
                            f.pc += 1;
                        } else if pttisfloat(v1) {
                            psetfltvalue(ra, pfltvalue(v1) + sc as f64);
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            if ptonumberns(v1, &mut n1) {
                                psetfltvalue(ra, n1 + sc as f64);
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::Unm => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let ra = f.rp(a);
                        if pttisinteger(v1) {
                            psetivalue(ra, (pivalue(v1) as u64).wrapping_neg() as i64);
                        } else {
                            let mut n1 = 0.0;
                            if ptonumberns(v1, &mut n1) {
                                psetfltvalue(ra, -n1);
                            } else {
                                // Metamethod fallback
                                let v1 = f.r(b);
                                lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                                if cold::try_push_unary_mm_frame(
                                    lua_state,
                                    v1,
                                    super::metamethod::TmKind::Unm,
                                    f.frame_idx,
                                )? {
                                    continue 'startfunc;
                                }
                                match super::metamethod::try_unary_tm(
                                    lua_state,
                                    v1,
                                    f.base + a,
                                    super::metamethod::TmKind::Unm,
                                ) {
                                    Ok(_) => {}
                                    Err(LuaError::Yield) => {
                                        let ci = lua_state.get_call_info_mut(f.frame_idx);
                                        ci.call_status |= CIST_PENDING_FINISH;
                                        return Err(LuaError::Yield);
                                    }
                                    Err(e) => return Err(e),
                                }
                                f.refresh_base(lua_state);
                                f.refresh_sp(lua_state);
                            }
                        }
                    }
                }

                // ===== Div (float-only) =====
                OpCode::Div => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.rp(c) as *const LuaValue;
                        let ra = f.rp(a);
                        if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, pfltvalue(v1) / pfltvalue(v2));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, n1 / n2);
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::IDiv => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.rp(c) as *const LuaValue;
                        let ra = f.rp(a);
                        if pttisinteger(v1) && pttisinteger(v2) {
                            let i2 = pivalue(v2);
                            if i2 != 0 {
                                psetivalue(ra, lua_idiv(pivalue(v1), i2));
                                f.pc += 1;
                            } else {
                                lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                                return Err(cold::error_div_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, (pfltvalue(v1) / pfltvalue(v2)).floor());
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, (n1 / n2).floor());
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::Mod => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.rp(c) as *const LuaValue;
                        let ra = f.rp(a);
                        if pttisinteger(v1) && pttisinteger(v2) {
                            let i2 = pivalue(v2);
                            if i2 != 0 {
                                psetivalue(ra, lua_imod(pivalue(v1), i2));
                                f.pc += 1;
                            } else {
                                lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                                return Err(cold::error_mod_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, lua_fmod(pfltvalue(v1), pfltvalue(v2)));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, lua_fmod(n1, n2));
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::Pow => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.rp(c) as *const LuaValue;
                        let ra = f.rp(a);
                        if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, luai_numpow(pfltvalue(v1), pfltvalue(v2)));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, luai_numpow(n1, n2));
                                f.pc += 1;
                            }
                        }
                    }
                }

                // ===== K-variant arithmetic =====
                OpCode::AddK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.constants_ptr.add(c);
                        let ra = f.rp(a);
                        if pttisinteger(v1) && pttisinteger(v2) {
                            psetivalue(ra, pivalue(v1).wrapping_add(pivalue(v2)));
                            f.pc += 1;
                        } else if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, pfltvalue(v1) + pfltvalue(v2));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, n1 + n2);
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::SubK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.constants_ptr.add(c);
                        let ra = f.rp(a);
                        if pttisinteger(v1) && pttisinteger(v2) {
                            psetivalue(ra, pivalue(v1).wrapping_sub(pivalue(v2)));
                            f.pc += 1;
                        } else if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, pfltvalue(v1) - pfltvalue(v2));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, n1 - n2);
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::MulK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.constants_ptr.add(c);
                        let ra = f.rp(a);
                        if pttisinteger(v1) && pttisinteger(v2) {
                            psetivalue(ra, pivalue(v1).wrapping_mul(pivalue(v2)));
                            f.pc += 1;
                        } else if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, pfltvalue(v1) * pfltvalue(v2));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, n1 * n2);
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::ModK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.constants_ptr.add(c);
                        let ra = f.rp(a);
                        if pttisinteger(v1) && pttisinteger(v2) {
                            let i2 = pivalue(v2);
                            if i2 != 0 {
                                psetivalue(ra, lua_imod(pivalue(v1), i2));
                                f.pc += 1;
                            } else {
                                lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                                return Err(cold::error_mod_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, lua_fmod(pfltvalue(v1), pfltvalue(v2)));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, lua_fmod(n1, n2));
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::PowK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.constants_ptr.add(c);
                        let ra = f.rp(a);
                        if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, luai_numpow(pfltvalue(v1), pfltvalue(v2)));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, luai_numpow(n1, n2));
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::DivK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.constants_ptr.add(c);
                        let ra = f.rp(a);
                        if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, pfltvalue(v1) / pfltvalue(v2));
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, n1 / n2);
                                f.pc += 1;
                            }
                        }
                    }
                }
                OpCode::IDivK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let v1 = f.rp(b) as *const LuaValue;
                        let v2 = f.constants_ptr.add(c);
                        let ra = f.rp(a);
                        if pttisinteger(v1) && pttisinteger(v2) {
                            let i2 = pivalue(v2);
                            if i2 != 0 {
                                psetivalue(ra, lua_idiv(pivalue(v1), i2));
                                f.pc += 1;
                            } else {
                                lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                                return Err(cold::error_div_by_zero(lua_state));
                            }
                        } else if pttisfloat(v1) && pttisfloat(v2) {
                            psetfltvalue(ra, (pfltvalue(v1) / pfltvalue(v2)).floor());
                            f.pc += 1;
                        } else {
                            let mut n1 = 0.0;
                            let mut n2 = 0.0;
                            if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
                                psetfltvalue(ra, (n1 / n2).floor());
                                f.pc += 1;
                            }
                        }
                    }
                }

                // ===== BITWISE (register-register) =====
                OpCode::BAnd => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let mut i1 = 0i64;
                        let mut i2 = 0i64;
                        if tointegerns(&*f.rp(b), &mut i1) && tointegerns(&*f.rp(c), &mut i2) {
                            psetivalue(f.rp(a), i1 & i2);
                            f.pc += 1;
                        }
                    }
                }
                OpCode::BOr => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let mut i1 = 0i64;
                        let mut i2 = 0i64;
                        if tointegerns(&*f.rp(b), &mut i1) && tointegerns(&*f.rp(c), &mut i2) {
                            psetivalue(f.rp(a), i1 | i2);
                            f.pc += 1;
                        }
                    }
                }
                OpCode::BXor => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let mut i1 = 0i64;
                        let mut i2 = 0i64;
                        if tointegerns(&*f.rp(b), &mut i1) && tointegerns(&*f.rp(c), &mut i2) {
                            psetivalue(f.rp(a), i1 ^ i2);
                            f.pc += 1;
                        }
                    }
                }
                OpCode::Shl => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let mut i1 = 0i64;
                        let mut i2 = 0i64;
                        if tointegerns(&*f.rp(b), &mut i1) && tointegerns(&*f.rp(c), &mut i2) {
                            psetivalue(f.rp(a), lua_shiftl(i1, i2));
                            f.pc += 1;
                        }
                    }
                }
                OpCode::Shr => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let mut i1 = 0i64;
                        let mut i2 = 0i64;
                        if tointegerns(&*f.rp(b), &mut i1) && tointegerns(&*f.rp(c), &mut i2) {
                            psetivalue(f.rp(a), lua_shiftr(i1, i2));
                            f.pc += 1;
                        }
                    }
                }

                // ===== BITWISE K-variants =====
                OpCode::BAndK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let mut i1 = 0i64;
                        let mut i2 = 0i64;
                        if tointegerns(&*f.rp(b), &mut i1) && tointeger(&*f.constants_ptr.add(c), &mut i2) {
                            psetivalue(f.rp(a), i1 & i2);
                            f.pc += 1;
                        }
                    }
                }
                OpCode::BOrK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let mut i1 = 0i64;
                        let mut i2 = 0i64;
                        if tointegerns(&*f.rp(b), &mut i1) && tointeger(&*f.constants_ptr.add(c), &mut i2) {
                            psetivalue(f.rp(a), i1 | i2);
                            f.pc += 1;
                        }
                    }
                }
                OpCode::BXorK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    unsafe {
                        let mut i1 = 0i64;
                        let mut i2 = 0i64;
                        if tointegerns(&*f.rp(b), &mut i1) && tointeger(&*f.constants_ptr.add(c), &mut i2) {
                            psetivalue(f.rp(a), i1 ^ i2);
                            f.pc += 1;
                        }
                    }
                }

                // ===== SHIFT immediate =====
                OpCode::ShlI => {
                    // sC << R[B]
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc();
                    unsafe {
                        let mut ib = 0i64;
                        if tointegerns(&*f.rp(b), &mut ib) {
                            psetivalue(f.rp(a), lua_shiftl(ic as i64, ib));
                            f.pc += 1;
                        }
                    }
                }
                OpCode::ShrI => {
                    // R[B] >> sC
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ic = instr.get_sc();
                    unsafe {
                        let mut ib = 0i64;
                        if tointegerns(&*f.rp(b), &mut ib) {
                            psetivalue(f.rp(a), lua_shiftr(ib, ic as i64));
                            f.pc += 1;
                        }
                    }
                }

                // ===== BNot (unary bitwise NOT) =====
                OpCode::BNot => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    unsafe {
                        let mut ib = 0i64;
                        if tointegerns(&*f.rp(b), &mut ib) {
                            psetivalue(f.rp(a), !ib);
                        } else {
                            // Metamethod fallback
                            let v1 = f.r(b);
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if cold::try_push_unary_mm_frame(
                                lua_state,
                                v1,
                                super::metamethod::TmKind::Bnot,
                                f.frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                            match super::metamethod::try_unary_tm(
                                lua_state,
                                v1,
                                f.base + a,
                                super::metamethod::TmKind::Bnot,
                            ) {
                                Ok(_) => {}
                                Err(crate::lua_vm::LuaError::Yield) => {
                                    let ci = lua_state.get_call_info_mut(f.frame_idx);
                                    ci.call_status |= CIST_PENDING_FINISH;
                                    return Err(crate::lua_vm::LuaError::Yield);
                                }
                                Err(e) => return Err(e),
                            }
                            f.refresh_base(lua_state);
                            f.refresh_sp(lua_state);
                        }
                    }
                }

                // ===== TEST / TESTSET =====
                OpCode::Test => {
                    let a = instr.get_a() as usize;
                    let k = instr.get_k();
                    let ra = unsafe { f.r(a) };
                    let is_false = ra.is_nil() || ra.tt() == LUA_VFALSE;
                    let cond = !is_false;
                    if cond != k {
                        f.pc += 1;
                    } else {
                        let next_instr = unsafe { *f.code_ptr.add(f.pc) };
                        f.pc += 1;
                        let sj = next_instr.get_sj();
                        f.pc = (f.pc as isize + sj as isize) as usize;
                    }
                }
                OpCode::TestSet => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();
                    let rb = unsafe { f.r(b) };
                    let is_false = rb.is_nil() || (rb.is_boolean() && rb.tt() == LUA_VFALSE);
                    if is_false == k {
                        f.pc += 1;
                    } else {
                        unsafe { f.set_r(a, rb) };
                        let next_instr = unsafe { *f.code_ptr.add(f.pc) };
                        f.pc += 1;
                        let sj = next_instr.get_sj();
                        f.pc = (f.pc as isize + sj as isize) as usize;
                    }
                }
                OpCode::Not => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let rb = unsafe { f.r(b) };
                    let is_false = rb.tt() == LUA_VFALSE || rb.is_nil();
                    unsafe { f.set_r(a, LuaValue::boolean(is_false)) };
                }

                // ===== FORLOOP (integer fast path) =====
                OpCode::ForLoop => {
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;
                    unsafe {
                        let ra = f.rp(a);
                        if pttisinteger(ra.add(1) as *const LuaValue) {
                            let count = pivalue(ra as *const LuaValue) as u64;
                            if count > 0 {
                                let step = pivalue(ra.add(1) as *const LuaValue);
                                let idx = pivalue(ra.add(2) as *const LuaValue);
                                (*ra).value.i = (count - 1) as i64;
                                (*ra.add(2)).value.i = idx.wrapping_add(step);
                                f.pc -= bx;
                            }
                        } else {
                            let step = pfltvalue(ra.add(1) as *const LuaValue);
                            let limit = pfltvalue(ra as *const LuaValue);
                            let idx = pfltvalue(ra.add(2) as *const LuaValue);
                            let new_idx = idx + step;
                            let cont = if step > 0.0 { new_idx <= limit } else { new_idx >= limit };
                            if cont {
                                (*ra.add(2)).value.n = new_idx;
                                f.pc -= bx;
                            }
                        }
                    }
                }

                // ===== UPVALUE ACCESS =====
                OpCode::GetUpval => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let ci = lua_state.get_call_info(f.frame_idx);
                    let upvalues = unsafe { ci.func.as_lua_function_unchecked().upvalues() };
                    let value = upvalues[b].as_ref().data.get_value();
                    unsafe { f.set_r(a, value) };
                }
                OpCode::SetUpval => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let value = unsafe { f.r(a) };
                    let ci = lua_state.get_call_info(f.frame_idx);
                    let upvalues = unsafe { ci.func.as_lua_function_unchecked().upvalues() };
                    let upval_ptr = upvalues[b];
                    upval_ptr.as_mut_ref().data.set_value(value);
                    if value.is_collectable() {
                        if let Some(gc_ptr) = value.as_gc_ptr() {
                            lua_state.gc_barrier(upval_ptr, gc_ptr);
                        }
                    }
                }

                // ===== RETURN =====
                OpCode::Return0 => {
                    if lua_state.hook_mask & crate::lua_vm::LUA_MASKRET != 0
                        && lua_state.allow_hook
                    {
                        hook_on_return(lua_state, f.frame_idx, f.pc as u32, 0)?;
                        f.refresh_sp(lua_state);
                    }
                    super::return_handler::handle_return0(lua_state, f.frame_idx);

                    let new_depth = lua_state.call_depth();
                    if new_depth <= target_depth {
                        return Ok(());
                    }
                    f.frame_idx = new_depth - 1;
                    let cs = lua_state.get_call_info(f.frame_idx).call_status;
                    if cs & (CIST_C | CIST_PENDING_FINISH) != 0 {
                        continue 'startfunc;
                    }
                    // Inline context restore
                    let ci = lua_state.get_call_info(f.frame_idx);
                    f.base = ci.base;
                    f.pc = ci.pc as usize;
                    chunk = unsafe { &*ci.chunk_ptr };
                    f.code_ptr = chunk.code.as_ptr();
                    f.code_len = chunk.code.len();
                    f.constants_ptr = chunk.constants.as_ptr();
                    f.sp = lua_state.stack_mut().as_mut_ptr();
                    if lua_state.hook_mask != 0 {
                        lua_state.oldpc = (f.pc - 1) as u32;
                    }
                    trap = lua_state.hook_mask != 0;
                }
                OpCode::Return1 => {
                    let a = instr.get_a() as usize;
                    if lua_state.hook_mask & crate::lua_vm::LUA_MASKRET != 0
                        && lua_state.allow_hook
                    {
                        hook_on_return(lua_state, f.frame_idx, f.pc as u32, 1)?;
                        f.refresh_sp(lua_state);
                    }
                    super::return_handler::handle_return1(lua_state, f.base, f.frame_idx, a);

                    let new_depth = lua_state.call_depth();
                    if new_depth <= target_depth {
                        return Ok(());
                    }
                    f.frame_idx = new_depth - 1;
                    let cs = lua_state.get_call_info(f.frame_idx).call_status;
                    if cs & (CIST_C | CIST_PENDING_FINISH) != 0 {
                        continue 'startfunc;
                    }
                    let ci = lua_state.get_call_info(f.frame_idx);
                    f.base = ci.base;
                    f.pc = ci.pc as usize;
                    chunk = unsafe { &*ci.chunk_ptr };
                    f.code_ptr = chunk.code.as_ptr();
                    f.code_len = chunk.code.len();
                    f.constants_ptr = chunk.constants.as_ptr();
                    f.sp = lua_state.stack_mut().as_mut_ptr();
                    if lua_state.hook_mask != 0 {
                        lua_state.oldpc = (f.pc - 1) as u32;
                    }
                    trap = lua_state.hook_mask != 0;
                }

                // ===== CALL (hot path: inline Lua function call) =====
                OpCode::Call => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let func_idx = f.base + a;

                    let func = unsafe { *f.sp.add(func_idx) };
                    if func.is_lua_function() {
                        let nargs = if b != 0 {
                            b - 1
                        } else {
                            let current_top = lua_state.get_top();
                            if current_top > func_idx + 1 { current_top - func_idx - 1 } else { 0 }
                        };
                        let nresults = if c == 0 { -1 } else { (c - 1) as i32 };

                        let lua_func = unsafe { func.as_lua_function_unchecked() };
                        let new_chunk_ptr = lua_func.chunk() as *const crate::lua_value::Chunk;
                        let new_base = func_idx + 1;

                        // Read param_count/max_stack_size before push_lua_frame
                        let param_count = unsafe { (*new_chunk_ptr).param_count };
                        let max_stack_size = unsafe { (*new_chunk_ptr).max_stack_size };

                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        lua_state.push_lua_frame(
                            &func, new_base, nargs, nresults,
                            param_count, max_stack_size, new_chunk_ptr,
                        )?;

                        // Inline callee entry — derive chunk from raw pointer
                        // (func's borrow is dead here; chunk_ptr outlives it)
                        f.frame_idx = lua_state.call_depth() - 1;
                        f.base = new_base;
                        chunk = unsafe { &*new_chunk_ptr };
                        f.code_ptr = chunk.code.as_ptr();
                        f.code_len = chunk.code.len();
                        f.constants_ptr = chunk.constants.as_ptr();
                        f.pc = 0;
                        f.sp = lua_state.stack_mut().as_mut_ptr();

                        if lua_state.hook_mask & crate::lua_vm::LUA_MASKCALL != 0
                            && lua_state.allow_hook
                        {
                            hook_on_call(lua_state, lua_state.hook_mask, 0, chunk)?;
                            f.refresh_sp(lua_state);
                        }
                        if lua_state.hook_mask != 0 {
                            lua_state.oldpc = if chunk.is_vararg { 0 } else { u32::MAX };
                        }
                        trap = lua_state.hook_mask != 0;
                        continue; // stay in mainloop
                    }

                    // Semi-cold path: __call metamethod on table
                    if func.ttistable() {
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        if cold::handle_call_metamethod(lua_state, func, func_idx, b, c)? {
                            continue 'startfunc;
                        }
                    }

                    // Cold path: C function or non-table __call metamethod
                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    match call::handle_call(lua_state, f.base, a, b, c, 0) {
                        Ok(FrameAction::Continue) => {
                            f.refresh_base(lua_state);
                            f.refresh_sp(lua_state);
                            lua_state.oldpc = (f.pc - 1) as u32;
                            trap = lua_state.hook_mask != 0;
                        }
                        Ok(FrameAction::Call) | Ok(FrameAction::TailCall) => {
                            continue 'startfunc;
                        }
                        Err(e) => return Err(e),
                    }
                }

                // ===== COMPARISON (Eq, Lt, Le with inline fast paths) =====
                OpCode::Eq => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();
                    let ra = unsafe { f.r(a) };
                    let rb = unsafe { f.r(b) };
                    if ra == rb {
                        if !k { f.pc += 1; }
                    } else if ra.tt() != rb.tt() {
                        if k { f.pc += 1; }
                    } else if ra.ttistable() || ra.ttisfulluserdata() {
                        if ra.tt == LUA_VTABLE {
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if super::noinline::try_comp_meta_table(
                                lua_state, ra, ra, rb,
                                super::metamethod::TmKind::Eq, f.frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        if cold::try_push_eq_mm_frame(lua_state, ra, rb, f.frame_idx)? {
                            continue 'startfunc;
                        }
                        let mut pc_idx = f.pc;
                        super::comparison_ops::exec_eq(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                        f.pc = pc_idx;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                    } else {
                        let mut pc_idx = f.pc;
                        super::comparison_ops::exec_eq(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                        f.pc = pc_idx;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                    }
                }
                OpCode::Lt => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();
                    let ra = unsafe { f.r(a) };
                    let rb = unsafe { f.r(b) };
                    if ra.ttisinteger() && rb.ttisinteger() {
                        if (ra.ivalue() < rb.ivalue()) != k { f.pc += 1; }
                    } else if ra.ttisfloat() && rb.ttisfloat() {
                        if (ra.fltvalue() < rb.fltvalue()) != k { f.pc += 1; }
                    } else if ra.tt == LUA_VTABLE {
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        if super::noinline::try_comp_meta_table(
                            lua_state, ra, ra, rb,
                            super::metamethod::TmKind::Lt, f.frame_idx,
                        )? {
                            continue 'startfunc;
                        }
                        let mut pc_idx = f.pc;
                        super::comparison_ops::exec_lt(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                        f.pc = pc_idx;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                    } else {
                        if rb.tt == LUA_VTABLE {
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if cold::try_push_comp_mm_frame(
                                lua_state, ra, rb,
                                super::metamethod::TmKind::Lt, f.frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = f.pc;
                        super::comparison_ops::exec_lt(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                        f.pc = pc_idx;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                    }
                }
                OpCode::Le => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();
                    let ra = unsafe { f.r(a) };
                    let rb = unsafe { f.r(b) };
                    if ra.ttisinteger() && rb.ttisinteger() {
                        if (ra.ivalue() <= rb.ivalue()) != k { f.pc += 1; }
                    } else if ra.ttisfloat() && rb.ttisfloat() {
                        if (ra.fltvalue() <= rb.fltvalue()) != k { f.pc += 1; }
                    } else if ra.tt == LUA_VTABLE {
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        if super::noinline::try_comp_meta_table(
                            lua_state, ra, ra, rb,
                            super::metamethod::TmKind::Le, f.frame_idx,
                        )? {
                            continue 'startfunc;
                        }
                        let mut pc_idx = f.pc;
                        super::comparison_ops::exec_le(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                        f.pc = pc_idx;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                    } else {
                        if rb.tt == LUA_VTABLE {
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if cold::try_push_comp_mm_frame(
                                lua_state, ra, rb,
                                super::metamethod::TmKind::Le, f.frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = f.pc;
                        super::comparison_ops::exec_le(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                        f.pc = pc_idx;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                    }
                }
                OpCode::EqK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let k = instr.get_k();
                    let ra = unsafe { f.r(a) };
                    let kb = unsafe { f.k(b) };
                    if (ra == kb) != k { f.pc += 1; }
                }
                OpCode::EqI => {
                    let a = instr.get_a() as usize;
                    let sb = instr.get_sb();
                    let k = instr.get_k();
                    let ra = unsafe { f.r(a) };
                    let cond = if ra.ttisinteger() {
                        ra.ivalue() == (sb as i64)
                    } else if ra.ttisfloat() {
                        ra.fltvalue() == (sb as f64)
                    } else {
                        false
                    };
                    if cond != k { f.pc += 1; }
                }
                OpCode::LtI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();
                    let ra = unsafe { f.r(a) };
                    if ra.ttisinteger() {
                        if (ra.ivalue() < (im as i64)) != k { f.pc += 1; }
                    } else if ra.ttisfloat() {
                        if (ra.fltvalue() < (im as f64)) != k { f.pc += 1; }
                    } else {
                        if ra.tt == LUA_VTABLE {
                            let isf = instr.get_c() != 0;
                            let imm_val = if isf { LuaValue::float(im as f64) } else { LuaValue::integer(im as i64) };
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if super::noinline::try_comp_meta_table(
                                lua_state, ra, ra, imm_val,
                                super::metamethod::TmKind::Lt, f.frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = f.pc;
                        super::comparison_ops::exec_lti(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                        f.pc = pc_idx;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                    }
                }
                OpCode::LeI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();
                    let ra = unsafe { f.r(a) };
                    if ra.ttisinteger() {
                        if (ra.ivalue() <= (im as i64)) != k { f.pc += 1; }
                    } else if ra.ttisfloat() {
                        if (ra.fltvalue() <= (im as f64)) != k { f.pc += 1; }
                    } else {
                        if ra.tt == LUA_VTABLE {
                            let isf = instr.get_c() != 0;
                            let imm_val = if isf { LuaValue::float(im as f64) } else { LuaValue::integer(im as i64) };
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if super::noinline::try_comp_meta_table(
                                lua_state, ra, ra, imm_val,
                                super::metamethod::TmKind::Le, f.frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = f.pc;
                        super::comparison_ops::exec_lei(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                        f.pc = pc_idx;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                    }
                }
                OpCode::GtI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();
                    let ra = unsafe { f.r(a) };
                    if ra.ttisinteger() {
                        if (ra.ivalue() > (im as i64)) != k { f.pc += 1; }
                    } else if ra.ttisfloat() {
                        if (ra.fltvalue() > (im as f64)) != k { f.pc += 1; }
                    } else {
                        if ra.tt == LUA_VTABLE {
                            let isf = instr.get_c() != 0;
                            let imm_val = if isf { LuaValue::float(im as f64) } else { LuaValue::integer(im as i64) };
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if super::noinline::try_comp_meta_table(
                                lua_state, ra, imm_val, ra,
                                super::metamethod::TmKind::Lt, f.frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = f.pc;
                        super::comparison_ops::exec_gti(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                        f.pc = pc_idx;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                    }
                }
                OpCode::GeI => {
                    let a = instr.get_a() as usize;
                    let im = instr.get_sb();
                    let k = instr.get_k();
                    let ra = unsafe { f.r(a) };
                    if ra.ttisinteger() {
                        if (ra.ivalue() >= (im as i64)) != k { f.pc += 1; }
                    } else if ra.ttisfloat() {
                        if (ra.fltvalue() >= (im as f64)) != k { f.pc += 1; }
                    } else {
                        if ra.tt == LUA_VTABLE {
                            let isf = instr.get_c() != 0;
                            let imm_val = if isf { LuaValue::float(im as f64) } else { LuaValue::integer(im as i64) };
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if super::noinline::try_comp_meta_table(
                                lua_state, ra, imm_val, ra,
                                super::metamethod::TmKind::Le, f.frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                        let mut pc_idx = f.pc;
                        super::comparison_ops::exec_gei(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                        f.pc = pc_idx;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                    }
                }

                // ============================================================
                // TABLE OPCODES
                // ============================================================
                OpCode::NewTable => {
                    let a = instr.get_a() as usize;
                    let vb = instr.get_vb() as usize;
                    let mut vc = instr.get_vc() as usize;
                    let k = instr.get_k();

                    let hash_size = if vb > 0 {
                        if vb > 31 { 0 } else { 1usize << (vb - 1) }
                    } else {
                        0
                    };

                    if k && f.pc < f.code_len {
                        let extra_instr = unsafe { *f.code_ptr.add(f.pc) };
                        if extra_instr.get_opcode() == OpCode::ExtraArg {
                            vc += extra_instr.get_ax() as usize * 1024;
                        }
                    }

                    f.pc += 1; // skip EXTRAARG

                    let value = lua_state.create_table(vc, hash_size)?;
                    unsafe { f.set_r(a, value) };

                    let new_top = f.base + a + 1;
                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    lua_state.set_top_raw(new_top);
                    lua_state.check_gc()?;
                    let frame_top = lua_state.get_call_info(f.frame_idx).top;
                    lua_state.set_top_raw(frame_top);
                    f.refresh_sp(lua_state);
                }

                OpCode::GetTable => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let rb = unsafe { f.r(b) };
                    let rc = unsafe { f.r(c) };

                    if rb.tt == LUA_VTABLE {
                        let table_gc = unsafe { &*(rb.value.ptr as *const GcTable) };
                        let table_ref = &table_gc.data;
                        let result = if rc.ttisinteger() {
                            table_ref.impl_table.fast_geti(rc.ivalue())
                        } else {
                            table_ref.impl_table.raw_get(&rc)
                        };
                        if let Some(val) = result {
                            unsafe { f.set_r(a, val) };
                            continue;
                        }
                        let meta = table_ref.meta_ptr();
                        if meta.is_null() {
                            unsafe { f.set_r(a, LuaValue::nil()) };
                            continue;
                        }
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        match noinline::try_index_meta_generic(lua_state, meta, rb, rc, f.frame_idx)? {
                            noinline::IndexResult::Found(val) => {
                                unsafe { f.set_r(a, val) };
                                continue;
                            }
                            noinline::IndexResult::CallMm => continue 'startfunc,
                            noinline::IndexResult::FallThrough => {}
                        }
                    }

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    let mut pc_idx = f.pc;
                    table_ops::exec_gettable(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                    f.pc = pc_idx;
                    f.refresh_base(lua_state);
                    f.refresh_sp(lua_state);
                }

                OpCode::GetI => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as i64;

                    let rb = unsafe { f.r(b) };

                    if rb.tt == LUA_VTABLE {
                        let table_gc = unsafe { &*(rb.value.ptr as *const GcTable) };
                        let table_ref = &table_gc.data;
                        if let Some(val) = table_ref.impl_table.fast_geti(c) {
                            unsafe { f.set_r(a, val) };
                            continue;
                        }
                        let meta = table_ref.meta_ptr();
                        if meta.is_null() {
                            unsafe { f.set_r(a, LuaValue::nil()) };
                            continue;
                        }
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        match noinline::try_index_meta_int(lua_state, meta, rb, c, f.frame_idx)? {
                            noinline::IndexResult::Found(val) => {
                                unsafe { f.set_r(a, val) };
                                continue;
                            }
                            noinline::IndexResult::CallMm => continue 'startfunc,
                            noinline::IndexResult::FallThrough => {}
                        }
                    }

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    let mut pc_idx = f.pc;
                    table_ops::exec_geti(lua_state, instr, f.base, f.frame_idx, &mut pc_idx)?;
                    f.pc = pc_idx;
                    f.refresh_base(lua_state);
                    f.refresh_sp(lua_state);
                }

                OpCode::GetField => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let rb = unsafe { f.r(b) };
                    let key = unsafe { &*f.constants_ptr.add(c) };

                    if rb.tt == LUA_VTABLE {
                        let table_gc = unsafe { &*(rb.value.ptr as *const GcTable) };
                        let table_ref = &table_gc.data;
                        if let Some(val) = table_ref.impl_table.get_shortstr_fast(key) {
                            unsafe { f.set_r(a, val) };
                            continue;
                        }
                        let meta = table_ref.meta_ptr();
                        if meta.is_null() {
                            unsafe { f.set_r(a, LuaValue::nil()) };
                            continue;
                        }
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        match noinline::try_index_meta_str(lua_state, meta, rb, key, f.frame_idx)? {
                            noinline::IndexResult::Found(val) => {
                                unsafe { f.set_r(a, val) };
                                continue;
                            }
                            noinline::IndexResult::CallMm => continue 'startfunc,
                            noinline::IndexResult::FallThrough => {}
                        }
                    }

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    let mut pc_idx = f.pc;
                    table_ops::exec_getfield(lua_state, instr, &chunk.constants, f.base, f.frame_idx, &mut pc_idx)?;
                    f.pc = pc_idx;
                    f.refresh_base(lua_state);
                    f.refresh_sp(lua_state);
                }

                OpCode::SetTable => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let ra = unsafe { f.r(a) };
                    let rb = unsafe { f.r(b) };
                    let val = if k {
                        unsafe { f.k(c) }
                    } else {
                        unsafe { f.r(c) }
                    };

                    if let Some(table_ref) = ra.as_table_mut() {
                        if !table_ref.has_metatable() {
                            if rb.ttisinteger() {
                                if table_ref.impl_table.fast_seti(rb.ivalue(), val) {
                                    if val.is_collectable()
                                        && let Some(gc_ptr) = ra.as_gc_ptr()
                                    {
                                        lua_state.gc_barrier_back(gc_ptr);
                                    }
                                    continue;
                                }
                                let delta = table_ref.impl_table.set_int_slow(rb.ivalue(), val);
                                if delta != 0
                                    && let Some(table_ptr) = ra.as_table_ptr()
                                {
                                    lua_state.gc_track_table_resize(table_ptr, delta);
                                }
                                if val.is_collectable()
                                    && let Some(gc_ptr) = ra.as_gc_ptr()
                                {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            if rb.is_nil() {
                                return Err(cold::error_table_index_nil(lua_state));
                            }
                            if rb.ttisfloat() && rb.fltvalue().is_nan() {
                                return Err(cold::error_table_index_nan(lua_state));
                            }
                            lua_state.raw_set(&ra, rb, val);
                            continue;
                        }
                        if rb.ttisinteger()
                            && table_ref.impl_table.fast_seti_existing(rb.ivalue(), val)
                        {
                            if val.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                        if let Some(existing) = table_ref.impl_table.raw_get(&rb)
                            && !existing.is_nil()
                        {
                            lua_state.raw_set(&ra, rb, val);
                            continue;
                        }
                        let meta = table_ref.meta_ptr();
                        if !meta.is_null() {
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if noinline::try_newindex_meta(lua_state, meta, ra, rb, val, f.frame_idx)? {
                                continue 'startfunc;
                            }
                        }
                    }

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    let mut pc_idx = f.pc;
                    table_ops::exec_settable(lua_state, instr, &chunk.constants, f.base, f.frame_idx, &mut pc_idx)?;
                    f.pc = pc_idx;
                    f.refresh_base(lua_state);
                    f.refresh_sp(lua_state);
                }

                OpCode::SetI => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let ra = unsafe { f.r(a) };
                    let value = if k {
                        unsafe { f.k(c) }
                    } else {
                        unsafe { f.r(c) }
                    };

                    if let Some(table_ref) = ra.as_table_mut() {
                        if !table_ref.has_metatable() {
                            if table_ref.impl_table.fast_seti(b as i64, value) {
                                if value.is_collectable()
                                    && let Some(gc_ptr) = ra.as_gc_ptr()
                                {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            let delta = table_ref.impl_table.set_int_slow(b as i64, value);
                            if delta != 0
                                && let Some(table_ptr) = ra.as_table_ptr()
                            {
                                lua_state.gc_track_table_resize(table_ptr, delta);
                            }
                            if value.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                        if table_ref.impl_table.fast_seti_existing(b as i64, value) {
                            if value.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                        let meta = table_ref.meta_ptr();
                        if !meta.is_null() {
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if noinline::try_newindex_meta(
                                lua_state, meta, ra, LuaValue::integer(b as i64), value, f.frame_idx,
                            )? {
                                continue 'startfunc;
                            }
                        }
                    }

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    let mut pc_idx = f.pc;
                    table_ops::exec_seti(lua_state, instr, &chunk.constants, f.base, f.frame_idx, &mut pc_idx)?;
                    f.pc = pc_idx;
                    f.refresh_base(lua_state);
                    f.refresh_sp(lua_state);
                }

                OpCode::SetField => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let ra = unsafe { f.r(a) };
                    let key = unsafe { &*f.constants_ptr.add(b) };
                    let value = if k {
                        unsafe { f.k(c) }
                    } else {
                        unsafe { f.r(c) }
                    };

                    if let Some(table_ref) = ra.as_table_mut() {
                        if table_ref.impl_table.fast_setfield(key, value) {
                            if value.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                        if !table_ref.has_metatable() {
                            if table_ref.impl_table.fast_setfield_newkey(key, value) {
                                table_ref.invalidate_tm_cache();
                                if value.is_collectable()
                                    && let Some(gc_ptr) = ra.as_gc_ptr()
                                {
                                    lua_state.gc_barrier_back(gc_ptr);
                                }
                                continue;
                            }
                            let (_, delta) = table_ref.impl_table.raw_set(key, value);
                            table_ref.invalidate_tm_cache();
                            if delta != 0
                                && let Some(table_ptr) = ra.as_table_ptr()
                            {
                                lua_state.gc_track_table_resize(table_ptr, delta);
                            }
                            if value.is_collectable()
                                && let Some(gc_ptr) = ra.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                        let meta = table_ref.meta_ptr();
                        if !meta.is_null() {
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if noinline::try_newindex_meta(lua_state, meta, ra, *key, value, f.frame_idx)? {
                                continue 'startfunc;
                            }
                        }
                    }

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    let mut pc_idx = f.pc;
                    table_ops::exec_setfield(lua_state, instr, &chunk.constants, f.base, f.frame_idx, &mut pc_idx)?;
                    f.pc = pc_idx;
                    f.refresh_base(lua_state);
                    f.refresh_sp(lua_state);
                }

                OpCode::Self_ => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let key = unsafe { &*f.constants_ptr.add(c) };
                    let rb = unsafe { f.r(b) };
                    // R[A+1] := R[B] (save object for method call)
                    unsafe { f.set_r(a + 1, rb) };

                    if rb.tt == LUA_VTABLE {
                        let table_gc = unsafe { &*(rb.value.ptr as *const GcTable) };
                        let table_ref = &table_gc.data;
                        if let Some(val) = table_ref.impl_table.get_shortstr_fast(key) {
                            unsafe { f.set_r(a, val) };
                            continue;
                        }
                        let meta = table_ref.meta_ptr();
                        if meta.is_null() {
                            unsafe { f.set_r(a, LuaValue::nil()) };
                            continue;
                        }
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        match noinline::try_index_meta_str(lua_state, meta, rb, key, f.frame_idx)? {
                            noinline::IndexResult::Found(val) => {
                                unsafe { f.set_r(a, val) };
                                continue;
                            }
                            noinline::IndexResult::CallMm => continue 'startfunc,
                            noinline::IndexResult::FallThrough => {}
                        }
                    }

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    let mut pc_idx = f.pc;
                    table_ops::exec_self(lua_state, instr, &chunk.constants, f.base, f.frame_idx, &mut pc_idx)?;
                    f.pc = pc_idx;
                    f.refresh_base(lua_state);
                    f.refresh_sp(lua_state);
                }

                OpCode::GetTabUp => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let upvalue_ptrs = unsafe {
                        let ci = lua_state.get_call_info(f.frame_idx);
                        ci.func.as_lua_function_unchecked().upvalues()
                    };
                    let upval = &upvalue_ptrs[b].as_ref().data;
                    let key = unsafe { &*f.constants_ptr.add(c) };
                    let table_value = upval.get_value_ref();

                    // Fast path: direct hash lookup for short string keys
                    let result = if table_value.tt == LUA_VTABLE {
                        let table = unsafe { &*(table_value.value.ptr as *const GcTable) };
                        let native = &table.data.impl_table;
                        if native.has_hash() {
                            native.get_shortstr_unchecked(key)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(val) = result {
                        unsafe { f.set_r(a, val) };
                    } else {
                        let table_value = *upval.get_value_ref();
                        if table_value.tt == LUA_VTABLE {
                            let table_gc = unsafe { &*(table_value.value.ptr as *const GcTable) };
                            let meta = table_gc.data.meta_ptr();
                            if !meta.is_null() {
                                lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                                match noinline::try_index_meta_str(lua_state, meta, table_value, key, f.frame_idx)? {
                                    noinline::IndexResult::Found(val) => {
                                        unsafe { f.set_r(a, val) };
                                        continue;
                                    }
                                    noinline::IndexResult::CallMm => continue 'startfunc,
                                    noinline::IndexResult::FallThrough => {}
                                }
                            }
                        }
                        // Slow path: metamethod lookup
                        let table_value = *lua_state
                            .get_call_info(f.frame_idx)
                            .func
                            .as_lua_function()
                            .unwrap()
                            .upvalues()[b]
                            .as_ref()
                            .data
                            .get_value_ref();
                        let write_pos = f.base + a;
                        let call_info = lua_state.get_call_info_mut(f.frame_idx);
                        if write_pos + 1 > call_info.top {
                            call_info.top = write_pos + 1;
                            lua_state.set_top(write_pos + 1)?;
                        }
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        match helper::lookup_from_metatable(lua_state, &table_value, key) {
                            Ok(result) => {
                                f.refresh_base(lua_state);
                                f.refresh_sp(lua_state);
                                unsafe {
                                    f.set_r(a, result.unwrap_or(LuaValue::nil()));
                                };
                            }
                            Err(LuaError::Yield) => {
                                let ci = lua_state.get_call_info_mut(f.frame_idx);
                                ci.pending_finish_get = a as i32;
                                ci.call_status |= CIST_PENDING_FINISH;
                                return Err(LuaError::Yield);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }

                OpCode::SetTabUp => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let key = unsafe { f.k(b) };
                    let value = if k {
                        unsafe { f.k(c) }
                    } else {
                        unsafe { f.r(c) }
                    };

                    let (table_val_copy, table_raw_ptr) = unsafe {
                        let ci = lua_state.get_call_info(f.frame_idx);
                        let upvalue_ptrs = ci.func.as_lua_function_unchecked().upvalues();
                        let upval = &upvalue_ptrs[a].as_ref().data;
                        let tv = upval.get_value_ref();
                        (*tv, tv as *const LuaValue)
                    };
                    if table_val_copy.tt == LUA_VTABLE {
                        let table = unsafe { &mut *(table_val_copy.value.ptr as *mut GcTable) };
                        let native = &mut table.data.impl_table;
                        if native.has_hash() && native.set_shortstr_unchecked(&key, value) {
                            if value.is_collectable()
                                && let Some(gc_ptr) = table_val_copy.as_gc_ptr()
                            {
                                lua_state.gc_barrier_back(gc_ptr);
                            }
                            continue;
                        }
                        let meta = table.data.meta_ptr();
                        if !meta.is_null() {
                            lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                            if noinline::try_newindex_meta(lua_state, meta, table_val_copy, key, value, f.frame_idx)? {
                                continue 'startfunc;
                            }
                        }
                    }

                    let table_value = unsafe { *table_raw_ptr };
                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    match helper::finishset(lua_state, &table_value, &key, value) {
                        Ok(_) => {
                            f.refresh_base(lua_state);
                            f.refresh_sp(lua_state);
                        }
                        Err(LuaError::Yield) => {
                            let ci = lua_state.get_call_info_mut(f.frame_idx);
                            ci.pending_finish_get = -2;
                            ci.call_status |= CIST_PENDING_FINISH;
                            return Err(LuaError::Yield);
                        }
                        Err(e) => return Err(e),
                    }
                }

                // ============================================================
                // LENGTH AND CONCATENATION
                // ============================================================
                OpCode::Len => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let rb = unsafe { f.r(b) };
                    if rb.ttistable() {
                        let table_gc = unsafe { &mut *(rb.value.ptr as *mut GcTable) };
                        let table = &mut table_gc.data;
                        let meta = table.meta_ptr();
                        if meta.is_null() {
                            setivalue(
                                unsafe { &mut *f.rp(a) },
                                table.len() as i64,
                            );
                            continue;
                        }
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        match noinline::try_len_meta(lua_state, meta, rb, f.frame_idx)? {
                            noinline::LenResult::RawLen => {
                                let tbl = unsafe { &*(rb.value.ptr as *const GcTable) };
                                setivalue(
                                    unsafe { &mut *f.rp(a) },
                                    tbl.data.len() as i64,
                                );
                                continue;
                            }
                            noinline::LenResult::CallMm => continue 'startfunc,
                            noinline::LenResult::FallThrough => {}
                        }
                    } else if let Some(s) = rb.as_str() {
                        setivalue(
                            unsafe { &mut *f.rp(a) },
                            s.len() as i64,
                        );
                        continue;
                    }
                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    cold::handle_len(lua_state, instr, &mut f.base, f.frame_idx, f.pc)?;
                    f.refresh_sp(lua_state);
                }

                OpCode::Concat => {
                    let a = instr.get_a() as usize;
                    let n = instr.get_b() as usize;
                    let concat_top = f.base + a + n;
                    if concat_top > lua_state.get_top() {
                        lua_state.set_top_raw(concat_top);
                    }
                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    super::concat::handle_concat(lua_state, instr, &mut f.base, f.frame_idx, f.pc)?;
                    f.refresh_sp(lua_state);
                }

                // ============================================================
                // RETURN (full)
                // ============================================================
                OpCode::Return => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);

                    if lua_state.hook_mask & crate::lua_vm::LUA_MASKRET != 0
                        && lua_state.allow_hook
                    {
                        let nres = if b != 0 {
                            (b - 1) as i32
                        } else {
                            (lua_state.get_top() - (f.base + a)) as i32
                        };
                        hook_on_return(lua_state, f.frame_idx, f.pc as u32, nres)?;
                    }

                    return_handler::handle_return(lua_state, f.base, f.frame_idx, a, b, c, k)?;
                    continue 'startfunc;
                }

                // ============================================================
                // TAILCALL
                // ============================================================
                OpCode::TailCall => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);

                    match call::handle_tailcall(lua_state, f.base, a, b) {
                        Ok(FrameAction::Continue) => {
                            f.refresh_base(lua_state);
                            f.refresh_sp(lua_state);
                            lua_state.oldpc = (f.pc - 1) as u32;
                            trap = lua_state.hook_mask != 0;
                        }
                        Ok(FrameAction::TailCall) => {
                            // Tail call replaced frame — inline context restore
                            let ci = lua_state.get_call_info(f.frame_idx);
                            f.base = ci.base;
                            chunk = unsafe { &*ci.chunk_ptr };
                            f.code_ptr = chunk.code.as_ptr();
                            f.code_len = chunk.code.len();
                            f.constants_ptr = chunk.constants.as_ptr();
                            f.pc = 0;
                            f.sp = lua_state.stack_mut().as_mut_ptr();

                            lua_state.oldpc = if chunk.is_vararg { 0 } else { u32::MAX };

                            if trap {
                                let hook_mask = lua_state.hook_mask;
                                if hook_mask & crate::lua_vm::LUA_MASKCALL != 0
                                    && lua_state.allow_hook
                                {
                                    hook_on_call(
                                        lua_state,
                                        hook_mask,
                                        lua_state.get_call_info(f.frame_idx).call_status,
                                        chunk,
                                    )?;
                                    f.refresh_sp(lua_state);
                                }
                                if hook_mask & crate::lua_vm::LUA_MASKCOUNT != 0 {
                                    lua_state.hook_count = lua_state.base_hook_count;
                                }
                            }
                        }
                        Ok(FrameAction::Call) => {
                            continue 'startfunc;
                        }
                        Err(e) => return Err(e),
                    }
                }

                // ============================================================
                // FOR LOOPS
                // ============================================================
                OpCode::ForPrep => {
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;
                    let mut pc_idx = f.pc;
                    cold::handle_forprep_int(lua_state, f.base + a, bx, f.frame_idx, &mut pc_idx)?;
                    f.pc = pc_idx;
                }

                OpCode::TForPrep => {
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let stack = lua_state.stack_mut();
                    let ra = f.base + a;
                    stack.swap(ra + 3, ra + 2);
                    lua_state.mark_tbc(ra + 2)?;
                    f.pc += bx;
                }

                OpCode::TForCall => {
                    let a = instr.get_a() as usize;
                    let c = instr.get_c() as usize;

                    let ra_base = f.base + a;

                    let (iterator, c_func_opt) = unsafe {
                        let stack = lua_state.stack_mut();
                        let iterator = *stack.get_unchecked(ra_base);
                        let state = *stack.get_unchecked(ra_base + 1);
                        let control = *stack.get_unchecked(ra_base + 3);

                        *stack.get_unchecked_mut(ra_base + 3) = iterator;
                        *stack.get_unchecked_mut(ra_base + 4) = state;
                        *stack.get_unchecked_mut(ra_base + 5) = control;

                        let c_func_opt = if let Some(cf) = iterator.as_cfunction() {
                            Some(cf)
                        } else {
                            iterator.as_cclosure().map(|cc| cc.func())
                        };

                        (iterator, c_func_opt)
                    };

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);

                    if let Some(c_func) = c_func_opt {
                        call::call_c_function_fast(
                            lua_state,
                            &iterator,
                            c_func,
                            ra_base + 3,
                            2,
                            c as i32 + 1,
                        )?;
                        f.refresh_base(lua_state);
                        f.refresh_sp(lua_state);
                        lua_state.oldpc = (f.pc - 1) as u32;
                    } else {
                        match call::handle_call(lua_state, f.base, a + 3, 3, c + 1, 0) {
                            Ok(FrameAction::Continue) => {
                                f.refresh_base(lua_state);
                                f.refresh_sp(lua_state);
                                lua_state.oldpc = (f.pc - 1) as u32;
                                trap = lua_state.hook_mask != 0;
                            }
                            Ok(FrameAction::Call) | Ok(FrameAction::TailCall) => {
                                continue 'startfunc;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }

                OpCode::TForLoop => {
                    let a = instr.get_a() as usize;
                    let bx = instr.get_bx() as usize;

                    let ra = f.base + a;
                    if !unsafe { *lua_state.stack().get_unchecked(ra + 3) }.is_nil() {
                        f.pc -= bx;
                    }
                }

                // ============================================================
                // METAMETHOD BINARY FALLBACKS
                // ============================================================
                OpCode::MmBin => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;

                    let base_mm = lua_state.get_frame_base(f.frame_idx);
                    let v1 = unsafe { *lua_state.stack().get_unchecked(base_mm + a) };
                    if v1.ttistable() {
                        let v2 = unsafe { *lua_state.stack().get_unchecked(base_mm + b) };
                        let result_reg = unsafe { *f.code_ptr.add(f.pc - 2) }.get_a() as usize;
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        match noinline::try_mmbin_table_fast(
                            lua_state, v1, v1, v2, c as u8, result_reg, f.frame_idx,
                        )? {
                            noinline::MmBinResult::CallMm => continue 'startfunc,
                            noinline::MmBinResult::Handled => {
                                f.refresh_base(lua_state);
                                continue;
                            }
                            noinline::MmBinResult::FallThrough => {}
                        }
                    }

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    metamethod::handle_mmbin(lua_state, a, b, c, f.pc, &chunk.code, f.frame_idx)?;
                    f.refresh_base(lua_state);
                    f.refresh_sp(lua_state);
                }

                OpCode::MmBinI => {
                    let a = instr.get_a() as usize;
                    let sb = instr.get_sb();
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let base_mm = lua_state.get_frame_base(f.frame_idx);
                    let v1 = unsafe { *lua_state.stack().get_unchecked(base_mm + a) };
                    if v1.ttistable() {
                        let imm = LuaValue::integer(sb as i64);
                        let (p1, p2) = if k { (imm, v1) } else { (v1, imm) };
                        let result_reg = unsafe { *f.code_ptr.add(f.pc - 2) }.get_a() as usize;
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        match noinline::try_mmbin_table_fast(
                            lua_state, v1, p1, p2, c as u8, result_reg, f.frame_idx,
                        )? {
                            noinline::MmBinResult::CallMm => continue 'startfunc,
                            noinline::MmBinResult::Handled => {
                                f.refresh_base(lua_state);
                                continue;
                            }
                            noinline::MmBinResult::FallThrough => {}
                        }
                    }

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    metamethod::handle_mmbini(lua_state, a, sb, c, k, f.pc, &chunk.code, f.frame_idx)?;
                    f.refresh_base(lua_state);
                    f.refresh_sp(lua_state);
                }

                OpCode::MmBinK => {
                    let a = instr.get_a() as usize;
                    let b = instr.get_b() as usize;
                    let c = instr.get_c() as usize;
                    let k = instr.get_k();

                    let base_mm = lua_state.get_frame_base(f.frame_idx);
                    let v1 = unsafe { *lua_state.stack().get_unchecked(base_mm + a) };
                    if v1.ttistable() {
                        let kb = unsafe { f.k(b) };
                        let (p1, p2) = if k { (kb, v1) } else { (v1, kb) };
                        let result_reg = unsafe { *f.code_ptr.add(f.pc - 2) }.get_a() as usize;
                        lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                        match noinline::try_mmbin_table_fast(
                            lua_state, v1, p1, p2, c as u8, result_reg, f.frame_idx,
                        )? {
                            noinline::MmBinResult::CallMm => continue 'startfunc,
                            noinline::MmBinResult::Handled => {
                                f.refresh_base(lua_state);
                                continue;
                            }
                            noinline::MmBinResult::FallThrough => {}
                        }
                    }

                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    metamethod::handle_mmbink(lua_state, a, b, c, k, f.pc, &chunk.code, &chunk.constants, f.frame_idx)?;
                    f.refresh_base(lua_state);
                    f.refresh_sp(lua_state);
                }

                // ============================================================
                // CLOSURE / VARARG / SETLIST
                // ============================================================
                OpCode::Closure => {
                    handle_closure(lua_state, instr, f.base, f.frame_idx, chunk, f.pc)?;
                    f.refresh_sp(lua_state);
                }

                OpCode::Vararg => {
                    closure_vararg_ops::exec_vararg(lua_state, instr, f.base, f.frame_idx, chunk)?;
                    f.refresh_sp(lua_state);
                }

                OpCode::GetVarg => {
                    cold::handle_getvarg(lua_state, instr, f.base, f.frame_idx)?;
                    f.refresh_sp(lua_state);
                }

                OpCode::VarargPrep => {
                    closure_vararg_ops::exec_varargprep(lua_state, f.frame_idx, chunk, &mut f.base)?;
                    f.refresh_sp(lua_state);
                    if lua_state.hook_mask != 0 {
                        lua_state.oldpc = u32::MAX;
                    }
                }

                OpCode::SetList => {
                    let mut pc_idx = f.pc;
                    closure_vararg_ops::exec_setlist(lua_state, instr, &chunk.code, f.base, &mut pc_idx)?;
                    f.pc = pc_idx;
                }

                // ============================================================
                // CLOSE / TBC / LOADKX / ERRNIL / EXTRAARG
                // ============================================================
                OpCode::Close => {
                    cold::handle_close(lua_state, instr, f.base, f.frame_idx, f.pc)?;
                }

                OpCode::Tbc => {
                    let a = instr.get_a() as usize;
                    lua_state.mark_tbc(f.base + a)?;
                }

                OpCode::LoadKX => {
                    let mut pc_idx = f.pc;
                    cold::handle_loadkx(lua_state, instr, f.base, f.frame_idx, &chunk.code, &chunk.constants, &mut pc_idx)?;
                    f.pc = pc_idx;
                }

                OpCode::ErrNNil => {
                    cold::handle_errnil(lua_state, instr, f.base, &chunk.constants, f.frame_idx, f.pc)?;
                }

                OpCode::ExtraArg => {
                    lua_state.set_frame_pc(f.frame_idx, f.pc as u32);
                    return Err(cold::error_unexpected_extraarg(lua_state));
                }
            }
        } // mainloop
    } // startfunc
}