/*----------------------------------------------------------------------
  Lua 5.5 Opcode System - Complete 1:1 Port from lopcodes.h

  Instruction Format (32-bit):
  All instructions have an opcode in the first 7 bits.

        3 3 2 2 2 2 2 2 2 2 2 2 1 1 1 1 1 1 1 1 1 1 0 0 0 0 0 0 0 0 0 0
        1 0 9 8 7 6 5 4 3 2 1 0 9 8 7 6 5 4 3 2 1 0 9 8 7 6 5 4 3 2 1 0
  iABC          C(8)     |      B(8)     |k|     A(8)      |   Op(7)     |
  ivABC         vC(10)     |     vB(6)   |k|     A(8)      |   Op(7)     |
  iABx                Bx(17)               |     A(8)      |   Op(7)     |
  iAsBx              sBx (signed)(17)      |     A(8)      |   Op(7)     |
  iAx                           Ax(25)                     |   Op(7)     |
  isJ                           sJ (signed)(25)            |   Op(7)     |

  ('v' stands for "variant", 's' for "signed", 'x' for "extended".)
  A signed argument is represented in excess K: The represented value is
  the written unsigned value minus K, where K is half (rounded down) the
  maximum value for the corresponding unsigned argument.
----------------------------------------------------------------------*/

// ============ Instruction Decoding Macros ============
// These macros are guaranteed to be inlined and produce optimal code.
// Use these instead of Instruction::get_* functions in hot paths.

use crate::OpCode;

/// Zero-cost abstraction for Lua 5.5 instruction encoding
/// Internally stores a 32-bit instruction following Lua 5.5 format
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction(u32);

impl Instruction {
    /// Create an Instruction from a raw u32 value
    #[inline(always)]
    pub const fn from_u32(value: u32) -> Self {
        Self(value)
    }

    /// Get the raw u32 value of this instruction
    #[inline(always)]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    // Size of each field
    pub const SIZE_OP: u32 = 7;
    pub const SIZE_A: u32 = 8;
    pub const SIZE_B: u32 = 8;
    pub const SIZE_C: u32 = 8;
    pub const SIZE_K: u32 = 1;
    // vABC format fields (variable-size B and C) - note: lowercase 'v' per Lua 5.5
    pub const SIZE_V_B: u32 = 6;
    pub const SIZE_V_C: u32 = 10;
    pub const SIZE_BX: u32 = Self::SIZE_C + Self::SIZE_B + Self::SIZE_K; // 17
    pub const SIZE_AX: u32 = Self::SIZE_BX + Self::SIZE_A; // 25
    pub const SIZE_SJ: u32 = Self::SIZE_BX + Self::SIZE_A; // 25

    // Position of each field
    pub const POS_OP: u32 = 0;
    pub const POS_A: u32 = Self::POS_OP + Self::SIZE_OP;
    pub const POS_K: u32 = Self::POS_A + Self::SIZE_A;
    pub const POS_B: u32 = Self::POS_K + Self::SIZE_K;
    pub const POS_C: u32 = Self::POS_B + Self::SIZE_B;
    // vABC format positions - note: lowercase 'v' per Lua 5.5
    pub const POS_V_B: u32 = Self::POS_K + Self::SIZE_K;
    pub const POS_V_C: u32 = Self::POS_V_B + Self::SIZE_V_B;
    pub const POS_BX: u32 = Self::POS_K;
    pub const POS_AX: u32 = Self::POS_A;
    pub const POS_SJ: u32 = Self::POS_A;

    // Maximum values
    pub const MAX_A: u32 = (1 << Self::SIZE_A) - 1;
    pub const MAX_B: u32 = (1 << Self::SIZE_B) - 1;
    pub const MAX_C: u32 = (1 << Self::SIZE_C) - 1;
    pub const MAX_V_B: u32 = (1 << Self::SIZE_V_B) - 1;
    pub const MAX_V_C: u32 = (1 << Self::SIZE_V_C) - 1;
    pub const MAX_BX: u32 = (1 << Self::SIZE_BX) - 1;
    pub const MAX_AX: u32 = (1 << Self::SIZE_AX) - 1;
    pub const MAX_SJ: u32 = (1 << Self::SIZE_SJ) - 1;

    // Offsets for signed arguments (Lua 5.5: OFFSET_X = MAXARG_X >> 1)
    pub const OFFSET_SB: i32 = (Self::MAX_C >> 1) as i32; // sB uses OFFSET_sC (127 for 8-bit)
    pub const OFFSET_SC: i32 = (Self::MAX_C >> 1) as i32; // 127 for 8-bit C
    pub const OFFSET_SBX: i32 = (Self::MAX_BX >> 1) as i32;
    pub const OFFSET_SJ: i32 = (Self::MAX_SJ >> 1) as i32;

    // Create masks
    #[inline(always)]
    fn mask1(n: u32, p: u32) -> u32 {
        unsafe { (!((!0u32).unchecked_shl(n))).unchecked_shl(p) }
    }

    #[inline(always)]
    fn mask0(n: u32, p: u32) -> u32 {
        !Self::mask1(n, p)
    }

    // Get/Set opcode
    #[inline(always)]
    pub fn get_opcode(self) -> OpCode {
        let op_byte = ((self.0 >> Self::POS_OP) & Self::mask1(Self::SIZE_OP, 0)) as u8;
        OpCode::from_u8(op_byte)
    }

    #[inline(always)]
    pub fn set_opcode(&mut self, op: OpCode) {
        unsafe {
            self.0 = (self.0 & Self::mask0(Self::SIZE_OP, Self::POS_OP))
                | ((op as u32).unchecked_shl(Self::POS_OP)
                    & Self::mask1(Self::SIZE_OP, Self::POS_OP));
        }
    }

    // Generic argument getter
    #[inline(always)]
    fn get_arg(&self, pos: u32, size: u32) -> u32 {
        unsafe { self.0.unchecked_shr(pos) & Self::mask1(size, 0) }
    }

    // Generic argument setter
    #[inline(always)]
    fn set_arg(&mut self, v: u32, pos: u32, size: u32) {
        unsafe {
            self.0 =
                (self.0 & Self::mask0(size, pos)) | (v.unchecked_shl(pos) & Self::mask1(size, pos));
        }
    }

    // Field accessors
    #[inline(always)]
    pub fn get_a(self) -> u32 {
        self.get_arg(Self::POS_A, Self::SIZE_A)
    }

    #[inline(always)]
    pub fn set_a(&mut self, v: u32) {
        self.set_arg(v, Self::POS_A, Self::SIZE_A);
    }

    #[inline(always)]
    pub fn get_b(self) -> u32 {
        self.get_arg(Self::POS_B, Self::SIZE_B)
    }

    #[inline(always)]
    pub fn get_sb(self) -> i32 {
        self.get_b() as i32 - Self::OFFSET_SB
    }

    #[inline(always)]
    pub fn set_b(&mut self, v: u32) {
        self.set_arg(v, Self::POS_B, Self::SIZE_B);
    }

    #[inline(always)]
    pub fn get_c(self) -> u32 {
        self.get_arg(Self::POS_C, Self::SIZE_C)
    }

    #[inline(always)]
    pub fn get_sc(self) -> i32 {
        self.get_c() as i32 - Self::OFFSET_SC
    }

    #[inline(always)]
    pub fn set_c(&mut self, v: u32) {
        self.set_arg(v, Self::POS_C, Self::SIZE_C);
    }

    #[inline(always)]
    pub fn get_k(self) -> bool {
        self.get_arg(Self::POS_K, Self::SIZE_K) != 0
    }

    #[inline(always)]
    pub fn set_k(&mut self, v: bool) {
        self.set_arg(if v { 1 } else { 0 }, Self::POS_K, Self::SIZE_K);
    }

    #[inline(always)]
    pub fn get_bx(self) -> u32 {
        self.get_arg(Self::POS_BX, Self::SIZE_BX)
    }

    #[inline(always)]
    pub fn get_sbx(self) -> i32 {
        self.get_bx() as i32 - Self::OFFSET_SBX
    }

    #[inline(always)]
    pub fn set_bx(&mut self, v: u32) {
        self.set_arg(v, Self::POS_BX, Self::SIZE_BX);
    }

    #[inline(always)]
    pub fn get_ax(self) -> u32 {
        self.get_arg(Self::POS_AX, Self::SIZE_AX)
    }

    #[inline(always)]
    pub fn set_ax(&mut self, v: u32) {
        self.set_arg(v, Self::POS_AX, Self::SIZE_AX);
    }

    #[inline(always)]
    pub fn get_sj(self) -> i32 {
        self.get_arg(Self::POS_SJ, Self::SIZE_SJ) as i32 - Self::OFFSET_SJ
    }

    #[inline(always)]
    pub fn set_sj(&mut self, v: i32) {
        self.set_arg((v + Self::OFFSET_SJ) as u32, Self::POS_SJ, Self::SIZE_SJ);
    }

    // Get vB and vC fields for vABCk format instructions (like NEWTABLE, SETLIST)
    #[inline(always)]
    pub fn get_vb(self) -> u32 {
        self.get_arg(Self::POS_V_B, Self::SIZE_V_B)
    }

    #[inline(always)]
    pub fn get_vc(self) -> u32 {
        self.get_arg(Self::POS_V_C, Self::SIZE_V_C)
    }

    // Instruction creation
    pub fn create_abc(op: OpCode, a: u32, b: u32, c: u32) -> Self {
        unsafe {
            Self(
                (op as u32).unchecked_shl(Self::POS_OP)
                    | a.unchecked_shl(Self::POS_A)
                    | b.unchecked_shl(Self::POS_B)
                    | c.unchecked_shl(Self::POS_C),
            )
        }
    }

    pub fn create_abck(op: OpCode, a: u32, b: u32, c: u32, k: bool) -> Self {
        unsafe {
            Self(
                (op as u32).unchecked_shl(Self::POS_OP)
                    | a.unchecked_shl(Self::POS_A)
                    | (if k { 1u32 } else { 0u32 }).unchecked_shl(Self::POS_K)
                    | b.unchecked_shl(Self::POS_B)
                    | c.unchecked_shl(Self::POS_C),
            )
        }
    }

    // Create instruction in vABCk format (variable-size B and C fields)
    // Used for instructions like NEWTABLE where C field is 10 bits instead of 8
    pub fn create_vabck(op: OpCode, a: u32, b: u32, c: u32, k: bool) -> Self {
        unsafe {
            Self(
                (op as u32).unchecked_shl(Self::POS_OP)
                    | a.unchecked_shl(Self::POS_A)
                    | (if k { 1u32 } else { 0u32 }).unchecked_shl(Self::POS_K)
                    | b.unchecked_shl(Self::POS_V_B)
                    | c.unchecked_shl(Self::POS_V_C),
            )
        }
    }

    pub fn create_abx(op: OpCode, a: u32, bx: u32) -> Self {
        unsafe {
            Self(
                (op as u32).unchecked_shl(Self::POS_OP)
                    | a.unchecked_shl(Self::POS_A)
                    | bx.unchecked_shl(Self::POS_BX),
            )
        }
    }

    pub fn create_asbx(op: OpCode, a: u32, sbx: i32) -> Self {
        Self::create_abx(op, a, (sbx + Self::OFFSET_SBX) as u32)
    }

    pub fn create_ax(op: OpCode, ax: u32) -> Self {
        unsafe { Self((op as u32).unchecked_shl(Self::POS_OP) | ax.unchecked_shl(Self::POS_AX)) }
    }

    pub fn create_sj(op: OpCode, sj: i32) -> Self {
        unsafe {
            Self(
                (op as u32).unchecked_shl(Self::POS_OP)
                    | ((sj + Self::OFFSET_SJ) as u32).unchecked_shl(Self::POS_SJ),
            )
        }
    }

    // Helper: RK(x) - if k then K[x] else R[x]
    #[inline(always)]
    pub fn is_k(x: u32) -> bool {
        unsafe { x & 1u32.unchecked_shl(Self::SIZE_B - 1) != 0 }
    }

    #[inline(always)]
    pub fn rk_index(x: u32) -> u32 {
        unsafe { x & !1u32.unchecked_shl(Self::SIZE_B - 1) }
    }

    // Convenience aliases for backwards compatibility
    #[inline(always)]
    pub fn encode_abc(op: OpCode, a: u32, b: u32, c: u32) -> Self {
        Self::create_abc(op, a, b, c)
    }

    #[inline(always)]
    pub fn encode_abck(op: OpCode, a: u32, b: u32, c: u32, k: u32) -> Self {
        Self::create_abck(op, a, b, c, k != 0)
    }

    #[inline(always)]
    pub fn encode_abx(op: OpCode, a: u32, bx: u32) -> Self {
        Self::create_abx(op, a, bx)
    }

    #[inline(always)]
    pub fn encode_asbx(op: OpCode, a: u32, sbx: i32) -> Self {
        Self::create_asbx(op, a, sbx)
    }
}

#[cfg(test)]
mod tests {
    use crate::{OpCode, lua_vm::opcode::OpMode};

    use super::*;

    #[test]
    fn test_instruction_abc() {
        let instr = Instruction::create_abc(OpCode::Move, 1, 2, 3);
        assert_eq!(instr.get_opcode(), OpCode::Move);
        assert_eq!(instr.get_a(), 1);
        assert_eq!(instr.get_b(), 2);
        assert_eq!(instr.get_c(), 3);
    }

    #[test]
    fn test_instruction_abck() {
        let instr = Instruction::create_abck(OpCode::Add, 5, 10, 20, true);
        assert_eq!(instr.get_opcode(), OpCode::Add);
        assert_eq!(instr.get_a(), 5);
        assert_eq!(instr.get_b(), 10);
        assert_eq!(instr.get_c(), 20);
        assert_eq!(instr.get_k(), true);
    }

    #[test]
    fn test_instruction_abx() {
        let instr = Instruction::create_abx(OpCode::LoadK, 3, 100);
        assert_eq!(instr.get_opcode(), OpCode::LoadK);
        assert_eq!(instr.get_a(), 3);
        assert_eq!(instr.get_bx(), 100);
    }

    #[test]
    fn test_instruction_asbx() {
        let instr = Instruction::create_asbx(OpCode::ForLoop, 2, -50);
        assert_eq!(instr.get_opcode(), OpCode::ForLoop);
        assert_eq!(instr.get_a(), 2);
        assert_eq!(instr.get_sbx(), -50);
    }

    #[test]
    fn test_instruction_ax() {
        let instr = Instruction::create_ax(OpCode::ExtraArg, 0xFFFFFF);
        assert_eq!(instr.get_opcode(), OpCode::ExtraArg);
        assert_eq!(instr.get_ax(), 0xFFFFFF);
    }

    #[test]
    fn test_instruction_sj() {
        let instr = Instruction::create_sj(OpCode::Jmp, 1000);
        assert_eq!(instr.get_opcode(), OpCode::Jmp);
        assert_eq!(instr.get_sj(), 1000);
    }

    #[test]
    fn test_instruction_boundaries() {
        // Test maximum values
        let max_a = Instruction::MAX_A;
        let max_b = Instruction::MAX_B;
        let max_c = Instruction::MAX_C;

        let instr = Instruction::create_abc(OpCode::Move, max_a, max_b, max_c);
        assert_eq!(instr.get_a(), max_a);
        assert_eq!(instr.get_b(), max_b);
        assert_eq!(instr.get_c(), max_c);
    }

    #[test]
    fn test_opcode_mode() {
        assert_eq!(OpCode::Move.get_mode(), OpMode::IABC);
        assert_eq!(OpCode::LoadK.get_mode(), OpMode::IABx);
        assert_eq!(OpCode::LoadI.get_mode(), OpMode::IAsBx);
        assert_eq!(OpCode::Jmp.get_mode(), OpMode::IsJ);
        assert_eq!(OpCode::ExtraArg.get_mode(), OpMode::IAx);
        assert_eq!(OpCode::Add.get_mode(), OpMode::IABC);
        assert_eq!(OpCode::TForCall.get_mode(), OpMode::IABC);
        assert_eq!(OpCode::TForLoop.get_mode(), OpMode::IABx);
        assert_eq!(OpCode::NewTable.get_mode(), OpMode::IvABC); // ivABC format in Lua 5.5
        assert_eq!(OpCode::SetList.get_mode(), OpMode::IvABC); // ivABC format in Lua 5.5
        assert_eq!(OpCode::ErrNNil.get_mode(), OpMode::IABx); // New in Lua 5.5
        assert_eq!(OpCode::GetVarg.get_mode(), OpMode::IABC); // New in Lua 5.5
    }

    #[test]
    fn test_set_fields() {
        let mut instr = Instruction::create_abc(OpCode::Move, 1, 2, 3);

        instr.set_a(10);
        assert_eq!(instr.get_a(), 10);
        assert_eq!(instr.get_b(), 2);
        assert_eq!(instr.get_c(), 3);

        instr.set_b(20);
        assert_eq!(instr.get_b(), 20);

        instr.set_c(30);
        assert_eq!(instr.get_c(), 30);

        assert_eq!(instr.get_opcode(), OpCode::Move);
    }

    #[test]
    fn test_signed_arguments() {
        // Test sBx (signed Bx)
        let instr_neg = Instruction::create_asbx(OpCode::ForLoop, 0, -100);
        assert_eq!(instr_neg.get_sbx(), -100);

        let instr_pos = Instruction::create_asbx(OpCode::ForLoop, 0, 100);
        assert_eq!(instr_pos.get_sbx(), 100);

        // Test sJ (signed jump)
        let jmp_neg = Instruction::create_sj(OpCode::Jmp, -500);
        assert_eq!(jmp_neg.get_sj(), -500);

        let jmp_pos = Instruction::create_sj(OpCode::Jmp, 500);
        assert_eq!(jmp_pos.get_sj(), 500);
    }

    #[test]
    fn test_bit_layout_detailed() {
        // Test iABC format with k bit at position 15
        let instr = Instruction::create_abck(OpCode::Add, 10, 20, 30, true);

        // Manual bit extraction to verify positions
        let raw = instr.as_u32();
        let op_bits = raw & 0x7F; // bits 0-6
        let a_bits = (raw >> 7) & 0xFF; // bits 7-14
        let k_bits = (raw >> 15) & 0x1; // bit 15
        let b_bits = (raw >> 16) & 0xFF; // bits 16-23
        let c_bits = (raw >> 24) & 0xFF; // bits 24-31

        assert_eq!(op_bits, OpCode::Add as u32);
        assert_eq!(a_bits, 10);
        assert_eq!(k_bits, 1);
        assert_eq!(b_bits, 20);
        assert_eq!(c_bits, 30);

        // Test with k=false
        let instr2 = Instruction::create_abck(OpCode::Add, 5, 15, 25, false);
        let k2_bits = (instr2.as_u32() >> 15) & 0x1;
        assert_eq!(k2_bits, 0);
        assert_eq!(instr2.get_k(), false);
    }

    #[test]
    fn test_position_constants() {
        // Verify all position constants match Lua 5.4 spec
        assert_eq!(Instruction::POS_OP, 0);
        assert_eq!(Instruction::POS_A, 7);
        assert_eq!(Instruction::POS_K, 15);
        assert_eq!(Instruction::POS_B, 16);
        assert_eq!(Instruction::POS_C, 24);
        assert_eq!(Instruction::POS_BX, 15); // BX starts at K position
    }

    #[test]
    fn test_size_constants() {
        // Verify all size constants match Lua 5.4 spec
        assert_eq!(Instruction::SIZE_OP, 7);
        assert_eq!(Instruction::SIZE_A, 8);
        assert_eq!(Instruction::SIZE_K, 1);
        assert_eq!(Instruction::SIZE_B, 8);
        assert_eq!(Instruction::SIZE_C, 8);
        assert_eq!(Instruction::SIZE_BX, 17); // K(1) + B(8) + C(8)
        assert_eq!(Instruction::SIZE_AX, 25); // BX(17) + A(8)
        assert_eq!(Instruction::SIZE_SJ, 25); // same as AX
    }

    #[test]
    fn test_offset_constants() {
        // Verify offset constants for signed fields (Lua 5.5)
        // OFFSET_SB = OFFSET_SC = (MAXARG_C >> 1) = (255 >> 1) = 127
        assert_eq!(Instruction::OFFSET_SB, 127);
        assert_eq!(Instruction::OFFSET_SC, 127);
        // OFFSET_SBX = (MAXARG_Bx >> 1) = ((1<<17)-1) >> 1 = 65535
        assert_eq!(Instruction::OFFSET_SBX, 65535);
        // OFFSET_SJ = (MAXARG_sJ >> 1) = ((1<<25)-1) >> 1 = 16777215
        assert_eq!(Instruction::OFFSET_SJ, 16777215);
    }

    #[test]
    fn test_signed_b_field() {
        // Test sB field (signed B, range -127 to 128, using OFFSET_SB=127)
        let pos_instr = Instruction::create_abc(OpCode::EqI, 0, 127 + 10, 0);
        assert_eq!(pos_instr.get_sb(), 10);

        let neg_instr = Instruction::create_abc(OpCode::EqI, 0, 127 - 10, 0);
        assert_eq!(neg_instr.get_sb(), -10);

        let zero_instr = Instruction::create_abc(OpCode::EqI, 0, 127, 0);
        assert_eq!(zero_instr.get_sb(), 0);
    }

    #[test]
    fn test_signed_c_field() {
        // Test sC field (signed C, range -127 to 128)
        let pos_instr = Instruction::create_abc(OpCode::ShrI, 0, 0, 127 + 10);
        assert_eq!(pos_instr.get_sc(), 10);

        let neg_instr = Instruction::create_abc(OpCode::ShrI, 0, 0, 127 - 10);
        assert_eq!(neg_instr.get_sc(), -10);

        let zero_instr = Instruction::create_abc(OpCode::ShrI, 0, 0, 127);
        assert_eq!(zero_instr.get_sc(), 0);
    }

    #[test]
    fn test_return_instruction_k_bit() {
        // RETURN instruction should have k=1 for final return
        let ret = Instruction::create_abck(OpCode::Return, 12, 2, 1, true);
        assert_eq!(ret.get_opcode(), OpCode::Return);
        assert_eq!(ret.get_a(), 12);
        assert_eq!(ret.get_b(), 2);
        assert_eq!(ret.get_c(), 1);
        assert_eq!(ret.get_k(), true);
    }
}
