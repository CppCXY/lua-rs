/*----------------------------------------------------------------------
  Lua 5.5 VM Arithmetic & Bitwise Operations

  Extracted from execute_loop.rs macros into generic #[inline(always)]
  functions. Each function is monomorphized per call site, producing
  identical machine code to the original macro approach.

  Design:
  1. Generic over operation closures (I: Fn(...), F: Fn(...)) — ensures
     the compiler can inline both the function AND the closure body.
  2. Functions return () for simple ops, LuaResult<()> for ops that can
     error (division by zero).
  3. The arithf_aux internal helper handles the common float fallback
     path used by all arithmetic operations.
----------------------------------------------------------------------*/

use crate::{
    Instruction, LuaResult, LuaState, LuaValue,
    lua_vm::{
        LuaError, StkId,
        call_info::CallInfo,
        execute::helper::{pk_val, ptonumberns, tointegerns},
    },
};

// ── Internal helper ─────────────────────────────────────────────

/// Float fallback for arithmetic ops.
/// Tries to convert both operands to f64 and apply the float operation.
/// Mirrors the `op_arithf_aux!` macro.
#[inline(always)]
fn arithf_aux(ra: StkId, pc: &mut usize, v1: StkId, v2: StkId, fop: impl Fn(f64, f64) -> f64) {
    let mut n1 = 0.0;
    let mut n2 = 0.0;
    if unsafe { ptonumberns(v1.as_const_ptr(), &mut n1) && ptonumberns(v2.as_const_ptr(), &mut n2) }
    {
        *pc += 1;
        ra.set_float(fop(n1, n2));
    }
}

// ── op_arithI: R[A] := iop(R[B], sC)  ──────────────────────────

#[inline(always)]
pub(crate) fn op_arith_i(
    base_stk: &mut StkId,
    pc: &mut usize,
    instr: Instruction,
    iop: impl Fn(i64, i32) -> i64,
    fop: impl Fn(f64, f64) -> f64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let sc = instr.get_sc();
    let v1 = (*base_stk).offset(b);
    if v1.is_integer() {
        *pc += 1;
        (*base_stk).offset(a).set_integer(iop(v1.ivalue(), sc));
    } else if v1.is_float() {
        *pc += 1;
        (*base_stk)
            .offset(a)
            .set_float(fop(v1.fltvalue(), sc as f64));
    }
}

// ── op_arith: R[A] := iop(R[B], R[C])  ──────────────────────────

#[inline(always)]
pub(crate) fn op_arith(
    base_stk: &mut StkId,
    pc: &mut usize,
    instr: Instruction,
    iop: impl Fn(i64, i64) -> i64,
    fop: impl Fn(f64, f64) -> f64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let v1 = (*base_stk).offset(b);
    let v2 = (*base_stk).offset(c);
    if v1.is_integer() && v2.is_integer() {
        *pc += 1;
        (*base_stk)
            .offset(a)
            .set_integer(iop(v1.ivalue(), v2.ivalue()));
    } else {
        arithf_aux((*base_stk).offset(a), pc, v1, v2, fop);
    }
}

// ── op_arithf: R[A] := fop(R[B], R[C])  ─────────────────────────

#[inline(always)]
pub(crate) fn op_arithf(
    base_stk: &mut StkId,
    pc: &mut usize,
    instr: Instruction,
    fop: impl Fn(f64, f64) -> f64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    arithf_aux(
        (*base_stk).offset(a),
        pc,
        (*base_stk).offset(b),
        (*base_stk).offset(c),
        fop,
    );
}

// ── op_arithK: R[A] := iop(R[B], K[C])  ─────────────────────────

#[inline(always)]
pub(crate) fn op_arith_k(
    base_stk: &mut StkId,
    pc: &mut usize,
    instr: Instruction,
    constants: &[LuaValue],
    iop: impl Fn(i64, i64) -> i64,
    fop: impl Fn(f64, f64) -> f64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let v1 = (*base_stk).offset(b);
    let v2 = pk_val(constants, c);
    if v1.is_integer() && v2.is_integer() {
        *pc += 1;
        (*base_stk)
            .offset(a)
            .set_integer(iop(v1.ivalue(), v2.ivalue()));
    } else {
        arithf_aux((*base_stk).offset(a), pc, v1, v2, fop);
    }
}

// ── op_arithfK: R[A] := fop(R[B], K[C])  ────────────────────────

#[inline(always)]
pub(crate) fn op_arithf_k(
    base_stk: &mut StkId,
    pc: &mut usize,
    instr: Instruction,
    constants: &[LuaValue],
    fop: impl Fn(f64, f64) -> f64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    arithf_aux(
        (*base_stk).offset(a),
        pc,
        (*base_stk).offset(b),
        pk_val(constants, c),
        fop,
    );
}

// ── op_arith_check_zero: R[A] := iop(R[B], R[C]) with zero check ─

#[inline(always)]
pub(crate) fn op_arith_check_zero(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    pc: &mut usize,
    instr: Instruction,
    iop: impl Fn(i64, i64) -> i64,
    fop: impl Fn(f64, f64) -> f64,
    err_fn: fn(&mut LuaState) -> LuaError,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let v1 = (*base_stk).offset(b);
    let v2 = (*base_stk).offset(c);
    if v1.is_integer() && v2.is_integer() {
        let i1 = v1.ivalue();
        let i2 = v2.ivalue();
        if i2 != 0 {
            *pc += 1;
            (*base_stk).offset(a).set_integer(iop(i1, i2));
        } else {
            ci.save_pc(*pc);
            return Err(err_fn(lua_state));
        }
    } else {
        arithf_aux((*base_stk).offset(a), pc, v1, v2, fop);
    }
    Ok(())
}

// ── op_arithK_check_zero: R[A] := iop(R[B], K[C]) with zero check ─

#[inline(always)]
pub(crate) fn op_arithk_check_zero(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: &mut StkId,
    pc: &mut usize,
    instr: Instruction,
    constants: &[LuaValue],
    iop: impl Fn(i64, i64) -> i64,
    fop: impl Fn(f64, f64) -> f64,
    err_fn: fn(&mut LuaState) -> LuaError,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let v1 = (*base_stk).offset(b);
    let v2 = pk_val(constants, c);
    if v1.is_integer() && v2.is_integer() {
        let i1 = v1.ivalue();
        let i2 = v2.ivalue();
        if i2 != 0 {
            *pc += 1;
            (*base_stk).offset(a).set_integer(iop(i1, i2));
        } else {
            ci.save_pc(*pc);
            return Err(err_fn(lua_state));
        }
    } else {
        arithf_aux((*base_stk).offset(a), pc, v1, v2, fop);
    }
    Ok(())
}

// ── op_bitwise: R[A] := op(R[B], R[C])  ─────────────────────────

#[inline(always)]
pub(crate) fn op_bitwise(
    base_stk: &mut StkId,
    pc: &mut usize,
    instr: Instruction,
    op: impl Fn(i64, i64) -> i64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let mut i1 = 0i64;
    let mut i2 = 0i64;
    if tointegerns((*base_stk).offset(b).get_ref(), &mut i1)
        && tointegerns((*base_stk).offset(c).get_ref(), &mut i2)
    {
        *pc += 1;
        (*base_stk).offset(a).set_integer(op(i1, i2));
    }
}

// ── op_bitwiseK: R[A] := op(R[B], K[C])  ────────────────────────

#[inline(always)]
pub(crate) fn op_bitwise_k(
    base_stk: &mut StkId,
    pc: &mut usize,
    instr: Instruction,
    constants: &[LuaValue],
    op: impl Fn(i64, i64) -> i64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let mut i1 = 0i64;
    let i2 = pk_val(constants, c).ivalue();
    if tointegerns((*base_stk).offset(b).get_ref(), &mut i1) {
        *pc += 1;
        (*base_stk).offset(a).set_integer(op(i1, i2));
    }
}
