//! Trace compiler — lowers Trace IR to native code via Cranelift.
//!
//! Compiled trace signature: `fn(stack: *mut LuaValue, base: usize) -> i32`
//! Returns 0 on successful loop iteration, or `snap_id > 0` on guard failure.

use cranelift_codegen::ir::{
    AbiParam, InstBuilder, MemFlags,
    condcodes::{IntCC, FloatCC},
    types::{I8, I32, I64, F64},
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::Module;

use super::runtime::with_module;
use super::trace::{Trace, TraceIr, IrType, CmpOp};

const LV: i32 = 16;
const VALUE_OFF: i32 = 0;
const TAG_OFF: i32 = 8;

const TAG_INT: i64 = 0x03;
const TAG_FLOAT: i64 = 0x13;
const TAG_TABLE: i64 = 0x45;
const TAG_SSTR: i64 = 0x44;
const TAG_TRUE: i64 = 0x11;
const TAG_NIL: i64 = 0x00;
const TAG_LCL: i64 = 0x66;

#[inline(always)]
fn val_off(slot: u16) -> i32 { slot as i32 * LV + VALUE_OFF }
#[inline(always)]
fn tag_off(slot: u16) -> i32 { slot as i32 * LV + TAG_OFF }

fn ir_type_tag(ty: &IrType) -> i64 {
    match ty {
        IrType::Int => TAG_INT,
        IrType::Float => TAG_FLOAT,
        IrType::Table => TAG_TABLE,
        IrType::String => TAG_SSTR,
        IrType::Bool => TAG_TRUE,
        IrType::Nil => TAG_NIL,
        IrType::Function => TAG_LCL,
    }
}

fn ir_result_type(op: &TraceIr) -> cranelift_codegen::ir::Type {
    match op {
        TraceIr::KFloat(_)
        | TraceIr::AddFloat { .. } | TraceIr::SubFloat { .. }
        | TraceIr::MulFloat { .. } | TraceIr::DivFloat { .. }
        | TraceIr::PowFloat { .. } | TraceIr::NegFloat { .. }
        | TraceIr::IntToFloat { .. } => F64,
        _ => I64,
    }
}

fn as_f64(b: &mut FunctionBuilder, v: cranelift_codegen::ir::Value) -> cranelift_codegen::ir::Value {
    if b.func.dfg.value_type(v) == I64 { b.ins().bitcast(F64, MemFlags::new(), v) } else { v }
}

fn as_i64(b: &mut FunctionBuilder, v: cranelift_codegen::ir::Value) -> cranelift_codegen::ir::Value {
    if b.func.dfg.value_type(v) == F64 { b.ins().bitcast(I64, MemFlags::new(), v) } else { v }
}

fn cmp_to_intcc(cmp: &CmpOp) -> IntCC {
    match cmp {
        CmpOp::Lt => IntCC::SignedLessThan,
        CmpOp::Le => IntCC::SignedLessThanOrEqual,
        CmpOp::Gt => IntCC::SignedGreaterThan,
        CmpOp::Ge => IntCC::SignedGreaterThanOrEqual,
        CmpOp::Eq => IntCC::Equal,
        CmpOp::Ne => IntCC::NotEqual,
    }
}

fn cmp_to_floatcc(cmp: &CmpOp) -> FloatCC {
    match cmp {
        CmpOp::Lt => FloatCC::LessThan,
        CmpOp::Le => FloatCC::LessThanOrEqual,
        CmpOp::Gt => FloatCC::GreaterThan,
        CmpOp::Ge => FloatCC::GreaterThanOrEqual,
        CmpOp::Eq => FloatCC::Equal,
        CmpOp::Ne => FloatCC::NotEqual,
    }
}

pub struct CompiledTrace {
    pub trace_id: u32,
    pub fn_ptr: *const u8,
    pub chunk_ptr: *const u8,
    pub head_pc: u32,
    /// PC to resume at for each side-exit (indexed by snap_id - 1).
    pub exit_pcs: Vec<u32>,
}

unsafe impl Send for CompiledTrace {}
unsafe impl Sync for CompiledTrace {}

pub type TraceFn = unsafe fn(*mut u8, usize, *const u8) -> i32;

/// Collected Cranelift FuncRef handles for external helpers.
struct HelperFuncs {
    pow: Option<cranelift_codegen::ir::FuncRef>,
    tab_geti: Option<cranelift_codegen::ir::FuncRef>,
    tab_seti: Option<cranelift_codegen::ir::FuncRef>,
    tab_gets: Option<cranelift_codegen::ir::FuncRef>,
    tab_sets: Option<cranelift_codegen::ir::FuncRef>,
    tab_len: Option<cranelift_codegen::ir::FuncRef>,
}

pub fn compile_trace(trace: &Trace) -> Result<CompiledTrace, String> {
    with_module(|module| {
        let ptr_type = module.target_config().pointer_type();
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr_type));
        sig.params.push(AbiParam::new(I64));
        sig.params.push(AbiParam::new(ptr_type)); // upvalue_ptrs
        sig.returns.push(AbiParam::new(I32));

        let func_name = format!("trace_{}", trace.id);
        let func_id = module
            .declare_function(&func_name, cranelift_module::Linkage::Local, &sig)
            .map_err(|e| format!("declare: {e}"))?;

        // Declare `pow(f64,f64)->f64` if this trace uses PowFloat.
        let needs_pow = trace.ops.iter().any(|op| matches!(op, TraceIr::PowFloat { .. }));
        let pow_func_id = if needs_pow {
            let mut pow_sig = module.make_signature();
            pow_sig.params.push(AbiParam::new(F64));
            pow_sig.params.push(AbiParam::new(F64));
            pow_sig.returns.push(AbiParam::new(F64));
            Some(module.declare_function("pow", cranelift_module::Linkage::Import, &pow_sig)
                .map_err(|e| format!("declare pow: {e}"))?)
        } else { None };

        // Declare table helper functions if this trace uses table ops.
        let needs_tab_geti = trace.ops.iter().any(|op| matches!(op, TraceIr::TabGetI { .. }));
        let needs_tab_seti = trace.ops.iter().any(|op| matches!(op, TraceIr::TabSetI { .. }));
        let needs_tab_gets = trace.ops.iter().any(|op| matches!(op, TraceIr::TabGetS { .. }));
        let needs_tab_sets = trace.ops.iter().any(|op| matches!(op, TraceIr::TabSetS { .. }));
        let needs_tab_len = trace.ops.iter().any(|op| matches!(op, TraceIr::TabLen { .. }));

        // jit_tab_geti(i64, i64) -> i64
        let tab_geti_id = if needs_tab_geti {
            let mut s = module.make_signature();
            s.params.push(AbiParam::new(I64));
            s.params.push(AbiParam::new(I64));
            s.returns.push(AbiParam::new(I64));
            Some(module.declare_function("jit_tab_geti", cranelift_module::Linkage::Import, &s)
                .map_err(|e| format!("declare jit_tab_geti: {e}"))?)
        } else { None };

        // jit_tab_seti(i64, i64, i64, i64) -> void
        let tab_seti_id = if needs_tab_seti {
            let mut s = module.make_signature();
            s.params.push(AbiParam::new(I64));
            s.params.push(AbiParam::new(I64));
            s.params.push(AbiParam::new(I64));
            s.params.push(AbiParam::new(I64));
            Some(module.declare_function("jit_tab_seti", cranelift_module::Linkage::Import, &s)
                .map_err(|e| format!("declare jit_tab_seti: {e}"))?)
        } else { None };

        // jit_tab_gets(i64, i64) -> i64
        let tab_gets_id = if needs_tab_gets {
            let mut s = module.make_signature();
            s.params.push(AbiParam::new(I64));
            s.params.push(AbiParam::new(I64));
            s.returns.push(AbiParam::new(I64));
            Some(module.declare_function("jit_tab_gets", cranelift_module::Linkage::Import, &s)
                .map_err(|e| format!("declare jit_tab_gets: {e}"))?)
        } else { None };

        // jit_tab_sets(i64, i64, i64, i64) -> void
        let tab_sets_id = if needs_tab_sets {
            let mut s = module.make_signature();
            s.params.push(AbiParam::new(I64));
            s.params.push(AbiParam::new(I64));
            s.params.push(AbiParam::new(I64));
            s.params.push(AbiParam::new(I64));
            Some(module.declare_function("jit_tab_sets", cranelift_module::Linkage::Import, &s)
                .map_err(|e| format!("declare jit_tab_sets: {e}"))?)
        } else { None };

        // jit_tab_len(i64) -> i64
        let tab_len_id = if needs_tab_len {
            let mut s = module.make_signature();
            s.params.push(AbiParam::new(I64));
            s.returns.push(AbiParam::new(I64));
            Some(module.declare_function("jit_tab_len", cranelift_module::Linkage::Import, &s)
                .map_err(|e| format!("declare jit_tab_len: {e}"))?)
        } else { None };

        let mut ctx = module.make_context();
        ctx.func.signature = sig;

        // Import helpers into the function.
        let helpers = HelperFuncs {
            pow: pow_func_id.map(|fid| module.declare_func_in_func(fid, &mut ctx.func)),
            tab_geti: tab_geti_id.map(|fid| module.declare_func_in_func(fid, &mut ctx.func)),
            tab_seti: tab_seti_id.map(|fid| module.declare_func_in_func(fid, &mut ctx.func)),
            tab_gets: tab_gets_id.map(|fid| module.declare_func_in_func(fid, &mut ctx.func)),
            tab_sets: tab_sets_id.map(|fid| module.declare_func_in_func(fid, &mut ctx.func)),
            tab_len: tab_len_id.map(|fid| module.declare_func_in_func(fid, &mut ctx.func)),
        };

        let mut fb_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            emit_trace_ir(&mut builder, trace, &helpers)?;
            builder.finalize();
        }

        module.define_function(func_id, &mut ctx)
            .map_err(|e| format!("define: {e}"))?;
        module.finalize_definitions()
            .map_err(|e| format!("finalize: {e}"))?;

        let code_ptr = module.get_finalized_function(func_id);
        let exit_pcs: Vec<u32> = trace.snapshots.iter().map(|s| s.pc).collect();
        Ok(CompiledTrace {
            trace_id: trace.id,
            fn_ptr: code_ptr as *const u8,
            chunk_ptr: trace.chunk_ptr,
            head_pc: trace.head_pc,
            exit_pcs,
        })
    })
}

fn emit_trace_ir(b: &mut FunctionBuilder, trace: &Trace, helpers: &HelperFuncs) -> Result<(), String> {
    let entry = b.create_block();
    b.append_block_params_for_function_params(entry);
    let exit_blocks: Vec<_> = (0..trace.snapshots.len()).map(|_| b.create_block()).collect();
    let loop_block = b.create_block();
    let mut loop_sealed = false;

    b.switch_to_block(entry);
    b.seal_block(entry);
    let stack_ptr = b.block_params(entry)[0];
    let base_val = b.block_params(entry)[1];
    let upval_ptrs = b.block_params(entry)[2];
    let lv = b.ins().iconst(I64, LV as i64);
    let base_off = b.ins().imul(base_val, lv);
    let sbase = b.ins().iadd(stack_ptr, base_off);

    // Pre-scan Phi nodes to allocate Cranelift Variables
    struct PhiInfo { var: Variable, entry_ref: usize, backedge_ref: usize }
    let mut phis: Vec<PhiInfo> = Vec::new();
    for op in &trace.ops {
        if let TraceIr::Phi { entry, backedge, .. } = op {
            let cl_ty = ir_result_type(&trace.ops[entry.index()]);
            let var = b.declare_var(cl_ty);
            phis.push(PhiInfo { var, entry_ref: entry.index(), backedge_ref: backedge.index() });
        }
    }

    // Safety counter: prevent infinite loops in compiled traces
    let var_counter = b.declare_var(I64);
    let safety_exit = b.create_block();
    let counter_init = b.ins().iconst(I64, 1_000_000);
    b.def_var(var_counter, counter_init);

    let mut vals: Vec<cranelift_codegen::ir::Value> = Vec::with_capacity(trace.ops.len());
    let mut phi_idx: usize = 0;

    for (i, op) in trace.ops.iter().enumerate() {
        let v = match op {
            TraceIr::GuardType { slot, expected, snap_id } => {
                let tag = b.ins().load(I8, MemFlags::trusted(), sbase, tag_off(*slot));
                let exp = b.ins().iconst(I8, ir_type_tag(expected));
                let ok = b.ins().icmp(IntCC::Equal, tag, exp);
                let cont = b.create_block();
                b.ins().brif(ok, cont, &[], exit_blocks[*snap_id as usize], &[]);
                b.seal_block(cont);
                b.switch_to_block(cont);
                b.ins().iconst(I64, 0)
            }
            TraceIr::GuardCmpI { lhs, rhs_imm, cmp, snap_id } => {
                let rhs = b.ins().iconst(I64, *rhs_imm);
                let cc = cmp_to_intcc(cmp);
                let ok = b.ins().icmp(cc, vals[lhs.index()], rhs);
                let cont = b.create_block();
                b.ins().brif(ok, cont, &[], exit_blocks[*snap_id as usize], &[]);
                b.seal_block(cont);
                b.switch_to_block(cont);
                b.ins().iconst(I64, 0)
            }
            TraceIr::GuardCmpRR { lhs, rhs, cmp, snap_id } => {
                let cc = cmp_to_intcc(cmp);
                let ok = b.ins().icmp(cc, vals[lhs.index()], vals[rhs.index()]);
                let cont = b.create_block();
                b.ins().brif(ok, cont, &[], exit_blocks[*snap_id as usize], &[]);
                b.seal_block(cont);
                b.switch_to_block(cont);
                b.ins().iconst(I64, 0)
            }
            TraceIr::GuardCmpF { lhs, rhs, cmp, snap_id } => {
                let cc = cmp_to_floatcc(cmp);
                let fl = as_f64(b, vals[lhs.index()]);
                let fr = as_f64(b, vals[rhs.index()]);
                let ok = b.ins().fcmp(cc, fl, fr);
                let cont = b.create_block();
                b.ins().brif(ok, cont, &[], exit_blocks[*snap_id as usize], &[]);
                b.seal_block(cont);
                b.switch_to_block(cont);
                b.ins().iconst(I64, 0)
            }
            TraceIr::GuardTruthy { .. } => {
                // After GuardType, truthiness is determined by type alone;
                // Int/Float/String/Table/Function are always truthy.
                b.ins().iconst(I64, 0)
            }
            TraceIr::KInt(n) => b.ins().iconst(I64, *n),
            TraceIr::KFloat(n) => b.ins().f64const(*n),
            TraceIr::LoadSlot { slot } =>
                b.ins().load(I64, MemFlags::trusted(), sbase, val_off(*slot)),
            TraceIr::StoreSlot { slot, val, ty } => {
                let sv = as_i64(b, vals[val.index()]);
                b.ins().store(MemFlags::trusted(), sv, sbase, val_off(*slot));
                let tg = b.ins().iconst(I8, ir_type_tag(ty));
                b.ins().store(MemFlags::trusted(), tg, sbase, tag_off(*slot));
                b.ins().iconst(I64, 0)
            }
            TraceIr::LoadUpval { upval_idx } => {
                // upvalue_ptrs is *const UpvaluePtr (array of GcPtr<GcUpvalue>, each 8 bytes).
                // Chain: upvalue_ptrs[idx].ptr → Gc<LuaUpvalue> → .data.v → *LuaValue
                // GcHeader is 8 bytes, so LuaUpvalue.v is at offset 8 in Gc<LuaUpvalue>.
                let off = b.ins().iconst(I64, *upval_idx as i64 * 8);
                let upval_slot = b.ins().iadd(upval_ptrs, off);
                // Load GcPtr.ptr (raw pointer to Gc<LuaUpvalue>)
                let gc_ptr = b.ins().load(I64, MemFlags::trusted(), upval_slot, 0);
                // Load LuaUpvalue.v (*mut LuaValue) at offset 8 (after GcHeader)
                let v_ptr = b.ins().load(I64, MemFlags::trusted(), gc_ptr, 8);
                // Load value payload from v_ptr
                b.ins().load(I64, MemFlags::trusted(), v_ptr, VALUE_OFF)
            }
            TraceIr::StoreUpval { upval_idx, val, ty } => {
                let off = b.ins().iconst(I64, *upval_idx as i64 * 8);
                let upval_slot = b.ins().iadd(upval_ptrs, off);
                let gc_ptr = b.ins().load(I64, MemFlags::trusted(), upval_slot, 0);
                let v_ptr = b.ins().load(I64, MemFlags::trusted(), gc_ptr, 8);
                let sv = as_i64(b, vals[val.index()]);
                b.ins().store(MemFlags::trusted(), sv, v_ptr, VALUE_OFF);
                let tg = b.ins().iconst(I8, ir_type_tag(ty));
                b.ins().store(MemFlags::trusted(), tg, v_ptr, TAG_OFF);
                b.ins().iconst(I64, 0)
            }
            TraceIr::Move { src } => vals[src.index()],
            TraceIr::IntToFloat { src } => b.ins().fcvt_from_sint(F64, vals[src.index()]),
            TraceIr::NegInt { src } => b.ins().ineg(vals[src.index()]),
            TraceIr::NegFloat { src } => {
                let v = as_f64(b, vals[src.index()]); b.ins().fneg(v)
            }
            TraceIr::AddInt { lhs, rhs } => b.ins().iadd(vals[lhs.index()], vals[rhs.index()]),
            TraceIr::SubInt { lhs, rhs } => b.ins().isub(vals[lhs.index()], vals[rhs.index()]),
            TraceIr::MulInt { lhs, rhs } => b.ins().imul(vals[lhs.index()], vals[rhs.index()]),
            TraceIr::IDivInt { lhs, rhs } => emit_idiv(b, vals[lhs.index()], vals[rhs.index()]),
            TraceIr::ModInt { lhs, rhs } => emit_imod(b, vals[lhs.index()], vals[rhs.index()]),
            TraceIr::AddFloat { lhs, rhs } => {
                let l = as_f64(b, vals[lhs.index()]); let r = as_f64(b, vals[rhs.index()]);
                b.ins().fadd(l, r)
            }
            TraceIr::SubFloat { lhs, rhs } => {
                let l = as_f64(b, vals[lhs.index()]); let r = as_f64(b, vals[rhs.index()]);
                b.ins().fsub(l, r)
            }
            TraceIr::MulFloat { lhs, rhs } => {
                let l = as_f64(b, vals[lhs.index()]); let r = as_f64(b, vals[rhs.index()]);
                b.ins().fmul(l, r)
            }
            TraceIr::DivFloat { lhs, rhs } => {
                let l = as_f64(b, vals[lhs.index()]); let r = as_f64(b, vals[rhs.index()]);
                b.ins().fdiv(l, r)
            }
            TraceIr::PowFloat { lhs, rhs } => {
                let l = as_f64(b, vals[lhs.index()]); let r = as_f64(b, vals[rhs.index()]);
                let fref = helpers.pow.expect("pow fref must be set for PowFloat");
                let call = b.ins().call(fref, &[l, r]);
                b.inst_results(call)[0]
            }
            TraceIr::BAndInt { lhs, rhs } => b.ins().band(vals[lhs.index()], vals[rhs.index()]),
            TraceIr::BOrInt { lhs, rhs } => b.ins().bor(vals[lhs.index()], vals[rhs.index()]),
            TraceIr::BXorInt { lhs, rhs } => b.ins().bxor(vals[lhs.index()], vals[rhs.index()]),
            TraceIr::BNotInt { src } => b.ins().bnot(vals[src.index()]),
            TraceIr::ShlInt { lhs, rhs } =>
                emit_lua_shiftl(b, vals[lhs.index()], vals[rhs.index()]),
            TraceIr::ShrInt { lhs, rhs } => {
                let neg = b.ins().ineg(vals[rhs.index()]);
                emit_lua_shiftl(b, vals[lhs.index()], neg)
            }
            // ── Table operations ───────────────────────────────────────
            TraceIr::TabGetI { table, index } => {
                let fref = helpers.tab_geti.expect("tab_geti fref");
                let call = b.ins().call(fref, &[vals[table.index()], vals[index.index()]]);
                b.inst_results(call)[0]
            }
            TraceIr::TabSetI { table, index, val, ty } => {
                let fref = helpers.tab_seti.expect("tab_seti fref");
                let sv = as_i64(b, vals[val.index()]);
                let tag = b.ins().iconst(I64, ir_type_tag(ty));
                b.ins().call(fref, &[vals[table.index()], vals[index.index()], sv, tag]);
                b.ins().iconst(I64, 0) // dummy result
            }
            TraceIr::TabGetS { table, key_ptr } => {
                let fref = helpers.tab_gets.expect("tab_gets fref");
                let kp = b.ins().iconst(I64, *key_ptr as i64);
                let call = b.ins().call(fref, &[vals[table.index()], kp]);
                b.inst_results(call)[0]
            }
            TraceIr::TabSetS { table, key_ptr, val, ty } => {
                let fref = helpers.tab_sets.expect("tab_sets fref");
                let kp = b.ins().iconst(I64, *key_ptr as i64);
                let sv = as_i64(b, vals[val.index()]);
                let tag = b.ins().iconst(I64, ir_type_tag(ty));
                b.ins().call(fref, &[vals[table.index()], kp, sv, tag]);
                b.ins().iconst(I64, 0) // dummy result
            }
            TraceIr::TabLen { table } => {
                let fref = helpers.tab_len.expect("tab_len fref");
                let call = b.ins().call(fref, &[vals[table.index()]]);
                b.inst_results(call)[0]
            }
            TraceIr::LoopStart => {
                for phi in &phis { b.def_var(phi.var, vals[phi.entry_ref]); }
                b.ins().jump(loop_block, &[]);
                b.switch_to_block(loop_block);
                b.ins().iconst(I64, 0)
            }
            TraceIr::Phi { .. } => {
                let v = b.use_var(phis[phi_idx].var);
                phi_idx += 1;
                v
            }
            TraceIr::LoopEnd => {
                for phi in &phis { b.def_var(phi.var, vals[phi.backedge_ref]); }
                // Decrement safety counter; exit if exhausted
                let cnt = b.use_var(var_counter);
                let one = b.ins().iconst(I64, 1);
                let new_cnt = b.ins().isub(cnt, one);
                b.def_var(var_counter, new_cnt);
                let zero64 = b.ins().iconst(I64, 0);
                let expired = b.ins().icmp(IntCC::SignedLessThanOrEqual, new_cnt, zero64);
                b.ins().brif(expired, safety_exit, &[], loop_block, &[]);
                b.seal_block(loop_block);
                loop_sealed = true;
                let dead = b.create_block();
                b.switch_to_block(dead);
                b.seal_block(dead);
                b.ins().iconst(I64, 0)
            }
            _ => return Err(format!("NYI: {op:?} at {i}")),
        };
        vals.push(v);
    }

    let zero = b.ins().iconst(I32, 0);
    b.ins().return_(&[zero]);

    if !loop_sealed {
        b.switch_to_block(loop_block);
        b.seal_block(loop_block);
        let z = b.ins().iconst(I32, 0);
        b.ins().return_(&[z]);
    }

    // ── Side-exit blocks: writeback snapshot entries then return snap_id ──
    for (i, &blk) in exit_blocks.iter().enumerate() {
        b.switch_to_block(blk);
        b.seal_block(blk);
        let snap = &trace.snapshots[i];
        for entry in &snap.entries {
            if let super::trace::SnapValue::Ref(tref) = entry.val {
                if (tref.index()) < vals.len() {
                    let sv = as_i64(b, vals[tref.index()]);
                    b.ins().store(MemFlags::trusted(), sv, sbase, val_off(entry.slot));
                    let tg = b.ins().iconst(I8, ir_type_tag(&entry.ty));
                    b.ins().store(MemFlags::trusted(), tg, sbase, tag_off(entry.slot));
                }
            }
        }
        let id = b.ins().iconst(I32, (i as i64) + 1);
        b.ins().return_(&[id]);
    }

    // ── Safety exit: writeback from last snapshot, return 0 ──
    b.switch_to_block(safety_exit);
    b.seal_block(safety_exit);
    if let Some(last_snap) = trace.snapshots.last() {
        for entry in &last_snap.entries {
            if let super::trace::SnapValue::Ref(tref) = entry.val {
                if (tref.index()) < vals.len() {
                    let sv = as_i64(b, vals[tref.index()]);
                    b.ins().store(MemFlags::trusted(), sv, sbase, val_off(entry.slot));
                    let tg = b.ins().iconst(I8, ir_type_tag(&entry.ty));
                    b.ins().store(MemFlags::trusted(), tg, sbase, tag_off(entry.slot));
                }
            }
        }
    }
    let sz = b.ins().iconst(I32, 0);
    b.ins().return_(&[sz]);

    Ok(())
}

// ── Arithmetic helpers ───────────────────────────────────────────────────────

/// Lua floor-division: rounds toward negative infinity.
fn emit_idiv(
    b: &mut FunctionBuilder,
    lhs: cranelift_codegen::ir::Value,
    rhs: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let q = b.ins().sdiv(lhs, rhs);
    let r = b.ins().srem(lhs, rhs);
    let zero = b.ins().iconst(I64, 0);
    let r_ne = b.ins().icmp(IntCC::NotEqual, r, zero);
    let xors = b.ins().bxor(lhs, rhs);
    let diff_s = b.ins().icmp(IntCC::SignedLessThan, xors, zero);
    let adj8 = b.ins().band(r_ne, diff_s);
    let adj = b.ins().uextend(I64, adj8);
    b.ins().isub(q, adj)
}

/// Lua modulo: remainder with sign of divisor.
fn emit_imod(
    b: &mut FunctionBuilder,
    lhs: cranelift_codegen::ir::Value,
    rhs: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let r = b.ins().srem(lhs, rhs);
    let zero = b.ins().iconst(I64, 0);
    let r_ne = b.ins().icmp(IntCC::NotEqual, r, zero);
    let xors = b.ins().bxor(r, rhs);
    let diff_s = b.ins().icmp(IntCC::SignedLessThan, xors, zero);
    let needs = b.ins().band(r_ne, diff_s);
    let needs64 = b.ins().uextend(I64, needs);
    let addend = b.ins().imul(needs64, rhs);
    b.ins().iadd(r, addend)
}

/// Lua bit-shift: `lua_shiftl(y, disp)`.
fn emit_lua_shiftl(
    b: &mut FunctionBuilder,
    y: cranelift_codegen::ir::Value,
    disp: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let zero = b.ins().iconst(I64, 0);
    let c63 = b.ins().iconst(I64, 63);
    let neg64 = b.ins().iconst(I64, -64i64);
    let non_neg = b.ins().icmp(IntCC::SignedGreaterThanOrEqual, disp, zero);
    let lt64 = b.ins().icmp(IntCC::SignedLessThanOrEqual, disp, c63);
    let do_shl = b.ins().band(non_neg, lt64);
    let is_neg = b.ins().icmp(IntCC::SignedLessThan, disp, zero);
    let gt_neg64 = b.ins().icmp(IntCC::SignedGreaterThan, disp, neg64);
    let do_shr = b.ins().band(is_neg, gt_neg64);
    let shl_res = b.ins().ishl(y, disp);
    let neg_disp = b.ins().ineg(disp);
    let shr_res = b.ins().ushr(y, neg_disp);
    let shr_or_zero = b.ins().select(do_shr, shr_res, zero);
    b.ins().select(do_shl, shl_res, shr_or_zero)
}
