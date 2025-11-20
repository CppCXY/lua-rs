/*----------------------------------------------------------------------
  Lua 5.4 Opcode System - Complete 1:1 Port from lopcodes.h

  Instruction Format (32-bit):
  - iABC:  [Op(7) | A(8) | k(1) | B(8) | C(8)]
  - iABx:  [Op(7) | A(8) | Bx(17)]
  - iAsBx: [Op(7) | A(8) | sBx(signed 17)]
  - iAx:   [Op(7) | Ax(25)]
  - isJ:   [Op(7) | sJ(signed 25)]
----------------------------------------------------------------------*/

pub mod dispatcher;

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
    ForLoop, // update counters; if loop continues then pc-=Bx
    ForPrep, // <check values and prepare counters>; if not to run then pc+=Bx+1

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
    VarargPrep, // (adjust vararg parameters)

    // Extra argument
    ExtraArg, // extra (larger) argument for previous opcode
}

impl OpCode {
    pub fn from_u8(byte: u8) -> Option<Self> {
        if byte <= OpCode::ExtraArg as u8 {
            Some(unsafe { std::mem::transmute(byte) })
        } else {
            None
        }
    }

    /// Get the instruction format mode for this opcode
    pub fn get_mode(self) -> OpMode {
        use OpCode::*;
        match self {
            LoadI | LoadF | LoadK | LoadKX | Jmp | ForLoop | ForPrep | TForPrep | TForCall
            | TForLoop | Closure => OpMode::IABx,

            ExtraArg => OpMode::IAx,

            _ => OpMode::IABC,
        }
    }
}

/// Lua 5.4 Instruction encoding/decoding
///
/// Bit layout (32-bit instruction):
/// - iABC:  [31..25: C(8)] [24..17: B(8)] [16: k(1)] [15..8: A(8)] [6..0: Op(7)]
/// - iABx:  [31..15: Bx(17)] [14..7: A(8)] [6..0: Op(7)]
/// - iAsBx: [31..15: sBx(17)] [14..7: A(8)] [6..0: Op(7)]
/// - iAx:   [31..7: Ax(25)] [6..0: Op(7)]
/// - isJ:   [31..7: sJ(25)] [6..0: Op(7)]  -- Note: sJ format has NO k bit!
pub struct Instruction;

impl Instruction {
    // Size of each field
    pub const SIZE_OP: u32 = 7;
    pub const SIZE_A: u32 = 8;
    pub const SIZE_B: u32 = 8;
    pub const SIZE_C: u32 = 8;
    pub const SIZE_K: u32 = 1;
    pub const SIZE_BX: u32 = Self::SIZE_C + Self::SIZE_B + Self::SIZE_K; // 17
    pub const SIZE_AX: u32 = Self::SIZE_BX + Self::SIZE_A; // 25
    pub const SIZE_SJ: u32 = Self::SIZE_BX + Self::SIZE_A; // 25

    // Position of each field
    pub const POS_OP: u32 = 0;
    pub const POS_A: u32 = Self::POS_OP + Self::SIZE_OP;
    pub const POS_K: u32 = Self::POS_A + Self::SIZE_A;
    pub const POS_B: u32 = Self::POS_K + Self::SIZE_K;
    pub const POS_C: u32 = Self::POS_B + Self::SIZE_B;
    pub const POS_BX: u32 = Self::POS_K;
    pub const POS_AX: u32 = Self::POS_A;
    pub const POS_SJ: u32 = Self::POS_A;

    // Maximum values
    pub const MAX_A: u32 = (1 << Self::SIZE_A) - 1;
    pub const MAX_B: u32 = (1 << Self::SIZE_B) - 1;
    pub const MAX_C: u32 = (1 << Self::SIZE_C) - 1;
    pub const MAX_BX: u32 = (1 << Self::SIZE_BX) - 1;
    pub const MAX_AX: u32 = (1 << Self::SIZE_AX) - 1;
    pub const MAX_SJ: u32 = (1 << Self::SIZE_SJ) - 1;

    // Offsets for signed arguments
    pub const OFFSET_SBX: i32 = (Self::MAX_BX >> 1) as i32;
    pub const OFFSET_SJ: i32 = (Self::MAX_SJ >> 1) as i32;
    pub const OFFSET_SC: i32 = (Self::MAX_C >> 1) as i32;

    // Create masks
    #[inline(always)]
    fn mask1(n: u32, p: u32) -> u32 {
        (!(!0u32 << n)) << p
    }

    #[inline(always)]
    fn mask0(n: u32, p: u32) -> u32 {
        !Self::mask1(n, p)
    }

    // Get/Set opcode
    #[inline(always)]
    pub fn get_opcode(i: u32) -> OpCode {
        let op_byte = ((i >> Self::POS_OP) & Self::mask1(Self::SIZE_OP, 0)) as u8;
        OpCode::from_u8(op_byte).expect("Invalid opcode")
    }

    #[inline(always)]
    pub fn set_opcode(i: &mut u32, op: OpCode) {
        *i = (*i & Self::mask0(Self::SIZE_OP, Self::POS_OP))
            | ((op as u32) << Self::POS_OP & Self::mask1(Self::SIZE_OP, Self::POS_OP));
    }

    // Generic argument getter
    #[inline(always)]
    fn get_arg(i: u32, pos: u32, size: u32) -> u32 {
        (i >> pos) & Self::mask1(size, 0)
    }

    // Generic argument setter
    #[inline(always)]
    fn set_arg(i: &mut u32, v: u32, pos: u32, size: u32) {
        *i = (*i & Self::mask0(size, pos)) | ((v << pos) & Self::mask1(size, pos));
    }

    // Field accessors
    #[inline(always)]
    pub fn get_a(i: u32) -> u32 {
        Self::get_arg(i, Self::POS_A, Self::SIZE_A)
    }

    #[inline(always)]
    pub fn set_a(i: &mut u32, v: u32) {
        Self::set_arg(i, v, Self::POS_A, Self::SIZE_A);
    }

    #[inline(always)]
    pub fn get_b(i: u32) -> u32 {
        Self::get_arg(i, Self::POS_B, Self::SIZE_B)
    }

    #[inline(always)]
    pub fn get_sb(i: u32) -> i32 {
        Self::get_b(i) as i32 - Self::OFFSET_SC
    }

    #[inline(always)]
    pub fn set_b(i: &mut u32, v: u32) {
        Self::set_arg(i, v, Self::POS_B, Self::SIZE_B);
    }

    #[inline(always)]
    pub fn get_c(i: u32) -> u32 {
        Self::get_arg(i, Self::POS_C, Self::SIZE_C)
    }

    #[inline(always)]
    pub fn get_sc(i: u32) -> i32 {
        Self::get_c(i) as i32 - Self::OFFSET_SC
    }

    #[inline(always)]
    pub fn set_c(i: &mut u32, v: u32) {
        Self::set_arg(i, v, Self::POS_C, Self::SIZE_C);
    }

    #[inline(always)]
    pub fn get_k(i: u32) -> bool {
        Self::get_arg(i, Self::POS_K, Self::SIZE_K) != 0
    }

    #[inline(always)]
    pub fn set_k(i: &mut u32, v: bool) {
        Self::set_arg(i, if v { 1 } else { 0 }, Self::POS_K, Self::SIZE_K);
    }

    #[inline(always)]
    pub fn get_bx(i: u32) -> u32 {
        Self::get_arg(i, Self::POS_BX, Self::SIZE_BX)
    }

    #[inline(always)]
    pub fn get_sbx(i: u32) -> i32 {
        Self::get_bx(i) as i32 - Self::OFFSET_SBX
    }

    #[inline(always)]
    pub fn set_bx(i: &mut u32, v: u32) {
        Self::set_arg(i, v, Self::POS_BX, Self::SIZE_BX);
    }

    #[inline(always)]
    pub fn get_ax(i: u32) -> u32 {
        Self::get_arg(i, Self::POS_AX, Self::SIZE_AX)
    }

    #[inline(always)]
    pub fn set_ax(i: &mut u32, v: u32) {
        Self::set_arg(i, v, Self::POS_AX, Self::SIZE_AX);
    }

    #[inline(always)]
    pub fn get_sj(i: u32) -> i32 {
        Self::get_arg(i, Self::POS_SJ, Self::SIZE_SJ) as i32 - Self::OFFSET_SJ
    }

    #[inline(always)]
    pub fn set_sj(i: &mut u32, v: i32) {
        Self::set_arg(i, (v + Self::OFFSET_SJ) as u32, Self::POS_SJ, Self::SIZE_SJ);
    }

    // Instruction creation
    pub fn create_abc(op: OpCode, a: u32, b: u32, c: u32) -> u32 {
        ((op as u32) << Self::POS_OP) | (a << Self::POS_A) | (b << Self::POS_B) | (c << Self::POS_C)
    }

    pub fn create_abck(op: OpCode, a: u32, b: u32, c: u32, k: bool) -> u32 {
        ((op as u32) << Self::POS_OP)
            | (a << Self::POS_A)
            | (if k { 1 } else { 0 } << Self::POS_K)
            | (b << Self::POS_B)
            | (c << Self::POS_C)
    }

    pub fn create_abx(op: OpCode, a: u32, bx: u32) -> u32 {
        ((op as u32) << Self::POS_OP) | (a << Self::POS_A) | (bx << Self::POS_BX)
    }

    pub fn create_asbx(op: OpCode, a: u32, sbx: i32) -> u32 {
        Self::create_abx(op, a, (sbx + Self::OFFSET_SBX) as u32)
    }

    pub fn create_ax(op: OpCode, ax: u32) -> u32 {
        ((op as u32) << Self::POS_OP) | (ax << Self::POS_AX)
    }

    pub fn create_sj(op: OpCode, sj: i32) -> u32 {
        ((op as u32) << Self::POS_OP) | (((sj + Self::OFFSET_SJ) as u32) << Self::POS_SJ)
    }

    // Helper: RK(x) - if k then K[x] else R[x]
    #[inline(always)]
    pub fn is_k(x: u32) -> bool {
        x & (1 << (Self::SIZE_B - 1)) != 0
    }

    #[inline(always)]
    pub fn rk_index(x: u32) -> u32 {
        x & !(1 << (Self::SIZE_B - 1))
    }

    // Convenience aliases
    #[inline(always)]
    pub fn encode_abc(op: OpCode, a: u32, b: u32, c: u32) -> u32 {
        Self::create_abc(op, a, b, c)
    }

    #[inline(always)]
    pub fn encode_abx(op: OpCode, a: u32, bx: u32) -> u32 {
        Self::create_abx(op, a, bx)
    }

    #[inline(always)]
    pub fn encode_asbx(op: OpCode, a: u32, sbx: i32) -> u32 {
        Self::create_asbx(op, a, sbx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instruction_abc() {
        let instr = Instruction::create_abc(OpCode::Move, 1, 2, 3);
        assert_eq!(Instruction::get_opcode(instr), OpCode::Move);
        assert_eq!(Instruction::get_a(instr), 1);
        assert_eq!(Instruction::get_b(instr), 2);
        assert_eq!(Instruction::get_c(instr), 3);
    }

    #[test]
    fn test_instruction_abck() {
        let instr = Instruction::create_abck(OpCode::Add, 5, 10, 20, true);
        assert_eq!(Instruction::get_opcode(instr), OpCode::Add);
        assert_eq!(Instruction::get_a(instr), 5);
        assert_eq!(Instruction::get_b(instr), 10);
        assert_eq!(Instruction::get_c(instr), 20);
        assert_eq!(Instruction::get_k(instr), true);
    }

    #[test]
    fn test_instruction_abx() {
        let instr = Instruction::create_abx(OpCode::LoadK, 3, 100);
        assert_eq!(Instruction::get_opcode(instr), OpCode::LoadK);
        assert_eq!(Instruction::get_a(instr), 3);
        assert_eq!(Instruction::get_bx(instr), 100);
    }

    #[test]
    fn test_instruction_asbx() {
        let instr = Instruction::create_asbx(OpCode::ForLoop, 2, -50);
        assert_eq!(Instruction::get_opcode(instr), OpCode::ForLoop);
        assert_eq!(Instruction::get_a(instr), 2);
        assert_eq!(Instruction::get_sbx(instr), -50);
    }

    #[test]
    fn test_instruction_ax() {
        let instr = Instruction::create_ax(OpCode::ExtraArg, 0xFFFFFF);
        assert_eq!(Instruction::get_opcode(instr), OpCode::ExtraArg);
        assert_eq!(Instruction::get_ax(instr), 0xFFFFFF);
    }

    #[test]
    fn test_instruction_sj() {
        let instr = Instruction::create_sj(OpCode::Jmp, 1000);
        assert_eq!(Instruction::get_opcode(instr), OpCode::Jmp);
        assert_eq!(Instruction::get_sj(instr), 1000);
    }

    #[test]
    fn test_instruction_boundaries() {
        // Test maximum values
        let max_a = Instruction::MAX_A;
        let max_b = Instruction::MAX_B;
        let max_c = Instruction::MAX_C;

        let instr = Instruction::create_abc(OpCode::Move, max_a, max_b, max_c);
        assert_eq!(Instruction::get_a(instr), max_a);
        assert_eq!(Instruction::get_b(instr), max_b);
        assert_eq!(Instruction::get_c(instr), max_c);
    }

    #[test]
    fn test_opcode_mode() {
        assert_eq!(OpCode::Move.get_mode(), OpMode::IABC);
        assert_eq!(OpCode::LoadK.get_mode(), OpMode::IABx);
        assert_eq!(OpCode::Jmp.get_mode(), OpMode::IABx);
        assert_eq!(OpCode::ExtraArg.get_mode(), OpMode::IAx);
        assert_eq!(OpCode::Add.get_mode(), OpMode::IABC);
    }

    #[test]
    fn test_set_fields() {
        let mut instr = Instruction::create_abc(OpCode::Move, 1, 2, 3);

        Instruction::set_a(&mut instr, 10);
        assert_eq!(Instruction::get_a(instr), 10);
        assert_eq!(Instruction::get_b(instr), 2);
        assert_eq!(Instruction::get_c(instr), 3);

        Instruction::set_b(&mut instr, 20);
        assert_eq!(Instruction::get_b(instr), 20);

        Instruction::set_c(&mut instr, 30);
        assert_eq!(Instruction::get_c(instr), 30);

        assert_eq!(Instruction::get_opcode(instr), OpCode::Move);
    }

    #[test]
    fn test_signed_arguments() {
        // Test sBx (signed Bx)
        let instr_neg = Instruction::create_asbx(OpCode::ForLoop, 0, -100);
        assert_eq!(Instruction::get_sbx(instr_neg), -100);

        let instr_pos = Instruction::create_asbx(OpCode::ForLoop, 0, 100);
        assert_eq!(Instruction::get_sbx(instr_pos), 100);

        // Test sJ (signed jump)
        let jmp_neg = Instruction::create_sj(OpCode::Jmp, -500);
        assert_eq!(Instruction::get_sj(jmp_neg), -500);

        let jmp_pos = Instruction::create_sj(OpCode::Jmp, 500);
        assert_eq!(Instruction::get_sj(jmp_pos), 500);
    }
}
