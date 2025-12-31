mod instruction;

pub use instruction::Instruction;

/// Instruction format modes (Lua 5.5)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpMode {
    IABC,   // iABC:  C(8) | B(8) | k(1) | A(8) | Op(7)
    IvABC,  // ivABC: vC(10) | vB(6) | k(1) | A(8) | Op(7) - variable-size B and C
    IABx,   // iABx:  Bx(17) | A(8) | Op(7)
    IAsBx,  // iAsBx: sBx(signed 17) | A(8) | Op(7)
    IAx,    // iAx:   Ax(25) | Op(7)
    IsJ,    // isJ:   sJ(signed 25) | Op(7)
}

/// Complete Lua 5.5 Opcode Set (86 opcodes)
/// Based on lopcodes.h from Lua 5.5.0
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpCode {
/*----------------------------------------------------------------------
  Lua 5.5 Opcode Definitions (matching lopcodes.h)
  Format: name       args    description
------------------------------------------------------------------------*/
    // Load and move operations
    Move = 0,   // A B      R[A] := R[B]
    LoadI,      // A sBx    R[A] := sBx
    LoadF,      // A sBx    R[A] := (lua_Number)sBx
    LoadK,      // A Bx     R[A] := K[Bx]
    LoadKX,     // A        R[A] := K[extra arg]
    LoadFalse,  // A        R[A] := false
    LFalseSkip, // A        R[A] := false; pc++
    LoadTrue,   // A        R[A] := true
    LoadNil,    // A B      R[A], R[A+1], ..., R[A+B] := nil

    // Upvalue operations
    GetUpval,   // A B      R[A] := UpValue[B]
    SetUpval,   // A B      UpValue[B] := R[A]

    // Table get operations
    GetTabUp,   // A B C    R[A] := UpValue[B][K[C]:shortstring]
    GetTable,   // A B C    R[A] := R[B][R[C]]
    GetI,       // A B C    R[A] := R[B][C]
    GetField,   // A B C    R[A] := R[B][K[C]:shortstring]

    // Table set operations
    SetTabUp,   // A B C    UpValue[A][K[B]:shortstring] := RK(C)
    SetTable,   // A B C    R[A][R[B]] := RK(C)
    SetI,       // A B C    R[A][B] := RK(C)
    SetField,   // A B C    R[A][K[B]:shortstring] := RK(C)

    // Table creation
    NewTable,   // A vB vC k  R[A] := {} (ivABC format)

    // Self call (method call syntax)
    Self_,      // A B C    R[A+1] := R[B]; R[A] := R[B][K[C]:shortstring]

    // Arithmetic with immediate
    AddI,       // A B sC   R[A] := R[B] + sC

    // Arithmetic with constant
    AddK,       // A B C    R[A] := R[B] + K[C]:number
    SubK,       // A B C    R[A] := R[B] - K[C]:number
    MulK,       // A B C    R[A] := R[B] * K[C]:number
    ModK,       // A B C    R[A] := R[B] % K[C]:number
    PowK,       // A B C    R[A] := R[B] ^ K[C]:number
    DivK,       // A B C    R[A] := R[B] / K[C]:number
    IDivK,      // A B C    R[A] := R[B] // K[C]:number

    // Bitwise operations with constant
    BAndK,      // A B C    R[A] := R[B] & K[C]:integer
    BOrK,       // A B C    R[A] := R[B] | K[C]:integer
    BXorK,      // A B C    R[A] := R[B] ~ K[C]:integer

    // Shift operations with immediate
    ShlI,       // A B sC   R[A] := sC << R[B]
    ShrI,       // A B sC   R[A] := R[B] >> sC

    // Arithmetic operations (register-register)
    Add,        // A B C    R[A] := R[B] + R[C]
    Sub,        // A B C    R[A] := R[B] - R[C]
    Mul,        // A B C    R[A] := R[B] * R[C]
    Mod,        // A B C    R[A] := R[B] % R[C]
    Pow,        // A B C    R[A] := R[B] ^ R[C]
    Div,        // A B C    R[A] := R[B] / R[C]
    IDiv,       // A B C    R[A] := R[B] // R[C]

    // Bitwise operations (register-register)
    BAnd,       // A B C    R[A] := R[B] & R[C]
    BOr,        // A B C    R[A] := R[B] | R[C]
    BXor,       // A B C    R[A] := R[B] ~ R[C]
    Shl,        // A B C    R[A] := R[B] << R[C]
    Shr,        // A B C    R[A] := R[B] >> R[C]

    // Metamethod fallback operations
    MmBin,      // A B C    call C metamethod over R[A] and R[B]
    MmBinI,     // A sB C k call C metamethod over R[A] and sB
    MmBinK,     // A B C k  call C metamethod over R[A] and K[B]

    // Unary operations
    Unm,        // A B      R[A] := -R[B]
    BNot,       // A B      R[A] := ~R[B]
    Not,        // A B      R[A] := not R[B]
    Len,        // A B      R[A] := #R[B] (length operator)

    // String concatenation
    Concat,     // A B      R[A] := R[A].. ... ..R[A + B - 1]

    // Upvalue management
    Close,      // A        close all upvalues >= R[A]
    Tbc,        // A        mark variable A "to be closed"

    // Control flow
    Jmp,        // sJ       pc += sJ

    // Comparison operations (register-register)
    Eq,         // A B k    if ((R[A] == R[B]) ~= k) then pc++
    Lt,         // A B k    if ((R[A] <  R[B]) ~= k) then pc++
    Le,         // A B k    if ((R[A] <= R[B]) ~= k) then pc++

    // Comparison with constant/immediate
    EqK,        // A B k    if ((R[A] == K[B]) ~= k) then pc++
    EqI,        // A sB k   if ((R[A] == sB) ~= k) then pc++
    LtI,        // A sB k   if ((R[A] < sB) ~= k) then pc++
    LeI,        // A sB k   if ((R[A] <= sB) ~= k) then pc++
    GtI,        // A sB k   if ((R[A] > sB) ~= k) then pc++
    GeI,        // A sB k   if ((R[A] >= sB) ~= k) then pc++

    // Conditional tests
    Test,       // A k      if (not R[A] == k) then pc++
    TestSet,    // A B k    if (not R[B] == k) then pc++ else R[A] := R[B]

    // Function calls
    Call,       // A B C    R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
    TailCall,   // A B C k  return R[A](R[A+1], ... ,R[A+B-1])

    // Return operations
    Return,     // A B C k  return R[A], ... ,R[A+B-2]
    Return0,    //          return
    Return1,    // A        return R[A]

    // Numeric for loops
    ForLoop,    // A Bx     update counters; if loop continues then pc-=Bx;
    ForPrep,    // A Bx     <check values and prepare counters>; if not to run then pc+=Bx+1;

    // Generic for loops
    TForPrep,   // A Bx     create upvalue for R[A + 3]; pc+=Bx
    TForCall,   // A C      R[A+4], ... ,R[A+3+C] := R[A](R[A+1], R[A+2])
    TForLoop,   // A Bx     if R[A+2] ~= nil then { R[A]=R[A+2]; pc -= Bx }

    // Table list initialization
    SetList,    // A vB vC k R[A][vC+i] := R[A+i], 1 <= i <= vB (ivABC format)

    // Closure creation
    Closure,    // A Bx     R[A] := closure(KPROTO[Bx])

    // Vararg operations
    Vararg,     // A B C k  R[A], ..., R[A+C-2] = varargs
    GetVarg,    // A B C    R[A] := R[B][R[C]], R[B] is vararg parameter (Lua 5.5)

    // Error checking for globals (Lua 5.5)
    ErrNNil,    // A Bx     raise error if R[A] ~= nil (K[Bx - 1] is global name)

    // Vararg preparation
    VarargPrep, //          (adjust varargs)

    // Extra argument for previous instruction
    ExtraArg,   // Ax       extra (larger) argument for previous opcode
}

impl OpCode {
    #[inline(always)]
    pub fn from_u8(byte: u8) -> Self {
        unsafe { std::mem::transmute(byte) }
    }

    /// Check if instruction uses "top" (IT mode - In Top)
    /// These instructions depend on the value of 'top' from previous instruction
    /// For all other instructions, top should be reset to base + nactvar
    ///
    /// From Lua 5.5 lopcodes.c:
    /// - CALL: IT=1 (uses top for vararg count)
    /// - TAILCALL: IT=1
    /// - RETURN: IT=1 (uses top for return count)
    /// - SETLIST: IT=1 (uses top for list size)
    /// - VARARGPREP: IT=1 (sets up varargs)
    pub fn uses_top(self) -> bool {
        use OpCode::*;
        matches!(self, Call | TailCall | Return | SetList | VarargPrep)
    }

    /// Get the instruction format mode for this opcode
    /// Based on Lua 5.5 lopcodes.c luaP_opmodes table
    pub fn get_mode(self) -> OpMode {
        use OpCode::*;
        match self {
            // iAsBx format (signed Bx)
            LoadI | LoadF => OpMode::IAsBx,

            // iABx format (unsigned Bx)
            LoadK | LoadKX | ForLoop | ForPrep | TForPrep | TForLoop | Closure | ErrNNil => OpMode::IABx,

            // isJ format (signed jump)
            Jmp => OpMode::IsJ,

            // iAx format
            ExtraArg => OpMode::IAx,

            // ivABC format (variable-size B and C fields)
            NewTable | SetList => OpMode::IvABC,

            // iABC format (everything else)
            _ => OpMode::IABC,
        }
    }
}
