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
    lua_vm::{LuaError, StkId, call_info::CallInfo, execute::helper::pk_val},
};

// ── Internal helper ─────────────────────────────────────────────

/// Float fallback for arithmetic ops.
/// Tries to convert both operands to f64 and apply the float operation.
/// Mirrors the `op_arithf_aux!` macro.
#[inline(always)]
fn arithf_aux(ra: StkId, pc: &mut usize, v1: StkId, v2: StkId, fop: impl Fn(f64, f64) -> f64) {
    let mut n1 = 0.0;
    let mut n2 = 0.0;
    if ptonumberns(v1, &mut n1) && ptonumberns(v2, &mut n2) {
        *pc += 1;
        ra.set_float(fop(n1, n2));
    }
}

// ── op_arithI: R[A] := iop(R[B], sC)  ──────────────────────────

#[inline(always)]
pub(crate) fn op_arith_i(
    base_stk: StkId,
    pc: &mut usize,
    instr: Instruction,
    iop: impl Fn(i64, i32) -> i64,
    fop: impl Fn(f64, f64) -> f64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let sc = instr.get_sc();
    let v1 = base_stk.offset(b);
    if v1.is_integer() {
        *pc += 1;
        base_stk.offset(a).set_integer(iop(v1.ivalue(), sc));
    } else if v1.is_float() {
        *pc += 1;
        base_stk.offset(a).set_float(fop(v1.fltvalue(), sc as f64));
    }
}

// ── op_arith: R[A] := iop(R[B], R[C])  ──────────────────────────

#[inline(always)]
pub(crate) fn op_arith(
    base_stk: StkId,
    pc: &mut usize,
    instr: Instruction,
    iop: impl Fn(i64, i64) -> i64,
    fop: impl Fn(f64, f64) -> f64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let v1 = base_stk.offset(b);
    let v2 = base_stk.offset(c);
    if v1.is_integer() && v2.is_integer() {
        *pc += 1;
        base_stk
            .offset(a)
            .set_integer(iop(v1.ivalue(), v2.ivalue()));
    } else {
        arithf_aux(base_stk.offset(a), pc, v1, v2, fop);
    }
}

// ── op_arithf: R[A] := fop(R[B], R[C])  ─────────────────────────

#[inline(always)]
pub(crate) fn op_arithf(
    base_stk: StkId,
    pc: &mut usize,
    instr: Instruction,
    fop: impl Fn(f64, f64) -> f64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    arithf_aux(
        base_stk.offset(a),
        pc,
        base_stk.offset(b),
        base_stk.offset(c),
        fop,
    );
}

// ── op_arithK: R[A] := iop(R[B], K[C])  ─────────────────────────

#[inline(always)]
pub(crate) fn op_arith_k(
    base_stk: StkId,
    pc: &mut usize,
    instr: Instruction,
    constants: &[LuaValue],
    iop: impl Fn(i64, i64) -> i64,
    fop: impl Fn(f64, f64) -> f64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let v1 = base_stk.offset(b);
    let v2 = pk_val(constants, c);
    if v1.is_integer() && v2.is_integer() {
        *pc += 1;
        base_stk
            .offset(a)
            .set_integer(iop(v1.ivalue(), v2.ivalue()));
    } else {
        arithf_aux(base_stk.offset(a), pc, v1, v2, fop);
    }
}

// ── op_arithfK: R[A] := fop(R[B], K[C])  ────────────────────────

#[inline(always)]
pub(crate) fn op_arithf_k(
    base_stk: StkId,
    pc: &mut usize,
    instr: Instruction,
    constants: &[LuaValue],
    fop: impl Fn(f64, f64) -> f64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    arithf_aux(
        base_stk.offset(a),
        pc,
        base_stk.offset(b),
        pk_val(constants, c),
        fop,
    );
}

// ── op_arith_check_zero: R[A] := iop(R[B], R[C]) with zero check ─

#[inline(always)]
pub(crate) fn op_arith_check_zero(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: StkId,
    pc: &mut usize,
    instr: Instruction,
    iop: impl Fn(i64, i64) -> i64,
    fop: impl Fn(f64, f64) -> f64,
    err_fn: fn(&mut LuaState) -> LuaError,
) -> LuaResult<()> {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let v1 = base_stk.offset(b);
    let v2 = base_stk.offset(c);
    if v1.is_integer() && v2.is_integer() {
        let i1 = v1.ivalue();
        let i2 = v2.ivalue();
        if i2 != 0 {
            *pc += 1;
            base_stk.offset(a).set_integer(iop(i1, i2));
        } else {
            ci.save_pc(*pc);
            return Err(err_fn(lua_state));
        }
    } else {
        arithf_aux(base_stk.offset(a), pc, v1, v2, fop);
    }
    Ok(())
}

// ── op_arithK_check_zero: R[A] := iop(R[B], K[C]) with zero check ─

#[inline(always)]
pub(crate) fn op_arithk_check_zero(
    lua_state: &mut LuaState,
    ci: &mut CallInfo,
    base_stk: StkId,
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
    let v1 = base_stk.offset(b);
    let v2 = pk_val(constants, c);
    if v1.is_integer() && v2.is_integer() {
        let i1 = v1.ivalue();
        let i2 = v2.ivalue();
        if i2 != 0 {
            *pc += 1;
            base_stk.offset(a).set_integer(iop(i1, i2));
        } else {
            ci.save_pc(*pc);
            return Err(err_fn(lua_state));
        }
    } else {
        arithf_aux(base_stk.offset(a), pc, v1, v2, fop);
    }
    Ok(())
}

// ── op_bitwise: R[A] := op(R[B], R[C])  ─────────────────────────

#[inline(always)]
pub(crate) fn op_bitwise(
    base_stk: StkId,
    pc: &mut usize,
    instr: Instruction,
    op: impl Fn(i64, i64) -> i64,
) {
    let a = instr.get_a() as usize;
    let b = instr.get_b() as usize;
    let c = instr.get_c() as usize;
    let mut i1 = 0i64;
    let mut i2 = 0i64;
    if ptointegerns(base_stk.offset(b), &mut i1) && ptointegerns(base_stk.offset(c), &mut i2) {
        *pc += 1;
        base_stk.offset(a).set_integer(op(i1, i2));
    }
}

// ── op_bitwiseK: R[A] := op(R[B], K[C])  ────────────────────────

#[inline(always)]
pub(crate) fn op_bitwise_k(
    base_stk: StkId,
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
    if ptointegerns(base_stk.offset(b), &mut i1) {
        *pc += 1;
        base_stk.offset(a).set_integer(op(i1, i2));
    }
}

#[inline(always)]
pub fn ptonumberns(v: StkId, out: &mut f64) -> bool {
    if v.is_float() {
        *out = v.fltvalue();
        true
    } else if v.is_integer() {
        *out = v.ivalue() as f64;
        true
    } else {
        false
    }
}

#[inline(always)]
pub fn ptointegerns(v: StkId, out: &mut i64) -> bool {
    if v.is_integer() {
        *out = v.ivalue();
        true
    } else if v.is_float() {
        // Try converting integral-valued floats (e.g. 5.0 -> 5)
        // Range check matches C Lua's lua_numbertointeger:
        //   f >= (i64::MIN as f64) && f < -(i64::MIN as f64)
        // Note: i64::MAX as f64 rounds UP to 2^63, so we must use strict <
        // with -(i64::MIN as f64) = 2^63 (exactly representable).
        let f = v.fltvalue();
        if f >= (i64::MIN as f64) && f < -(i64::MIN as f64) && f == (f as i64 as f64) {
            *out = f as i64;
            true
        } else {
            false
        }
    } else {
        false
    }
}

/// luaV_shiftl - Shift integer x left by y positions.
/// If y is negative, shifts right (LOGICAL/unsigned shift).
/// Matches Lua 5.5's luaV_shiftl from lvm.c.
#[inline(always)]
pub fn lua_shiftl(x: i64, y: i64) -> i64 {
    if y < 0 {
        // Right shift (logical/unsigned)
        if y <= -64 {
            0
        } else {
            ((x as u64) >> ((-y) as u32)) as i64
        }
    } else {
        // Left shift
        if y >= 64 {
            0
        } else {
            ((x as u64) << (y as u32)) as i64
        }
    }
}

/// luaV_shiftr - Shift integer x right by y positions.
/// luaV_shiftr(x, y) = luaV_shiftl(x, -y)
#[inline(always)]
pub fn lua_shiftr(x: i64, y: i64) -> i64 {
    lua_shiftl(x, y.wrapping_neg())
}

/// Lua floor division for integers: a // b
/// Equivalent to luaV_idiv in Lua 5.5
#[inline(always)]
pub fn lua_idiv(a: i64, b: i64) -> i64 {
    // Handle overflow case: MIN_INT / -1 would overflow, wrapping gives MIN_INT (floor division same result)
    if b == -1 {
        return a.wrapping_neg();
    }
    let q = a / b;
    // If the signs of a and b differ and there is a remainder,
    // subtract 1 to achieve floor division (toward -infinity)
    if (a ^ b) < 0 && a % b != 0 {
        q.wrapping_sub(1)
    } else {
        q
    }
}

/// Lua modulo for integers: a % b
/// Equivalent to luaV_mod in Lua 5.5: m = a % b; if m != 0 && (m ^ b) < 0 then m += b
#[inline(always)]
pub fn lua_imod(a: i64, b: i64) -> i64 {
    // Handle overflow case: MIN_INT % -1 = 0
    if b == -1 {
        return 0;
    }
    let m = a % b;
    if m != 0 && (m ^ b) < 0 {
        m.wrapping_add(b)
    } else {
        m
    }
}

/// Float modulo matching C Lua's `luai_nummod`.
/// Uses hardware fmod (Rust's `%` operator on f64) then adjusts sign.
#[inline(always)]
pub fn lua_fmod(a: f64, b: f64) -> f64 {
    let mut m = a % b; // C fmod
    if m != 0.0 && ((m > 0.0) != (b > 0.0)) {
        m += b;
    }
    m
}

/// luai_numpow - Power operation matching Lua 5.5's luai_numpow macro:
///   #define luai_numpow(L,a,b)  ((b)==2 ? (a)*(a) : pow(a,b))
#[inline(always)]
pub fn luai_numpow(a: f64, b: f64) -> f64 {
    if b == 2.0 {
        a * a
    } else if a.fract() == 0.0 && b.fract() == 0.0 && b >= 0.0 && b <= u64::MAX as f64 {
        let mut base = a;
        let mut exp = b as u64;
        let mut result = 1.0;
        while exp != 0 {
            if exp & 1 == 1 {
                result *= base;
            }
            exp >>= 1;
            if exp != 0 {
                base *= base;
            }
        }
        result
    } else {
        a.powf(b)
    }
}
