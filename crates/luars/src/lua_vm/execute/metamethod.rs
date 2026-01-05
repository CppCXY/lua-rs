use crate::lua_value::LuaValue;
use crate::lua_vm::opcode::Instruction;
/// Metamethod operations
///
/// Implements MMBIN, MMBINI, MMBINK opcodes
/// Based on Lua 5.5 ltm.c
use crate::lua_vm::{LuaResult, LuaState, LuaError};
use super::FrameAction;

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
    N = 25, // number of tag methods
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
    base: usize,
    a: usize,             // First operand register
    b: usize,             // Second operand register
    c: usize,             // Tag method (TMS)
    pc: usize,            // Current PC
    code: &[Instruction], // Code array to get previous instruction
) -> LuaResult<()> {
    // Get the original arithmetic instruction (pc-2)
    if pc < 2 {
        return Err(lua_state.error("MMBIN: invalid pc".to_string()));
    }

    let pi = code[pc - 2]; // Previous instruction (the original arithmetic op)
    let result_reg = (pi.as_u32() & 0xFF) as usize; // RA(pi) - result register from original instruction

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

    // Call metamethod
    let result = try_bin_tm(lua_state, v1, v2, tm)?;

    // Store result
    lua_state.stack_set(base + result_reg, result)?;

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
    base: usize,
    a: usize, // Operand register
    sb: i32,  // Immediate value
    c: usize, // Tag method (TMS)
    k: bool,  // flip flag
    pc: usize,
    code: &[Instruction],
) -> LuaResult<()> {
    // Get the original arithmetic instruction
    if pc < 2 {
        return Err(lua_state.error("MMBINI: invalid pc".to_string()));
    }

    let pi = code[pc - 2];
    let result_reg = (pi.as_u32() & 0xFF) as usize;

    // Get operand
    let v1 = lua_state
        .stack_get(base + a)
        .ok_or_else(|| lua_state.error("MMBINI: operand not found".to_string()))?;

    // Create integer value for immediate
    let v2 = LuaValue::integer(sb as i64);

    // Get tag method
    let tm = TmKind::from_u8(c as u8)
        .ok_or_else(|| lua_state.error(format!("MMBINI: invalid tag method {}", c)))?;

    // Call metamethod (flip if needed)
    let result = if k {
        // flip: v2 op v1
        try_bin_tm(lua_state, v2, v1, tm)?
    } else {
        // normal: v1 op v2
        try_bin_tm(lua_state, v1, v2, tm)?
    };

    // Store result
    lua_state.stack_set(base + result_reg, result)?;

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
    base: usize,
    a: usize, // Operand register
    b: usize, // Constant index
    c: usize, // Tag method (TMS)
    k: bool,  // flip flag
    pc: usize,
    code: &[Instruction],
    constants: &[LuaValue],
) -> LuaResult<()> {
    // Get the original arithmetic instruction
    if pc < 2 {
        return Err(lua_state.error("MMBINK: invalid pc".to_string()));
    }

    let pi = code[pc - 2];
    let result_reg = (pi.as_u32() & 0xFF) as usize;

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

    // Call metamethod (flip if needed)
    let result = if k {
        // flip: v2 op v1
        try_bin_tm(lua_state, v2, v1, tm)?
    } else {
        // normal: v1 op v2
        try_bin_tm(lua_state, v1, v2, tm)?
    };

    // Store result
    lua_state.stack_set(base + result_reg, result)?;

    Ok(())
}

/// Try binary metamethod
/// Corresponds to luaT_trybinTM in ltm.c
fn try_bin_tm(
    lua_state: &mut LuaState,
    p1: LuaValue,
    p2: LuaValue,
    tm: TmKind,
) -> LuaResult<LuaValue> {
    let tm_name = tm.name();
    
    // Try to get metamethod from p1, then p2
    let metamethod = get_binop_metamethod(lua_state, &p1, tm_name)
        .or_else(|| get_binop_metamethod(lua_state, &p2, tm_name));
    
    if let Some(mm) = metamethod {
        // Call metamethod with (p1, p2) as arguments
        call_metamethod(lua_state, mm, p1, p2)
    } else {
        // No metamethod found, return error
        let msg = match tm {
            TmKind::Band | TmKind::Bor | TmKind::Bxor | TmKind::Shl | TmKind::Shr | TmKind::Bnot => {
                "attempt to perform bitwise operation on non-number values"
            }
            _ => "attempt to perform arithmetic on non-number values",
        };
        Err(lua_state.error(msg.to_string()))
    }
}

/// Get binary operation metamethod from a value
fn get_binop_metamethod(lua_state: &mut LuaState, value: &LuaValue, tm_name: &str) -> Option<LuaValue> {
    // Get metatable based on value type
    let metatable = get_metatable(lua_state, value)?;
    
    // Look up the metamethod in the metatable
    // CRITICAL: Use raw access to avoid triggering __index metamethod
    // This matches Lua 5.5's luaH_Hgetshortstr which is a raw access
    if let Some(_table_id) = metatable.as_table_id() {
        let vm = lua_state.vm_mut();
        let key = vm.create_string(tm_name);
        vm.table_get_raw(&metatable, &key) // Use RAW access!
    } else {
        None
    }
}

/// Get metatable for a value
fn get_metatable(lua_state: &mut LuaState, value: &LuaValue) -> Option<LuaValue> {
    let vm = lua_state.vm_mut();
    
    match value.kind() {
        crate::lua_value::LuaValueKind::Table => {
            if let Some(table_id) = value.as_table_id() {
                vm.object_pool.get_table(table_id)
                    .and_then(|t| t.get_metatable())
            } else {
                None
            }
        }
        crate::lua_value::LuaValueKind::Userdata => {
            if let Some(ud_id) = value.as_userdata_id() {
                vm.object_pool.get_userdata(ud_id)
                    .map(|ud| ud.get_metatable())
            } else {
                None
            }
        }
        crate::lua_value::LuaValueKind::String => {
            // Strings share a global metatable
            vm.string_mt
        }
        _ => None,
    }
}

/// Call a metamethod with two arguments
/// Based on Lua 5.5's luaT_callTMres - returns the result value directly
fn call_metamethod(
    lua_state: &mut LuaState,
    metamethod: LuaValue,
    arg1: LuaValue,
    arg2: LuaValue,
) -> LuaResult<LuaValue> {
    // Like Lua's luaT_callTMres:
    // 1. Save result position offset  
    // 2. Push function and args at top.p
    // 3. Call with luaD_call(L, func, 1) - nresults=1
    // 4. Move result from top-1 to result position
    
    // **Critical**: We need to push at top and let the call mechanism
    // handle everything. After call, result will be at the function position.
    
    let func_pos = lua_state.get_top();
    
    // Push function and arguments
    lua_state.push_value(metamethod)?;
    lua_state.push_value(arg1)?;
    lua_state.push_value(arg2)?;
    
    // Now call: luaD_call pushes a frame, executes, pops frame, and places result at func_pos
    // Our call_c_function does exactly this for C functions
    // For Lua functions, we need similar treatment
    
    if metamethod.is_cfunction() {
        // C function: call_c_function handles everything
        crate::lua_vm::execute::call::call_c_function(lua_state, func_pos, 2, 1)?;
    } else if let Some(func_id) = metamethod.as_function_id() {
        let is_lua = {
            let func_obj = lua_state.vm_mut().object_pool.get_function(func_id)
                .ok_or(LuaError::RuntimeError)?;
            func_obj.is_lua_function()
        };
        
        if is_lua {
            // Lua function: push frame and execute
            let new_base = func_pos + 1;
            lua_state.push_frame(metamethod, new_base, 2, 1)?;
            
            // Execute the new frame
            crate::lua_vm::execute::lua_execute(lua_state)?;
        } else {
            // GC C function
            crate::lua_vm::execute::call::call_c_function(lua_state, func_pos, 2, 1)?;
        }
    } else {
        return Err(lua_state.error("attempt to call non-function as metamethod".to_string()));
    }
    
    // Get result (should be at func_pos after call completed)
    let result = lua_state.stack_get(func_pos)
        .unwrap_or(LuaValue::nil());
    
    // Don't modify stack - caller will handle cleanup
    // Actually, we should restore stack as if the call never happened for the caller
    // But the result needs to stay somewhere... 
    // In Lua's case, the caller handles the result position
    
    // For now, leave result at func_pos and restore top
    // NO! We need to return the value, so just get it and don't change stack
    // Actually in our API design, we return LuaValue, so we can just return it
    // and let the caller decide what to do with stack
    
    Ok(result)
}

/// Try comparison metamethod (for Lt and Le)
/// Returns Some(bool) if metamethod was called, None if no metamethod
pub fn try_comp_tm(
    lua_state: &mut LuaState,
    p1: LuaValue,
    p2: LuaValue,
    tm: TmKind,
) -> LuaResult<Option<bool>> {
    let tm_name = tm.name();
    
    // Try to get metamethod from p1, then p2
    let metamethod = get_binop_metamethod(lua_state, &p1, tm_name)
        .or_else(|| get_binop_metamethod(lua_state, &p2, tm_name));
    
    if let Some(mm) = metamethod {
        // Call metamethod and convert result to boolean
        let result = call_metamethod(lua_state, mm, p1, p2)?;
        Ok(Some(!result.is_falsy()))
    } else {
        Ok(None)
    }
}

/// Try equality metamethod
/// Only calls metamethod if both values have the same metatable or both are userdata
/// Returns Some(bool) if metamethod was called, None if no metamethod
pub fn try_eq_tm(
    lua_state: &mut LuaState,
    p1: LuaValue,
    p2: LuaValue,
) -> LuaResult<Option<bool>> {
    // In Lua 5.5, __eq is only called when:
    // 1. Both operands have the same metatable, OR
    // 2. At least one is userdata and they both have __eq metamethods
    
    let mt1 = get_metatable(lua_state, &p1);
    let mt2 = get_metatable(lua_state, &p2);
    
    // Check if they have the same metatable
    let same_mt = match (mt1, mt2) {
        (Some(m1), Some(m2)) => {
            // Compare metatable identity
            if let (Some(t1), Some(t2)) = (m1.as_table_id(), m2.as_table_id()) {
                t1 == t2
            } else {
                false
            }
        }
        _ => false,
    };
    
    if !same_mt {
        return Ok(None);
    }
    
    // Now try to get __eq metamethod
    if let Some(metamethod) = get_binop_metamethod(lua_state, &p1, "__eq") {
        let result = call_metamethod(lua_state, metamethod, p1, p2)?;
        Ok(Some(!result.is_falsy()))
    } else {
        Ok(None)
    }
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
            TmKind::N => "__n", // Not a real metamethod
        }
    }
}
