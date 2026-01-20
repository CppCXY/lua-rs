use crate::lua_value::LuaValue;
use crate::lua_vm::execute::call::{self, call_c_function};
use crate::lua_vm::execute::helper::get_binop_metamethod;
use crate::lua_vm::execute::lua_execute_until;
use crate::lua_vm::opcode::Instruction;
/// Metamethod operations
///
/// Implements MMBIN, MMBINI, MMBINK opcodes
/// Based on Lua 5.5 ltm.c
use crate::lua_vm::{LuaResult, LuaState, get_metamethod_event};

/// Try unary metamethod (for __unm, __bnot)
/// Port of luaT_trybinTM for unary operations
pub fn try_unary_tm(
    lua_state: &mut LuaState,
    operand: LuaValue,
    result_pos: usize,
    tm_kind: TmKind,
) -> LuaResult<()> {
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
#[inline]
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
    // Get the original arithmetic instruction (pc-2)
    if pc < 2 {
        return Err(lua_state.error("MMBIN: invalid pc".to_string()));
    }

    let pi = code[pc - 2]; // Previous instruction (the original arithmetic op)
    let result_reg = pi.get_a() as usize; // RA(pi) - result register from original instruction

    // CRITICAL: Get base from frame, not parameter (parameter may be stale)
    let base = lua_state.get_frame_base(frame_idx);

    // Get operands
    let v1 = lua_state
        .stack_get(base + a)
        .ok_or_else(|| lua_state.error("MMBIN: operand 1 not found".to_string()))?;
    let v2 = lua_state
        .stack_get(base + b)
        .ok_or_else(|| lua_state.error("MMBIN: operand 2 not found".to_string()))?;

    // Get tag method
    let tm = TmKind::from_u8(c as u8)
        .ok_or_else(|| lua_state.error(format!("MMBIN: invalid tag method {}", c)))?;

    // Call metamethod (may change stack/base)
    let result = try_bin_tm(lua_state, v1, v2, tm)?;

    // CRITICAL: Reload base after metamethod call as it may have changed
    let current_base = lua_state.get_frame_base(frame_idx);

    // Store result
    lua_state.stack_set(current_base + result_reg, result)?;

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
#[inline]
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
    // Get the original arithmetic instruction
    if pc < 2 {
        return Err(lua_state.error("MMBINI: invalid pc".to_string()));
    }

    let pi = code[pc - 2];
    let result_reg = pi.get_a() as usize;

    // CRITICAL: Get base from frame, not parameter
    let base = lua_state.get_frame_base(frame_idx);

    // Get operand
    let v1 = lua_state
        .stack_get(base + a)
        .ok_or_else(|| lua_state.error("MMBINI: operand not found".to_string()))?;

    // Create integer value for immediate
    let v2 = LuaValue::integer(sb as i64);

    // Get tag method
    let tm = TmKind::from_u8(c as u8)
        .ok_or_else(|| lua_state.error(format!("MMBINI: invalid tag method {}", c)))?;

    // Call metamethod (flip if needed, may change stack/base)
    let result = if k {
        // flip: v2 op v1
        try_bin_tm(lua_state, v2, v1, tm)?
    } else {
        // normal: v1 op v2
        try_bin_tm(lua_state, v1, v2, tm)?
    };

    // CRITICAL: Reload base after metamethod call
    let current_base = lua_state.get_frame_base(frame_idx);

    // Store result
    lua_state.stack_set(current_base + result_reg, result)?;

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
#[inline]
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
    // Get the original arithmetic instruction
    if pc < 2 {
        return Err(lua_state.error("MMBINK: invalid pc".to_string()));
    }

    let pi = code[pc - 2];
    let result_reg = pi.get_a() as usize;

    // CRITICAL: Get base from frame, not parameter
    let base = lua_state.get_frame_base(frame_idx);

    // Get operand
    let v1 = lua_state
        .stack_get(base + a)
        .ok_or_else(|| lua_state.error("MMBINK: operand not found".to_string()))?;

    // Get constant
    if b >= constants.len() {
        return Err(lua_state.error(format!("MMBINK: invalid constant index {}", b)));
    }
    let v2 = constants[b];

    // Get tag method
    let tm = TmKind::from_u8(c as u8)
        .ok_or_else(|| lua_state.error(format!("MMBINK: invalid tag method {}", c)))?;

    // Call metamethod (flip if needed, may change stack/base)
    let result = if k {
        // flip: v2 op v1
        try_bin_tm(lua_state, v2, v1, tm)?
    } else {
        // normal: v1 op v2
        try_bin_tm(lua_state, v1, v2, tm)?
    };

    // CRITICAL: Reload base after metamethod call
    let current_base = lua_state.get_frame_base(frame_idx);

    // Store result
    lua_state.stack_set(current_base + result_reg, result)?;

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
    // CRITICAL: Port of Lua 5.5's Protect macro's savestate(L,ci)
    // Before pushing arguments, set L->top.p = ci->top.p
    // This ensures func_pos starts at the correct position
    if let Some(frame) = lua_state.current_frame() {
        lua_state.set_top(frame.top);
    }

    let func_pos = lua_state.get_top();
    // Push function and arguments
    lua_state.push_value(metamethod)?;
    lua_state.push_value(arg1)?;
    lua_state.push_value(arg2)?;

    // Call the metamethod with nresults=1
    if metamethod.is_cfunction() {
        call::call_c_function(lua_state, func_pos, 2, 1)?;
    } else if let Some(func_body) = metamethod.as_lua_function() {
        let is_lua = func_body.is_lua_function();

        if is_lua {
            let new_base = func_pos + 1;
            let caller_depth = lua_state.call_depth();

            lua_state.push_frame(metamethod, new_base, 2, 1)?;
            lua_execute_until(lua_state, caller_depth)?;
        } else {
            call_c_function(lua_state, func_pos, 2, 1)?;
        }
    } else {
        return Err(lua_state.error("attempt to call non-function as metamethod".to_string()));
    }

    // CRITICAL: Lua 5.5's behavior after luaD_call with nresults=1:
    // - Return value is at position 'func' (replaces the function)
    // - L->top.p = func + 1 (points after the return value)
    // - setobjs2s(L, res, --L->top.p) does:
    //   1. Decrement L->top.p to 'func'
    //   2. Copy value from 'func' to 'res'
    // After this, L->top.p = func (back to where it was before push)
    let top = lua_state.get_top();
    let result = if top > func_pos {
        // Get return value (should be at func_pos after call returns)
        let result_val = lua_state.stack_get(func_pos).unwrap_or(LuaValue::nil());
        // Reset top to func_pos (matching Lua 5.5's --L->top.p behavior)
        lua_state.set_top(func_pos);
        result_val
    } else {
        LuaValue::nil()
    };

    Ok(result)
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
    // CRITICAL: Port of Lua 5.5's Protect macro's savestate(L,ci)
    // Before pushing arguments, set L->top.p = ci->top.p
    // This ensures func_pos starts at the correct position
    if let Some(frame) = lua_state.current_frame() {
        lua_state.set_top(frame.top);
    }

    let func_pos = lua_state.get_top();

    // Push function and 3 arguments
    lua_state.push_value(metamethod)?;
    lua_state.push_value(arg1)?;
    lua_state.push_value(arg2)?;
    lua_state.push_value(arg3)?;

    // Call with 0 results (nresults=0)
    if metamethod.is_cfunction() {
        call::call_c_function(lua_state, func_pos, 3, 0)?;
    } else if let Some(func_body) = metamethod.as_lua_function() {
        let is_lua = func_body.is_lua_function();

        if is_lua {
            let new_base = func_pos + 1;
            let caller_depth = lua_state.call_depth();

            lua_state.push_frame(metamethod, new_base, 3, 0)?;
            lua_execute_until(lua_state, caller_depth)?;
        } else {
            call_c_function(lua_state, func_pos, 3, 0)?;
        }
    } else {
        return Err(lua_state.error("attempt to call non-function as metamethod".to_string()));
    }

    // No return value expected (nresults=0)
    // Unlike call_tm_res, we don't need to get any result
    // The call itself has adjusted top appropriately
    // Don't reset top to func_pos as that would destroy the stack!

    lua_state.check_gc()?;
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
        match value {
            0 => Some(TmKind::Index),
            1 => Some(TmKind::NewIndex),
            2 => Some(TmKind::Gc),
            3 => Some(TmKind::Mode),
            4 => Some(TmKind::Len),
            5 => Some(TmKind::Eq),
            6 => Some(TmKind::Add),
            7 => Some(TmKind::Sub),
            8 => Some(TmKind::Mul),
            9 => Some(TmKind::Mod),
            10 => Some(TmKind::Pow),
            11 => Some(TmKind::Div),
            12 => Some(TmKind::IDiv),
            13 => Some(TmKind::Band),
            14 => Some(TmKind::Bor),
            15 => Some(TmKind::Bxor),
            16 => Some(TmKind::Shl),
            17 => Some(TmKind::Shr),
            18 => Some(TmKind::Unm),
            19 => Some(TmKind::Bnot),
            20 => Some(TmKind::Lt),
            21 => Some(TmKind::Le),
            22 => Some(TmKind::Concat),
            23 => Some(TmKind::Call),
            24 => Some(TmKind::Close),
            25 => Some(TmKind::ToString),
            _ => None,
        }
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
