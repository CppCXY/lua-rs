/// Cranelift IR generation for integer numeric for-loops.
///
/// # Compiled function contract
///
/// ```text
/// unsafe extern "C" fn(stack_base: *mut u8) -> i32
/// ```
///
/// - `stack_base` = pointer to `stack[base]`, i.e. `stack.as_mut_ptr() + base * 16`.
/// - Return 0  → JIT ran the entire loop successfully.
/// - Return -1 → type mismatch at entry, fall back to the interpreter (deopt).
///
/// All accesses use the layout of `LuaValue`:
/// ```text
/// offset  0: value (i64 — integer payload, or pointer bits for other types)
/// offset  8: tt    (u8  — type tag)
/// offset  9-15: padding
/// total: 16 bytes
/// ```
///
/// `LUA_VNUMINT` = 0x03  (makevariant!(LUA_TNUMBER=3, variant=0) = 3 | (0<<4) = 3)

use cranelift_codegen::{
    Context,
    ir::{
        AbiParam, InstBuilder, MemFlags,
        condcodes::IntCC,
        types::{I8, I32, I64},
    },
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{Linkage, Module};

use super::analyzer::{BodyInstr, LoopAnalysis};
use super::{runtime, JitLoopFn};

/// `makevariant!(LUA_TNUMBER=3, 0)` = 3 = 0x03
const LUA_VNUMINT: i64 = 3;

/// Byte size of `LuaValue` in memory.
const LV: i32 = 16;

/// Byte offset of the value payload within a `LuaValue`.
const VALUE_OFF: i32 = 0;

/// Byte offset of the type-tag byte within a `LuaValue`.
const TAG_OFF: i32 = 8;

/// Byte offset from `stack_base` to register `r`'s value field.
#[inline(always)]
fn val_off(r: u8) -> i32 {
    r as i32 * LV + VALUE_OFF
}

/// Byte offset from `stack_base` to register `r`'s type-tag field.
#[inline(always)]
fn tag_off(r: u8) -> i32 {
    r as i32 * LV + TAG_OFF
}

// ── Public entry point ───────────────────────────────────────────────────────

/// JIT-compile the analyzed loop.
/// Returns `Some(fn_ptr)` on success or `None` if Cranelift rejected the function.
pub fn compile(analysis: &LoopAnalysis) -> Option<JitLoopFn> {
    runtime::with_module(|module| {
        // Each compiled loop gets a unique function name.
        static CTR: std::sync::atomic::AtomicU32 =
            std::sync::atomic::AtomicU32::new(0);
        let id   = CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = format!("__lua_jit_loop_{}", id);

        // fn(*mut u8) -> i32
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(I64));   // stack_base (pointer as i64)
        sig.returns.push(AbiParam::new(I32));

        let func_id = module
            .declare_function(&name, Linkage::Local, &sig)
            .ok()?;

        let mut ctx      = module.make_context();
        ctx.func.signature = sig;

        let mut fb_ctx = FunctionBuilderContext::new();

        let ok = emit_ir(&mut ctx, &mut fb_ctx, analysis);
        if !ok {
            module.clear_context(&mut ctx);
            return None;
        }

        module.define_function(func_id, &mut ctx).ok()?;
        module.clear_context(&mut ctx);
        module.finalize_definitions().ok()?;

        let raw = module.get_finalized_function(func_id);
        // SAFETY: we just compiled a function matching JitLoopFn's ABI.
        Some(unsafe { std::mem::transmute::<*const u8, JitLoopFn>(raw) })
    })
}

// ── IR builder ───────────────────────────────────────────────────────────────

/// Emit Cranelift IR matching the Lua 5.5 ForLoop do-while semantics.
///
/// Interpreter loop structure (must be exactly replicated):
/// ```text
/// ForPrep:  count = (limit - init) / step,  stack[a]   = count
///           stack[a+1] = step,  stack[a+2] = init  (= first idx)
///
/// [BODY executed with current idx]            ← first run with no pre-check
/// ForLoop: if count > 0 { count--; idx += step; JUMP BACK to BODY }
///          else { FALL THROUGH }
/// ```
/// So `count` is the number of **additional** iterations after the first body run.
/// Total iterations = count_initial + 1.
///
/// JIT equivalent (do-while):
/// ```text
/// pre_loop: load count, idx, step    // count = N-1 where N = total iters
/// body: [execute body with current idx]
/// epilog: if count > 0 → count--, idx += step → goto body
///         else         → goto exit
/// exit: write back count=0, idx (final after last body), all written regs
///       return 0
/// ```
fn emit_ir(
    ctx:    &mut Context,
    fb_ctx: &mut FunctionBuilderContext,
    analysis: &LoopAnalysis,
) -> bool {
    let a       = analysis.a;
    let ra_cnt  = a;           // stack[a]   = iteration count (N-1)
    let ra_step = a + 1;       // stack[a+1] = step  (constant)
    let ra_idx  = a + 2;       // stack[a+2] = control variable (idx)

    let mut b = FunctionBuilder::new(&mut ctx.func, fb_ctx);

    // ── blocks ────────────────────────────────────────────────────────────
    // entry   → (type-check pass) → pre_loop  |  (fail) → deopt
    // pre_loop → body_block  (first iteration, no pre-check)
    // body_block → epilog_block
    // epilog_block: count > 0 → update → body_block  |  count == 0 → exit_block
    // exit_block: write-back, return 0
    // deopt_block: return -1
    let entry_block   = b.create_block();
    let pre_loop_block = b.create_block();
    let body_block    = b.create_block();  // NOT sealed until back-edge is emitted
    let epilog_block  = b.create_block();
    let exit_block    = b.create_block();
    let deopt_block   = b.create_block();

    // ── SSA variables ─────────────────────────────────────────────────────
    let var_count = b.declare_var(I64);
    let var_idx   = b.declare_var(I64);
    let written_vars: Vec<(u8, Variable)> = {
        let mut v = Vec::with_capacity(analysis.written.len());
        for &r in &analysis.written {
            let var = b.declare_var(I64);
            v.push((r, var));
        }
        v
    };

    // ── BLOCK: entry ──────────────────────────────────────────────────────
    b.append_block_params_for_function_params(entry_block);
    b.switch_to_block(entry_block);
    b.seal_block(entry_block);

    let stack_base = b.block_params(entry_block)[0];
    let expected   = b.ins().iconst(I8, LUA_VNUMINT);

    // Check all relevant type tags in one combined test.
    // Only check loop-carried registers plus the ForLoop control registers.
    // Body-local temporaries (written before first read each iteration) must
    // NOT be checked: they hold nil (or stale values) at loop entry.
    let mut regs_to_check: Vec<u8> = vec![ra_cnt, ra_step, ra_idx];
    for &r in &analysis.loop_carried {
        if !regs_to_check.contains(&r) {
            regs_to_check.push(r);
        }
    }
    let mut all_ok: cranelift_codegen::ir::Value = {
        let tag = b.ins().load(I8, MemFlags::trusted(), stack_base, tag_off(regs_to_check[0]));
        b.ins().icmp(IntCC::Equal, tag, expected)
    };
    for &r in &regs_to_check[1..] {
        let tag = b.ins().load(I8, MemFlags::trusted(), stack_base, tag_off(r));
        let ok  = b.ins().icmp(IntCC::Equal, tag, expected);
        all_ok  = b.ins().band(all_ok, ok);
    }
    b.ins().brif(all_ok, pre_loop_block, &[], deopt_block, &[]);

    // ── BLOCK: pre_loop ───────────────────────────────────────────────────
    // Load initial values; jump directly to body (no pre-check, matching
    // the interpreter's unconditional first entry into the loop body).
    b.seal_block(pre_loop_block);
    b.switch_to_block(pre_loop_block);

    let count_init = b.ins().load(I64, MemFlags::trusted(), stack_base, val_off(ra_cnt));
    let idx_init   = b.ins().load(I64, MemFlags::trusted(), stack_base, val_off(ra_idx));
    let step_val   = b.ins().load(I64, MemFlags::trusted(), stack_base, val_off(ra_step));

    b.def_var(var_count, count_init);
    b.def_var(var_idx,   idx_init);
    for &(r, var) in &written_vars {
        let init = if analysis.loop_carried.contains(&r) {
            // Loop-carried register: load its current integer value from memory.
            b.ins().load(I64, MemFlags::trusted(), stack_base, val_off(r))
        } else {
            // Body-local temporary: always written before first read each
            // iteration, so the initial value is irrelevant.  Use 0 to
            // satisfy Cranelift's SSA requirement (def on all paths).
            b.ins().iconst(I64, 0)
        };
        b.def_var(var, init);
    }
    b.ins().jump(body_block, &[]);
    // body_block now has two predecessors: pre_loop and (later) epilog_block.
    // Do NOT seal body_block yet.

    // ── BLOCK: body ───────────────────────────────────────────────────────
    b.switch_to_block(body_block);

    for &instr in &analysis.body {
        match instr {
            // ── Register-register arithmetic ──────────────────────────────
            BodyInstr::AddRR { dest, lhs, rhs } => {
                let vl  = read_reg(&mut b, lhs, ra_idx, var_idx, &written_vars, stack_base);
                let vr  = read_reg(&mut b, rhs, ra_idx, var_idx, &written_vars, stack_base);
                let res = b.ins().iadd(vl, vr);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::SubRR { dest, lhs, rhs } => {
                let vl  = read_reg(&mut b, lhs, ra_idx, var_idx, &written_vars, stack_base);
                let vr  = read_reg(&mut b, rhs, ra_idx, var_idx, &written_vars, stack_base);
                let res = b.ins().isub(vl, vr);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::MulRR { dest, lhs, rhs } => {
                let vl  = read_reg(&mut b, lhs, ra_idx, var_idx, &written_vars, stack_base);
                let vr  = read_reg(&mut b, rhs, ra_idx, var_idx, &written_vars, stack_base);
                let res = b.ins().imul(vl, vr);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::IDivRR { dest, lhs, rhs } => {
                // Lua floor division.  Zero-divisor → deopt (return -1).
                // Since we haven't written any output yet when checking the
                // divisor we can safely deopt here.
                let vl   = read_reg(&mut b, lhs, ra_idx, var_idx, &written_vars, stack_base);
                let vr   = read_reg(&mut b, rhs, ra_idx, var_idx, &written_vars, stack_base);
                let zero = b.ins().iconst(I64, 0);
                let nz   = b.ins().icmp(IntCC::NotEqual, vr, zero);
                let ok_block  = b.create_block();
                b.seal_block(ok_block);
                b.ins().brif(nz, ok_block, &[], deopt_block, &[]);
                b.switch_to_block(ok_block);

                let res = emit_idiv(&mut b, vl, vr);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::ModRR { dest, lhs, rhs } => {
                let vl   = read_reg(&mut b, lhs, ra_idx, var_idx, &written_vars, stack_base);
                let vr   = read_reg(&mut b, rhs, ra_idx, var_idx, &written_vars, stack_base);
                let zero = b.ins().iconst(I64, 0);
                let nz   = b.ins().icmp(IntCC::NotEqual, vr, zero);
                let ok_block  = b.create_block();
                b.seal_block(ok_block);
                b.ins().brif(nz, ok_block, &[], deopt_block, &[]);
                b.switch_to_block(ok_block);

                let res = emit_imod(&mut b, vl, vr);
                write_reg(&mut b, dest, res, &written_vars);
            }
            // ── Register-immediate arithmetic ─────────────────────────────
            BodyInstr::AddImm { dest, src, imm } => {
                let vs      = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let imm_val = b.ins().iconst(I64, imm);
                let res     = b.ins().iadd(vs, imm_val);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::SubImm { dest, src, imm } => {
                let vs      = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let imm_val = b.ins().iconst(I64, imm);
                let res     = b.ins().isub(vs, imm_val);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::MulImm { dest, src, imm } => {
                let vs      = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let imm_val = b.ins().iconst(I64, imm);
                let res     = b.ins().imul(vs, imm_val);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::IDivImm { dest, src, imm } => {
                // Constant divisor is guaranteed non-zero by the analyzer.
                let vs  = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let ki  = b.ins().iconst(I64, imm);
                let res = emit_idiv(&mut b, vs, ki);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::ModImm { dest, src, imm } => {
                let vs  = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let ki  = b.ins().iconst(I64, imm);
                let res = emit_imod(&mut b, vs, ki);
                write_reg(&mut b, dest, res, &written_vars);
            }
            // ── Bitwise register-immediate ────────────────────────────────
            BodyInstr::BAndImm { dest, src, imm } => {
                let vs  = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let ki  = b.ins().iconst(I64, imm);
                let res = b.ins().band(vs, ki);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::BOrImm { dest, src, imm } => {
                let vs  = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let ki  = b.ins().iconst(I64, imm);
                let res = b.ins().bor(vs, ki);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::BXorImm { dest, src, imm } => {
                let vs  = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let ki  = b.ins().iconst(I64, imm);
                let res = b.ins().bxor(vs, ki);
                write_reg(&mut b, dest, res, &written_vars);
            }
            // ── Bitwise register-register ─────────────────────────────────
            BodyInstr::BAndRR { dest, lhs, rhs } => {
                let vl  = read_reg(&mut b, lhs, ra_idx, var_idx, &written_vars, stack_base);
                let vr  = read_reg(&mut b, rhs, ra_idx, var_idx, &written_vars, stack_base);
                let res = b.ins().band(vl, vr);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::BOrRR { dest, lhs, rhs } => {
                let vl  = read_reg(&mut b, lhs, ra_idx, var_idx, &written_vars, stack_base);
                let vr  = read_reg(&mut b, rhs, ra_idx, var_idx, &written_vars, stack_base);
                let res = b.ins().bor(vl, vr);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::BXorRR { dest, lhs, rhs } => {
                let vl  = read_reg(&mut b, lhs, ra_idx, var_idx, &written_vars, stack_base);
                let vr  = read_reg(&mut b, rhs, ra_idx, var_idx, &written_vars, stack_base);
                let res = b.ins().bxor(vl, vr);
                write_reg(&mut b, dest, res, &written_vars);
            }
            // ── Unary ops ─────────────────────────────────────────────────
            BodyInstr::Unm { dest, src } => {
                let vs  = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let res = b.ins().ineg(vs);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::BNot { dest, src } => {
                let vs  = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let res = b.ins().bnot(vs);
                write_reg(&mut b, dest, res, &written_vars);
            }
            // ── Data movement ─────────────────────────────────────────────
            BodyInstr::Move { dest, src } => {
                let vs  = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                write_reg(&mut b, dest, vs, &written_vars);
            }
            BodyInstr::LoadI { dest, imm } => {
                let cv = b.ins().iconst(I64, imm);
                write_reg(&mut b, dest, cv, &written_vars);
            }
            // ── Shift ops ─────────────────────────────────────────────────
            BodyInstr::ShrImm { dest, src, imm } => {
                // lua_shiftr(R[src], imm) with compile-time constant imm.
                // Select the right instruction at JIT compile time — no branches needed.
                let vs = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let res = if imm == 0 {
                    vs
                } else if imm >= 64 || imm <= -64 {
                    b.ins().iconst(I64, 0)
                } else if imm > 0 {
                    b.ins().ushr_imm(vs, imm)
                } else {
                    // negative imm: right-shift becomes left-shift
                    b.ins().ishl_imm(vs, -imm)
                };
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::ShlIConst { dest, src, imm } => {
                // lua_shiftl(imm_const, R[src]):  constant VALUE shifted left by variable COUNT.
                let count = read_reg(&mut b, src, ra_idx, var_idx, &written_vars, stack_base);
                let value = b.ins().iconst(I64, imm);
                let res   = emit_lua_shiftl(&mut b, value, count);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::ShlRR { dest, lhs, rhs } => {
                // lua_shiftl(R[lhs], R[rhs])
                let vl  = read_reg(&mut b, lhs, ra_idx, var_idx, &written_vars, stack_base);
                let vr  = read_reg(&mut b, rhs, ra_idx, var_idx, &written_vars, stack_base);
                let res = emit_lua_shiftl(&mut b, vl, vr);
                write_reg(&mut b, dest, res, &written_vars);
            }
            BodyInstr::ShrRR { dest, lhs, rhs } => {
                // lua_shiftr(R[lhs], R[rhs]) = lua_shiftl(R[lhs], -R[rhs])
                let vl      = read_reg(&mut b, lhs, ra_idx, var_idx, &written_vars, stack_base);
                let vr      = read_reg(&mut b, rhs, ra_idx, var_idx, &written_vars, stack_base);
                let neg_vr  = b.ins().ineg(vr);
                let res     = emit_lua_shiftl(&mut b, vl, neg_vr);
                write_reg(&mut b, dest, res, &written_vars);
            }
        }
    }
    b.ins().jump(epilog_block, &[]);

    // ── BLOCK: epilog ─────────────────────────────────────────────────────
    // Mirrors interpreter's ForLoop: if count > 0, decrement count, advance
    // idx, loop back.  When count == 0 the last body iteration has already
    // executed, so we fall through to exit without another idx advance.
    b.seal_block(epilog_block); // one predecessor: body_block
    b.switch_to_block(epilog_block);

    let count_now = b.use_var(var_count);
    let zero      = b.ins().iconst(I64, 0);
    let more      = b.ins().icmp(IntCC::UnsignedGreaterThan, count_now, zero);

    // On the "more iterations" path — update count and idx, jump back.
    // On the "done" path — jump to exit (idx and count already at final values).
    let update_block = b.create_block();
    b.seal_block(update_block); // one predecessor: epilog brif
    b.ins().brif(more, update_block, &[], exit_block, &[]);

    b.switch_to_block(update_block);
    let one       = b.ins().iconst(I64, 1);
    let new_count = b.ins().isub(count_now, one);
    let idx_now   = b.use_var(var_idx);
    let new_idx   = b.ins().iadd(idx_now, step_val);
    b.def_var(var_count, new_count);
    b.def_var(var_idx,   new_idx);
    b.ins().jump(body_block, &[]);

    // Now both predecessors of body_block are connected — seal it.
    b.seal_block(body_block);
    b.seal_block(exit_block);

    // ── BLOCK: exit ───────────────────────────────────────────────────────
    b.switch_to_block(exit_block);

    // Final count is 0 (the epilog fell through instead of looping).
    let final_count = b.ins().iconst(I64, 0);
    // Final idx is the value after the last body execution (not incremented again).
    let final_idx   = b.use_var(var_idx);
    b.ins().store(MemFlags::trusted(), final_count, stack_base, val_off(ra_cnt));
    b.ins().store(MemFlags::trusted(), final_idx,   stack_base, val_off(ra_idx));

    for &(r, var) in &written_vars {
        let final_val = b.use_var(var);
        b.ins().store(MemFlags::trusted(), final_val, stack_base, val_off(r));
    }
    let ret_ok = b.ins().iconst(I32, 0);
    b.ins().return_(&[ret_ok]);

    // ── BLOCK: deopt ──────────────────────────────────────────────────────
    b.seal_block(deopt_block);
    b.switch_to_block(deopt_block);
    let ret_deopt = b.ins().iconst(I32, -1);
    b.ins().return_(&[ret_deopt]);

    b.finalize();
    true
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Read register `r` as an SSA value:
/// - If `r == ra_idx`: loop control variable (use `var_idx`)  
/// - If `r` is a written reg: use its Variable  
/// - Otherwise: load from stack (read-only, dominated by pre_loop)
fn read_reg(
    b:           &mut FunctionBuilder,
    r:           u8,
    ra_idx:      u8,
    var_idx:     Variable,
    written_vars: &[(u8, Variable)],
    stack_base:  cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    if r == ra_idx {
        return b.use_var(var_idx);
    }
    if let Some(&(_, var)) = written_vars.iter().find(|&&(rr, _)| rr == r) {
        return b.use_var(var);
    }
    // read-only register — load directly (safe: pre_loop_block dominates here)
    b.ins().load(I64, MemFlags::trusted(), stack_base, val_off(r))
}

/// Update register `r`'s SSA variable to `val`.
fn write_reg(
    b:           &mut FunctionBuilder,
    r:           u8,
    val:         cranelift_codegen::ir::Value,
    written_vars: &[(u8, Variable)],
) {
    if let Some(&(_, var)) = written_vars.iter().find(|&&(rr, _)| rr == r) {
        b.def_var(var, val);
    }
    // If r isn't in written_vars this is a logic error in the analyzer, silently ignore.
}

// ── Lua-semantics arithmetic helpers ─────────────────────────────────────────

/// Emit Cranelift IR for Lua integer floor division (`a // b`).
///
/// Lua uses floor division (toward −∞), while Cranelift's `sdiv` truncates
/// toward zero.  The correction needed is:
///   `floor_div = trunc_div - (remainder != 0 AND sign(a) != sign(b))`
///
/// The XOR-sign trick: `bxor(a, b) < 0  ⟺  a and b have different signs`.
fn emit_idiv(
    b:   &mut FunctionBuilder,
    lhs: cranelift_codegen::ir::Value,
    rhs: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    use cranelift_codegen::ir::types::I64;
    let q    = b.ins().sdiv(lhs, rhs);
    let r    = b.ins().srem(lhs, rhs);
    let zero = b.ins().iconst(I64, 0);
    // r != 0  (boolean i8: 0 or 1)
    let r_ne   = b.ins().icmp(IntCC::NotEqual, r, zero);
    // different signs iff bxor(lhs, rhs) < 0
    let xors   = b.ins().bxor(lhs, rhs);
    let diff_s = b.ins().icmp(IntCC::SignedLessThan, xors, zero);
    // correction needed iff both conditions hold
    let adj8   = b.ins().band(r_ne, diff_s);
    let adj    = b.ins().uextend(I64, adj8);
    b.ins().isub(q, adj)
}

/// Emit Cranelift IR for Lua integer modulo (`a % b`).
///
/// C `srem` returns the remainder with the sign of the dividend.
/// Lua `%` returns the remainder with the sign of the divisor.
/// Correction: if `srem != 0` and the result has the wrong sign, add `b`.
///
/// Formula matching `lua_imod`:
///   r = srem(a, b)
///   if r != 0 AND sign(r) != sign(b): r += b
fn emit_imod(
    b:   &mut FunctionBuilder,
    lhs: cranelift_codegen::ir::Value,
    rhs: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    use cranelift_codegen::ir::types::I64;
    let r    = b.ins().srem(lhs, rhs);
    let zero = b.ins().iconst(I64, 0);
    let r_ne   = b.ins().icmp(IntCC::NotEqual, r, zero);
    // sign(r) != sign(rhs) iff bxor(r, rhs) < 0
    let xors   = b.ins().bxor(r, rhs);
    let diff_s = b.ins().icmp(IntCC::SignedLessThan, xors, zero);
    let needs  = b.ins().band(r_ne, diff_s);  // i8: 0 or 1
    // add rhs if correction needed, 0 otherwise
    let needs64 = b.ins().uextend(I64, needs);        // 0 or 1
    let addend  = b.ins().imul(needs64, rhs);          // 0 or rhs
    b.ins().iadd(r, addend)
}

/// Emit Cranelift IR for Lua's `lua_shiftl(y, disp)`:
///   if  0 ≤ disp < 64  → y << disp   (logical left shift)
///   if -64 < disp < 0  → y >> (-disp) (logical right shift)
///   if |disp| ≥ 64     → 0
///
/// This is branchless: we compute both possible shifts and select with `select`.
/// Out-of-range intermediate results are harmless because they are never selected.
fn emit_lua_shiftl(
    b:    &mut FunctionBuilder,
    y:    cranelift_codegen::ir::Value,
    disp: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    use cranelift_codegen::ir::types::I64;
    let zero    = b.ins().iconst(I64, 0);
    let c63     = b.ins().iconst(I64, 63);
    let neg_c64 = b.ins().iconst(I64, -64i64);

    // Condition: 0 ≤ disp ≤ 63  →  apply ishl
    let non_neg = b.ins().icmp(IntCC::SignedGreaterThanOrEqual, disp, zero);
    let lt64    = b.ins().icmp(IntCC::SignedLessThanOrEqual,    disp, c63);
    let do_shl  = b.ins().band(non_neg, lt64);

    // Condition: -64 < disp < 0  →  apply ushr(-disp)
    let is_neg   = b.ins().icmp(IntCC::SignedLessThan,         disp, zero);
    let gt_neg64 = b.ins().icmp(IntCC::SignedGreaterThan,      disp, neg_c64);
    let do_shr   = b.ins().band(is_neg, gt_neg64);

    // Compute shl: Cranelift uses low 6 bits, which is correct for disp in [0, 63].
    let shl_res = b.ins().ishl(y, disp);

    // Compute shr: need -disp; for disp in (-64, 0), -disp is in (0, 64] which is valid.
    let neg_disp = b.ins().ineg(disp);
    let shr_res  = b.ins().ushr(y, neg_disp);

    // Select: do_shl → shl_res; else do_shr → shr_res; else → 0
    let shr_or_zero = b.ins().select(do_shr, shr_res, zero);
    b.ins().select(do_shl, shl_res, shr_or_zero)
}
