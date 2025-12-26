mod instruction;

pub use instruction::Instruction;

/// Instruction format modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpMode {
    IABC,
    IABx,
    IAsBx,
    IAx,
    IsJ,
}

/// Complete Lua 5.4 Opcode Set (83 opcodes)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpCode {
    // Load/Move operations
    Move = 0,   // R[A] := R[B]
    LoadI,      // R[A] := sBx
    LoadF,      // R[A] := (lua_Number)sBx
    LoadK,      // R[A] := K[Bx]
    LoadKX,     // R[A] := K[extra arg]
    LoadFalse,  // R[A] := false
    LFalseSkip, // R[A] := false; pc++
    LoadTrue,   // R[A] := true
    LoadNil,    // R[A], R[A+1], ..., R[A+B] := nil

    // Upvalue operations
    GetUpval, // R[A] := UpValue[B]
    SetUpval, // UpValue[B] := R[A]

    // Table get operations
    GetTabUp, // R[A] := UpValue[B][K[C]:string]
    GetTable, // R[A] := R[B][R[C]]
    GetI,     // R[A] := R[B][C]
    GetField, // R[A] := R[B][K[C]:string]

    // Table set operations
    SetTabUp, // UpValue[A][K[B]:string] := RK(C)
    SetTable, // R[A][R[B]] := RK(C)
    SetI,     // R[A][B] := RK(C)
    SetField, // R[A][K[B]:string] := RK(C)

    // Table creation
    NewTable, // R[A] := {}

    // Self call
    Self_, // R[A+1] := R[B]; R[A] := R[B][RK(C):string]

    // Arithmetic with immediate/constant
    AddI,  // R[A] := R[B] + sC
    AddK,  // R[A] := R[B] + K[C]:number
    SubK,  // R[A] := R[B] - K[C]:number
    MulK,  // R[A] := R[B] * K[C]:number
    ModK,  // R[A] := R[B] % K[C]:number
    PowK,  // R[A] := R[B] ^ K[C]:number
    DivK,  // R[A] := R[B] / K[C]:number
    IDivK, // R[A] := R[B] // K[C]:number

    // Bitwise with constant
    BAndK, // R[A] := R[B] & K[C]:integer
    BOrK,  // R[A] := R[B] | K[C]:integer
    BXorK, // R[A] := R[B] ~ K[C]:integer

    // Shift operations
    ShrI, // R[A] := R[B] >> sC
    ShlI, // R[A] := sC << R[B]

    // Arithmetic operations (register-register)
    Add,  // R[A] := R[B] + R[C]
    Sub,  // R[A] := R[B] - R[C]
    Mul,  // R[A] := R[B] * R[C]
    Mod,  // R[A] := R[B] % R[C]
    Pow,  // R[A] := R[B] ^ R[C]
    Div,  // R[A] := R[B] / R[C]
    IDiv, // R[A] := R[B] // R[C]

    // Bitwise operations (register-register)
    BAnd, // R[A] := R[B] & R[C]
    BOr,  // R[A] := R[B] | R[C]
    BXor, // R[A] := R[B] ~ R[C]
    Shl,  // R[A] := R[B] << R[C]
    Shr,  // R[A] := R[B] >> R[C]

    // Metamethod binary operations
    MmBin,  // call C metamethod over R[A] and R[B]
    MmBinI, // call C metamethod over R[A] and sB
    MmBinK, // call C metamethod over R[A] and K[B]

    // Unary operations
    Unm,  // R[A] := -R[B]
    BNot, // R[A] := ~R[B]
    Not,  // R[A] := not R[B]
    Len,  // R[A] := #R[B]

    // Concatenation
    Concat, // R[A] := R[A].. ... ..R[A + B - 1]

    // Upvalue management
    Close, // close all upvalues >= R[A]
    Tbc,   // mark variable A "to be closed"

    // Jump
    Jmp, // pc += sJ

    // Comparison operations
    Eq, // if ((R[A] == R[B]) ~= k) then pc++
    Lt, // if ((R[A] <  R[B]) ~= k) then pc++
    Le, // if ((R[A] <= R[B]) ~= k) then pc++

    // Comparison with constant/immediate
    EqK, // if ((R[A] == K[B]) ~= k) then pc++
    EqI, // if ((R[A] == sB) ~= k) then pc++
    LtI, // if ((R[A] < sB) ~= k) then pc++
    LeI, // if ((R[A] <= sB) ~= k) then pc++
    GtI, // if ((R[A] > sB) ~= k) then pc++
    GeI, // if ((R[A] >= sB) ~= k) then pc++

    // Test operations
    Test,    // if (not R[A] == k) then pc++
    TestSet, // if (not R[B] == k) then pc++ else R[A] := R[B]

    // Call operations
    Call,     // R[A], ... ,R[A+C-2] := R[A](R[A+1], ... ,R[A+B-1])
    TailCall, // return R[A](R[A+1], ... ,R[A+B-1])

    // Return operations
    Return,  // return R[A], ... ,R[A+B-2]
    Return0, // return
    Return1, // return R[A]

    // For loops
    ForLoop, // update counters; if loop continues then pc-=Bx;
    ForPrep, // <check values and prepare counters>; if not to run then pc+=Bx+1;

    // Generic for loops
    TForPrep, // create upvalue for R[A + 3]; pc+=Bx
    TForCall, // R[A+4], ... ,R[A+3+C] := R[A](R[A+1], R[A+2])
    TForLoop, // if R[A+2] ~= nil then { R[A]=R[A+2]; pc -= Bx }

    // Table list initialization
    SetList, // R[A][C+i] := R[A+i], 1 <= i <= B

    // Closure creation
    Closure, // R[A] := closure(KPROTO[Bx])

    // Vararg operations
    Vararg,     // R[A], R[A+1], ..., R[A+C-2] = vararg
    GetVarg,    // R[A] := R[B][R[C]], R[B] is vararg parameter (Lua 5.5)
    
    // Error checking for global variables (Lua 5.5)
    ErrNNil, // raise error if R[A] ~= nil (K[Bx - 1] is global name)
    
    VarargPrep, // (adjust vararg parameters)

    // Extra argument
    ExtraArg, // extra (larger) argument for previous opcode
}

impl OpCode {
    #[inline(always)]
    pub fn from_u8(byte: u8) -> Self {
        unsafe { std::mem::transmute(byte) }
    }

    /// Check if instruction uses "top" (IT mode - In Top)
    /// These instructions depend on the value of 'top' from previous instruction
    /// For all other instructions, top should be reset to base + nactvar
    #[inline(always)]
    /// Check if this opcode uses "top" from previous instruction (isIT in Lua 5.4)
    /// These instructions expect top to be set correctly by previous instruction.
    /// For all other instructions, Lua 5.4 resets top = base before execution.
    ///
    /// From Lua 5.4 lopcodes.c:
    /// - CALL: IT=1 (uses top for vararg count)
    /// - TAILCALL: IT=1
    /// - RETURN: IT=1 (uses top for return count)
    /// - SETLIST: IT=1 (uses top for list size)
    /// - VARARGPREP: IT=1 (sets up varargs)
    ///
    /// Note: RETURN0, RETURN1, Vararg, Concat are NOT IT instructions in Lua 5.4!
    pub fn uses_top(self) -> bool {
        use OpCode::*;
        matches!(self, Call | TailCall | Return | SetList | VarargPrep)
    }

    /// Get the instruction format mode for this opcode
    /// Based on Lua 5.4 lopcodes.c luaP_opmodes table
    pub fn get_mode(self) -> OpMode {
        use OpCode::*;
        match self {
            // iAsBx format (signed Bx)
            LoadI | LoadF => OpMode::IAsBx,

            // iABx format (unsigned Bx)
            LoadK | LoadKX | ForLoop | ForPrep | TForPrep | TForLoop | Closure => OpMode::IABx,

            // isJ format (signed jump)
            Jmp => OpMode::IsJ,

            // iAx format
            ExtraArg => OpMode::IAx,

            // iABC format (everything else, including TFORCALL)
            _ => OpMode::IABC,
        }
    }
}
