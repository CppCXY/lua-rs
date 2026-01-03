use crate::lua_value::LuaValue;
use crate::lua_vm::opcode::Instruction;
/// Metamethod operations
///
/// Implements MMBIN, MMBINI, MMBINK opcodes
/// Based on Lua 5.5 ltm.c
use crate::lua_vm::{LuaResult, LuaState};

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
    _p1: LuaValue,
    _p2: LuaValue,
    tm: TmKind,
) -> LuaResult<LuaValue> {
    // TODO: Implement proper metamethod lookup and call
    // For now, just return an error since we don't have metamethod support yet
    // Full implementation needs:
    // 1. Get metatable from p1 or p2 via vm_mut().get_metatable()
    // 2. Lookup metamethod by name in metatable
    // 3. If found, call it with (p1, p2) as arguments
    // 4. Return result

    // No metamethod found, return error
    let msg = match tm {
        TmKind::Band | TmKind::Bor | TmKind::Bxor | TmKind::Shl | TmKind::Shr | TmKind::Bnot => {
            "attempt to perform bitwise operation on non-number values"
        }
        _ => "attempt to perform arithmetic on non-number values",
    };

    Err(lua_state.error(msg.to_string()))
}

// Remove unused helper functions for now
// They will be implemented properly when we have full metamethod support

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
