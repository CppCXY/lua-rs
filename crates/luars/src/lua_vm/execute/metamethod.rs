use crate::lua_value::LuaValue;
use crate::lua_vm::execute::call::{self, call_c_function};
use crate::lua_vm::execute::helper::{get_binop_metamethod, tonumberns};
use crate::lua_vm::execute::lua_execute;
use crate::lua_vm::opcode::Instruction;
/// Metamethod operations
///
/// Implements MMBIN, MMBINI, MMBINK opcodes
/// Based on Lua 5.5 ltm.c
use crate::lua_vm::{LuaResult, LuaState, get_metamethod_event};
use crate::stdlib::basic::parse_number::parse_lua_number;

/// Try unary metamethod (for __unm, __bnot)
/// Port of luaT_trybinTM for unary operations
pub fn try_unary_tm(
    lua_state: &mut LuaState,
    operand: LuaValue,
    result_pos: usize,
    tm_kind: TmKind,
) -> LuaResult<()> {
    // String coercion for arithmetic unary ops (Unm: -"10" == -10.0)
    if tm_kind == TmKind::Unm {
        let mut n = 0f64;
        if tonumberns(&operand, &mut n) {
            let stack = lua_state.stack_mut();
            stack[result_pos] = LuaValue::float(-n);
            return Ok(());
        }
    } else if tm_kind == TmKind::Bnot {
        if let Some(i) = try_to_integer(&operand) {
            let stack = lua_state.stack_mut();
            stack[result_pos] = LuaValue::integer(!i);
            return Ok(());
        }
    }

    // Try to get metamethod from operand
    let metamethod = get_metamethod_event(lua_state, &operand, tm_kind);
    if let Some(mm) = metamethod {
        // Call metamethod: mm(operand, operand) -> result
        let result = call_tm_res(lua_state, mm, operand, operand)?;

        // Store result
        let stack = lua_state.stack_mut();
        stack[result_pos] = result;
        Ok(())
    } else {
        // No metamethod found
        Err(lua_state.error(format!(
            "attempt to perform '{}' on a {} value",
            tm_kind.name(),
            operand.type_name()
        )))
    }
}

/// Handle MMBIN opcode
/// Call metamethod over R[A] and R[B]
///
/// From lvm.c:
/// ```c
/// vmcase(OP_MMBIN) {
///   StkId ra = RA(i);
///   Instruction pi = *(pc - 2);  /* original arith. expression */
///   TValue *rb = vRB(i);
///   TMS tm = (TMS)GETARG_C(i);
///   StkId result = RA(pi);
///   lua_assert(OP_ADD <= GET_OPCODE(pi) && GET_OPCODE(pi) <= OP_SHR);
///   Protect(luaT_trybinTM(L, s2v(ra), rb, result, tm));
///   vmbreak;
/// }
/// ```
pub fn handle_mmbin(
    lua_state: &mut LuaState,
    _base: usize,         // Unused, kept for compatibility
    a: usize,             // First operand register
    b: usize,             // Second operand register
    c: usize,             // Tag method (TMS)
    pc: usize,            // Current PC
    code: &[Instruction], // Code array to get previous instruction
    frame_idx: usize,     // Frame index for accessing current base
) -> LuaResult<()> {
    // Get the original arithmetic instruction (pc-2) — unchecked since valid bytecode guarantees pc >= 2
    let pi = unsafe { *code.get_unchecked(pc - 2) };
    let result_reg = pi.get_a() as usize;

    // Get base from frame, not parameter (parameter may be stale)
    let base = lua_state.get_frame_base(frame_idx);

    // Get operands — unchecked since stack was validated at frame push
    let stack = lua_state.stack_mut();
    let v1 = unsafe { *stack.get_unchecked(base + a) };
    let v2 = unsafe { *stack.get_unchecked(base + b) };

    // Get tag method — unchecked since compiler guarantees valid TmKind in MMBIN instruction
    let tm = unsafe { TmKind::from_u8_unchecked(c as u8) };

    // Call metamethod (may change stack/base)
    let result = try_bin_tm(lua_state, v1, v2, tm)?;

    // Reload base after metamethod call as it may have changed
    let current_base = lua_state.get_frame_base(frame_idx);

    // Store result — unchecked
    unsafe {
        *lua_state
            .stack_mut()
            .get_unchecked_mut(current_base + result_reg) = result
    };

    Ok(())
}

/// Handle MMBINI opcode  
/// Call metamethod over R[A] and immediate value sB
///
/// From lvm.c:
/// ```c
/// vmcase(OP_MMBINI) {
///   StkId ra = RA(i);
///   Instruction pi = *(pc - 2);  /* original arith. expression */
///   int imm = GETARG_sB(i);
///   TMS tm = (TMS)GETARG_C(i);
///   int flip = GETARG_k(i);
///   StkId result = RA(pi);
///   Protect(luaT_trybiniTM(L, s2v(ra), imm, flip, result, tm));
///   vmbreak;
/// }
/// ```
pub fn handle_mmbini(
    lua_state: &mut LuaState,
    _base: usize, // Unused, kept for compatibility
    a: usize,     // Operand register
    sb: i32,      // Immediate value
    c: usize,     // Tag method (TMS)
    k: bool,      // flip flag
    pc: usize,
    code: &[Instruction],
    frame_idx: usize, // Frame index for accessing current base
) -> LuaResult<()> {
    // Get the original arithmetic instruction — unchecked
    let pi = unsafe { *code.get_unchecked(pc - 2) };
    let result_reg = pi.get_a() as usize;

    let base = lua_state.get_frame_base(frame_idx);

    // Get operand — unchecked
    let v1 = unsafe { *lua_state.stack_mut().get_unchecked(base + a) };

    // Create integer value for immediate
    let v2 = LuaValue::integer(sb as i64);

    // Get tag method — unchecked
    let tm = unsafe { TmKind::from_u8_unchecked(c as u8) };

    // Call metamethod (flip if needed, may change stack/base)
    let result = if k {
        try_bin_tm(lua_state, v2, v1, tm)?
    } else {
        try_bin_tm(lua_state, v1, v2, tm)?
    };

    // Reload base after metamethod call
    let current_base = lua_state.get_frame_base(frame_idx);

    // Store result — unchecked
    unsafe {
        *lua_state
            .stack_mut()
            .get_unchecked_mut(current_base + result_reg) = result
    };

    Ok(())
}

/// Handle MMBINK opcode
/// Call metamethod over R[A] and K[B]
///
/// From lvm.c:
/// ```c
/// vmcase(OP_MMBINK) {
///   StkId ra = RA(i);
///   Instruction pi = *(pc - 2);  /* original arith. expression */
///   TValue *imm = KB(i);
///   TMS tm = (TMS)GETARG_C(i);
///   int flip = GETARG_k(i);
///   StkId result = RA(pi);
///   Protect(luaT_trybinassocTM(L, s2v(ra), imm, flip, result, tm));
///   vmbreak;
/// }
/// ```
pub fn handle_mmbink(
    lua_state: &mut LuaState,
    _base: usize, // Unused, kept for compatibility
    a: usize,     // Operand register
    b: usize,     // Constant index
    c: usize,     // Tag method (TMS)
    k: bool,      // flip flag
    pc: usize,
    code: &[Instruction],
    constants: &[LuaValue],
    frame_idx: usize, // Frame index for accessing current base
) -> LuaResult<()> {
    // Get the original arithmetic instruction — unchecked
    let pi = unsafe { *code.get_unchecked(pc - 2) };
    let result_reg = pi.get_a() as usize;

    let base = lua_state.get_frame_base(frame_idx);

    // Get operand — unchecked
    let v1 = unsafe { *lua_state.stack_mut().get_unchecked(base + a) };

    // Get constant — unchecked
    let v2 = unsafe { *constants.get_unchecked(b) };

    // Get tag method — unchecked
    let tm = unsafe { TmKind::from_u8_unchecked(c as u8) };

    // Call metamethod (flip if needed, may change stack/base)
    let result = if k {
        try_bin_tm(lua_state, v2, v1, tm)?
    } else {
        try_bin_tm(lua_state, v1, v2, tm)?
    };

    // Reload base after metamethod call
    let current_base = lua_state.get_frame_base(frame_idx);

    // Store result — unchecked
    unsafe {
        *lua_state
            .stack_mut()
            .get_unchecked_mut(current_base + result_reg) = result
    };

    Ok(())
}

/// Try binary metamethod
/// Corresponds to luaT_trybinTM in ltm.c
/// Like Lua 5.5's luaT_trybinTM:
/// ```c
/// void luaT_trybinTM (lua_State *L, const TValue *p1, const TValue *p2,
///                     StkId res, TMS event) {
///   if (l_unlikely(callbinTM(L, p1, p2, res, event) < 0)) {
///     switch (event) {
///       case TM_BAND: case TM_BOR: case TM_BXOR:
///       case TM_SHL: case TM_SHR: case TM_BNOT: {
///         if (ttisnumber(p1) && ttisnumber(p2))
///           luaG_tointerror(L, p1, p2);
///         else
///           luaG_opinterror(L, p1, p2, "perform bitwise operation on");
///       }
///       /* calls never return, but to avoid warnings: *//* FALLTHROUGH */
///       default:
///         luaG_opinterror(L, p1, p2, "perform arithmetic on");
///     }
///   }
/// }
/// ```
fn try_bin_tm(
    lua_state: &mut LuaState,
    p1: LuaValue,
    p2: LuaValue,
    tm_kind: TmKind,
) -> LuaResult<LuaValue> {
    // NOTE: String-to-number coercion is NOT needed here.
    // The original arithmetic/bitwise opcode (OP_ADD, OP_ADDI, etc.) already
    // tried tonumberns/tointeger before falling through to MMBIN/MMBINI/MMBINK.
    // If we reach here, coercion has already failed.

    // Try to get metamethod from p1, then p2
    let metamethod = get_binop_metamethod(lua_state, &p1, &p2, tm_kind);
    if let Some(mm) = metamethod {
        // Call metamethod with (p1, p2) as arguments
        call_tm_res(lua_state, mm, p1, p2)
    } else {
        // No metamethod found, return error
        let msg = match tm_kind {
            TmKind::Band
            | TmKind::Bor
            | TmKind::Bxor
            | TmKind::Shl
            | TmKind::Shr
            | TmKind::Bnot => "attempt to perform bitwise operation on non-number values",
            _ => "attempt to perform arithmetic on non-number values",
        };
        Err(lua_state.error(msg.to_string()))
    }
}

/// Try to convert a LuaValue to integer (for bitwise string coercion)
fn try_to_integer(v: &LuaValue) -> Option<i64> {
    if let Some(i) = v.as_integer() {
        return Some(i);
    }
    if let Some(f) = v.as_number() {
        if f == f.floor() && f.is_finite() {
            // Check f is within i64 range before casting
            if f >= (i64::MIN as f64) && f < (i64::MAX as f64) {
                return Some(f as i64);
            }
        }
        return None;
    }
    if let Some(s) = v.as_str() {
        let parsed = parse_lua_number(s);
        if let Some(i) = parsed.as_integer() {
            return Some(i);
        }
        if let Some(f) = parsed.as_number() {
            if f == f.floor() && f.is_finite() {
                if f >= (i64::MIN as f64) && f < (i64::MAX as f64) {
                    return Some(f as i64);
                }
            }
        }
    }
    None
}

/// Call a metamethod with two arguments
/// Based on Lua 5.5's luaT_callTMres - returns the result value directly
/// Port of Lua 5.5's luaT_callTMres from ltm.c:119
/// ```c
/// lu_byte luaT_callTMres (lua_State *L, const TValue *f, const TValue *p1,
///                         const TValue *p2, StkId res) {
///   ptrdiff_t result = savestack(L, res);
///   StkId func = L->top.p;
///   setobj2s(L, func, f);  /* push function (assume EXTRA_STACK) */
///   setobj2s(L, func + 1, p1);  /* 1st argument */
///   setobj2s(L, func + 2, p2);  /* 2nd argument */
///   L->top.p += 3;
///   /* metamethod may yield only when called from Lua code */
///   if (isLuacode(L->ci))
///     luaD_call(L, func, 1);
///   else
///     luaD_callnoyield(L, func, 1);
///   res = restorestack(L, result);
///   setobjs2s(L, res, --L->top.p);  /* move result to its place */
///   return ttypetag(s2v(res));  /* return tag of the result */
/// }
/// ```
pub fn call_tm_res(
    lua_state: &mut LuaState,
    metamethod: LuaValue,
    arg1: LuaValue,
    arg2: LuaValue,
) -> LuaResult<LuaValue> {
    // Sync top to ci_top — ensures metamethod args are pushed above current frame
    if let Some(frame) = lua_state.current_frame() {
        let ci_top = frame.top;
        if lua_state.get_top() != ci_top {
            lua_state.set_top_raw(ci_top);
        }
    }

    let func_pos = lua_state.get_top();

    // Direct stack write — like Lua 5.5's setobj2s, no bounds checking.
    // The EXTRA_STACK (5 slots) guarantees space above ci->top.
    {
        let stack = lua_state.stack_mut();
        stack[func_pos] = metamethod; // push function
        stack[func_pos + 1] = arg1; // 1st argument
        stack[func_pos + 2] = arg2; // 2nd argument
    }
    // L->top.p += 3
    lua_state.set_top_raw(func_pos + 3);

    // Call the metamethod with nresults=1
    // Check Lua function first (most common for metamethods)
    if metamethod.is_lua_function() {
        // Use specialized push_lua_frame — avoids redundant type dispatch in generic push_frame
        let lua_func = unsafe { metamethod.as_lua_function_unchecked() };
        let chunk = lua_func.chunk();
        let param_count = chunk.param_count;
        let max_stack_size = chunk.max_stack_size as usize;

        let new_base = func_pos + 1;
        let caller_depth = lua_state.call_depth();

        lua_state.push_lua_frame(&metamethod, new_base, 2, 1, param_count, max_stack_size)?;
        lua_state.inc_n_ccalls()?;
        let r = lua_execute(lua_state, caller_depth);
        lua_state.dec_n_ccalls();
        r?;
    } else if metamethod.is_cfunction() {
        call_c_function(lua_state, func_pos, 2, 1)?;
    } else {
        return Err(lua_state.error("attempt to call non-function as metamethod".to_string()));
    }

    // Lua 5.5: setobjs2s(L, res, --L->top.p)
    // Return value is at func_pos after call returns, then L->top.p = func_pos
    let result_val = lua_state.stack_mut()[func_pos];
    lua_state.set_top_raw(func_pos);

    Ok(result_val)
}

/// Port of Lua 5.5's luaT_callTM from ltm.c:103
/// Calls metamethod without expecting a return value
/// ```c
/// void luaT_callTM (lua_State *L, const TValue *f, const TValue *p1,
///                   const TValue *p2, const TValue *p3) {
///   StkId func = L->top.p;
///   setobj2s(L, func, f);  /* push function (assume EXTRA_STACK) */
///   setobj2s(L, func + 1, p1);  /* 1st argument */
///   setobj2s(L, func + 2, p2);  /* 2nd argument */
///   setobj2s(L, func + 3, p3);  /* 3rd argument */
///   L->top.p = func + 4;
///   /* metamethod may yield only when called from Lua code */
///   if (isLuacode(L->ci))
///     luaD_call(L, func, 0);
///   else
///     luaD_callnoyield(L, func, 0);
/// }
/// ```
pub fn call_tm(
    lua_state: &mut LuaState,
    metamethod: LuaValue,
    arg1: LuaValue,
    arg2: LuaValue,
    arg3: LuaValue,
) -> LuaResult<()> {
    // savestate: L->top.p = ci->top.p
    if let Some(frame) = lua_state.current_frame() {
        let ci_top = frame.top;
        if lua_state.get_top() != ci_top {
            lua_state.set_top_raw(ci_top);
        }
    }

    let func_pos = lua_state.get_top();

    // Direct stack write — like Lua 5.5's setobj2s
    {
        let stack = lua_state.stack_mut();
        stack[func_pos] = metamethod; // push function
        stack[func_pos + 1] = arg1; // 1st argument
        stack[func_pos + 2] = arg2; // 2nd argument
        stack[func_pos + 3] = arg3; // 3rd argument
    }
    // L->top.p = func + 4
    lua_state.set_top_raw(func_pos + 4);

    // Call with 0 results (nresults=0)
    if metamethod.is_lua_function() {
        // Use specialized push_lua_frame
        let lua_func = unsafe { metamethod.as_lua_function_unchecked() };
        let chunk = lua_func.chunk();
        let param_count = chunk.param_count;
        let max_stack_size = chunk.max_stack_size as usize;

        let new_base = func_pos + 1;
        let caller_depth = lua_state.call_depth();

        lua_state.push_lua_frame(&metamethod, new_base, 3, 0, param_count, max_stack_size)?;
        lua_state.inc_n_ccalls()?;
        let r = lua_execute(lua_state, caller_depth);
        lua_state.dec_n_ccalls();
        r?;
    } else if metamethod.is_cfunction() {
        call::call_c_function(lua_state, func_pos, 3, 0)?;
    } else {
        return Err(lua_state.error("attempt to call non-function as metamethod".to_string()));
    }

    Ok(())
}

/// Try comparison metamethod (for Lt and Le)
/// Returns Some(bool) if metamethod was called, None if no metamethod
pub fn try_comp_tm(
    lua_state: &mut LuaState,
    p1: LuaValue,
    p2: LuaValue,
    tm_kind: TmKind,
) -> LuaResult<Option<bool>> {
    // Try to get metamethod from p1, then p2
    let metamethod = get_binop_metamethod(lua_state, &p1, &p2, tm_kind);

    if let Some(mm) = metamethod {
        // Call metamethod and convert result to boolean
        let result = call_tm_res(lua_state, mm, p1, p2)?;
        // GC check is already done in luaT_callTMres
        Ok(Some(!result.is_falsy()))
    } else {
        Ok(None)
    }
}

/// Equality comparison - direct port of Lua 5.5's luaV_equalobj
/// Returns true if values are equal, false otherwise
/// Handles metamethods for tables and userdata
pub fn equalobj(lua_state: &mut LuaState, t1: LuaValue, t2: LuaValue) -> LuaResult<bool> {
    // Direct port of lvm.c:582 luaV_equalobj
    if t1 == t2 {
        return Ok(true);
    }

    if t1.tt() != t2.tt() {
        return Ok(false);
    }

    if t1.ttisfulluserdata() {
        // Userdata: first check identity
        if let (Some(u_ptr1), Some(u_ptr2)) = (t1.as_userdata_ptr(), t2.as_userdata_ptr()) {
            if u_ptr1 == u_ptr2 {
                return Ok(true);
            }
        }
        // Different userdata - try __eq metamethod
        let tm = get_binop_metamethod(lua_state, &t1, &t2, TmKind::Eq);

        if let Some(metamethod) = tm {
            let result = call_tm_res(lua_state, metamethod, t1, t2)?;
            return Ok(!result.is_falsy());
        } else {
            return Ok(false);
        }
    }

    if t1.ttistable() {
        // Tables: first check identity
        if let (Some(t_ptr1), Some(t_ptr2)) = (t1.as_table_ptr(), t2.as_table_ptr()) {
            if t_ptr1 == t_ptr2 {
                return Ok(true);
            }
        }
        // Different tables - try __eq metamethod
        let tm = get_binop_metamethod(lua_state, &t1, &t2, TmKind::Eq);
        if let Some(metamethod) = tm {
            let result = call_tm_res(lua_state, metamethod, t1, t2)?;
            return Ok(!result.is_falsy());
        } else {
            return Ok(false);
        }
    }

    if t1.ttiscfunction() {
        // C functions: compare function pointers
        return Ok(unsafe { t1.value.f == t2.value.f });
    }

    // Lua functions, threads, etc.: compare GC pointers
    if let (Some(f_ptr1), Some(f_ptr2)) = (t1.as_function_ptr(), t2.as_function_ptr()) {
        return Ok(f_ptr1 == f_ptr2);
    }

    Ok(false)
}

/// Tag Method types (TMS from ltm.h)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TmKind {
    Index = 0,
    NewIndex = 1,
    Gc = 2,
    Mode = 3,
    Len = 4,
    Eq = 5,
    Add = 6,
    Sub = 7,
    Mul = 8,
    Mod = 9,
    Pow = 10,
    Div = 11,
    IDiv = 12,
    Band = 13,
    Bor = 14,
    Bxor = 15,
    Shl = 16,
    Shr = 17,
    Unm = 18,
    Bnot = 19,
    Lt = 20,
    Le = 21,
    Concat = 22,
    Call = 23,
    Close = 24,
    ToString = 25,
    N = 26, // number of tag methods
}

impl TmKind {
    /// Convert u8 to TmKind
    pub fn from_u8(value: u8) -> Option<Self> {
        if value <= TmKind::ToString as u8 {
            Some(unsafe { Self::from_u8_unchecked(value) })
        } else {
            None
        }
    }

    /// Convert u8 to TmKind without bounds checking.
    /// SAFETY: caller must ensure value <= TmKind::ToString (25)
    #[inline(always)]
    pub unsafe fn from_u8_unchecked(value: u8) -> Self {
        unsafe { std::mem::transmute(value) }
    }

    /// Get the metamethod name
    pub const fn name(self) -> &'static str {
        match self {
            TmKind::Index => "__index",
            TmKind::NewIndex => "__newindex",
            TmKind::Gc => "__gc",
            TmKind::Mode => "__mode",
            TmKind::Len => "__len",
            TmKind::Eq => "__eq",
            TmKind::Add => "__add",
            TmKind::Sub => "__sub",
            TmKind::Mul => "__mul",
            TmKind::Mod => "__mod",
            TmKind::Pow => "__pow",
            TmKind::Div => "__div",
            TmKind::IDiv => "__idiv",
            TmKind::Band => "__band",
            TmKind::Bor => "__bor",
            TmKind::Bxor => "__bxor",
            TmKind::Shl => "__shl",
            TmKind::Shr => "__shr",
            TmKind::Unm => "__unm",
            TmKind::Bnot => "__bnot",
            TmKind::Lt => "__lt",
            TmKind::Le => "__le",
            TmKind::Concat => "__concat",
            TmKind::Call => "__call",
            TmKind::Close => "__close",
            TmKind::ToString => "__tostring",
            TmKind::N => "__n", // Not a real metamethod
        }
    }
}
