/*----------------------------------------------------------------------
  Lua 5.4 Opcode System - Complete 1:1 Port from lopcodes.h

  Instruction Format (32-bit):
  - iABC:  [Op(7) | A(8) | k(1) | B(8) | C(8)]
  - iABx:  [Op(7) | A(8) | Bx(17)]
  - iAsBx: [Op(7) | A(8) | sBx(signed 17)]
  - iAx:   [Op(7) | Ax(25)]
  - isJ:   [Op(7) | sJ(signed 25)]
----------------------------------------------------------------------*/

// ============ Instruction Decoding Macros ============
// These macros are guaranteed to be inlined and produce optimal code.
// Use these instead of Instruction::get_* functions in hot paths.

use crate::OpCode;

#[macro_export]
macro_rules! get_op {
    ($instr:expr) => {
        OpCode::from_u8((($instr) & 0x7F) as u8)
    };
}

/// Get A field (bits 7-14, 8 bits)
#[macro_export]
macro_rules! get_a {
    ($instr:expr) => {
        ((($instr) >> 7) & 0xFF) as usize
    };
}

/// Get B field (bits 16-23, 8 bits)
#[macro_export]
macro_rules! get_b {
    ($instr:expr) => {
        ((($instr) >> 16) & 0xFF) as usize
    };
}

/// Get C field (bits 24-31, 8 bits)
#[macro_export]
macro_rules! get_c {
    ($instr:expr) => {
        ((($instr) >> 24) & 0xFF) as usize
    };
}

/// Get k flag (bit 15, 1 bit)
#[macro_export]
macro_rules! get_k {
    ($instr:expr) => {
        ((($instr) >> 15) & 1) != 0
    };
}

/// Get Bx field (bits 15-31, 17 bits, unsigned)
#[macro_export]
macro_rules! get_bx {
    ($instr:expr) => {
        (($instr) >> 15) as usize
    };
}

/// Get sBx field (bits 15-31, 17 bits, signed with offset 0xFFFF)
#[macro_export]
macro_rules! get_sbx {
    ($instr:expr) => {
        ((($instr) >> 15) as i32) - 0xFFFF
    };
}

/// Get Ax field (bits 7-31, 25 bits, unsigned)
#[macro_export]
macro_rules! get_ax {
    ($instr:expr) => {
        (($instr) >> 7) as usize
    };
}

/// Get sJ field (bits 7-31, 25 bits, signed with offset 0xFFFFFF)
#[macro_export]
macro_rules! get_sj {
    ($instr:expr) => {
        ((($instr) >> 7) as i32) - 0xFFFFFF
    };
}

/// Get sB field (signed B, offset 128)
#[macro_export]
macro_rules! get_sb {
    ($instr:expr) => {
        (((($instr) >> 16) & 0xFF) as i32) - 128
    };
}

/// Get sC field (signed C, offset 127)
#[macro_export]
macro_rules! get_sc {
    ($instr:expr) => {
        (((($instr) >> 24) & 0xFF) as i32) - 127
    };
}

pub struct Instruction;

impl Instruction {
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

    // Offsets for signed arguments
    pub const OFFSET_SB: i32 = 128; // For signed B field (-128 to 127)
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
        OpCode::from_u8(op_byte)
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
        Self::get_b(i) as i32 - Self::OFFSET_SB
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

    // Get vB and vC fields for vABCk format instructions (like NEWTABLE, SETLIST)
    #[inline(always)]
    pub fn get_vb(i: u32) -> u32 {
        Self::get_arg(i, Self::POS_V_B, Self::SIZE_V_B)
    }

    #[inline(always)]
    pub fn get_vc(i: u32) -> u32 {
        Self::get_arg(i, Self::POS_V_C, Self::SIZE_V_C)
    }

    // Instruction creation
    pub fn create_abc(op: OpCode, a: u32, b: u32, c: u32) -> u32 {
        ((op as u32) << Self::POS_OP) | (a << Self::POS_A) | (b << Self::POS_B) | (c << Self::POS_C)
    }

    pub fn create_abck(op: OpCode, a: u32, b: u32, c: u32, k: bool) -> u32 {
        ((op as u32) << Self::POS_OP)
            | (a << Self::POS_A)
            | ((if k { 1 } else { 0 }) << Self::POS_K)
            | (b << Self::POS_B)
            | (c << Self::POS_C)
    }

    // Create instruction in vABCk format (variable-size B and C fields)
    // Used for instructions like NEWTABLE where C field is 10 bits instead of 8
    pub fn create_vabck(op: OpCode, a: u32, b: u32, c: u32, k: bool) -> u32 {
        ((op as u32) << Self::POS_OP)
            | (a << Self::POS_A)
            | ((if k { 1 } else { 0 }) << Self::POS_K)
            | (b << Self::POS_V_B)
            | (c << Self::POS_V_C)
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
    pub fn encode_abck(op: OpCode, a: u32, b: u32, c: u32, k: u32) -> u32 {
        Self::create_abck(op, a, b, c, k != 0)
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
    use crate::{OpCode, lua_vm::opcode::OpMode};

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
        assert_eq!(OpCode::Jmp.get_mode(), OpMode::IsJ); // JMP uses sJ format (signed jump)
        assert_eq!(OpCode::ExtraArg.get_mode(), OpMode::IAx);
        assert_eq!(OpCode::Add.get_mode(), OpMode::IABC);
        assert_eq!(OpCode::TForCall.get_mode(), OpMode::IABC); // TFORCALL uses ABC format
        assert_eq!(OpCode::TForLoop.get_mode(), OpMode::IABx); // TFORLOOP uses ABx format
        assert_eq!(OpCode::LoadI.get_mode(), OpMode::IAsBx); // LOADI uses signed sBx
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

    #[test]
    fn test_bit_layout_detailed() {
        // Test iABC format with k bit at position 15
        let instr = Instruction::create_abck(OpCode::Add, 10, 20, 30, true);

        // Manual bit extraction to verify positions
        let op_bits = instr & 0x7F; // bits 0-6
        let a_bits = (instr >> 7) & 0xFF; // bits 7-14
        let k_bits = (instr >> 15) & 0x1; // bit 15
        let b_bits = (instr >> 16) & 0xFF; // bits 16-23
        let c_bits = (instr >> 24) & 0xFF; // bits 24-31

        assert_eq!(op_bits, OpCode::Add as u32);
        assert_eq!(a_bits, 10);
        assert_eq!(k_bits, 1);
        assert_eq!(b_bits, 20);
        assert_eq!(c_bits, 30);

        // Test with k=false
        let instr2 = Instruction::create_abck(OpCode::Add, 5, 15, 25, false);
        let k2_bits = (instr2 >> 15) & 0x1;
        assert_eq!(k2_bits, 0);
        assert_eq!(Instruction::get_k(instr2), false);
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
        // Verify offset constants for signed fields
        assert_eq!(Instruction::OFFSET_SB, 128);
        assert_eq!(Instruction::OFFSET_SBX, 65535);
        assert_eq!(Instruction::OFFSET_SJ, 16777215);
        assert_eq!(Instruction::OFFSET_SC, 127);
    }

    #[test]
    fn test_signed_b_field() {
        // Test sB field (signed B, range -128 to 127)
        let pos_instr = Instruction::create_abc(OpCode::EqI, 0, 128 + 10, 0);
        assert_eq!(Instruction::get_sb(pos_instr), 10);

        let neg_instr = Instruction::create_abc(OpCode::EqI, 0, 128 - 10, 0);
        assert_eq!(Instruction::get_sb(neg_instr), -10);

        let zero_instr = Instruction::create_abc(OpCode::EqI, 0, 128, 0);
        assert_eq!(Instruction::get_sb(zero_instr), 0);
    }

    #[test]
    fn test_signed_c_field() {
        // Test sC field (signed C, range -127 to 128)
        let pos_instr = Instruction::create_abc(OpCode::ShrI, 0, 0, 127 + 10);
        assert_eq!(Instruction::get_sc(pos_instr), 10);

        let neg_instr = Instruction::create_abc(OpCode::ShrI, 0, 0, 127 - 10);
        assert_eq!(Instruction::get_sc(neg_instr), -10);

        let zero_instr = Instruction::create_abc(OpCode::ShrI, 0, 0, 127);
        assert_eq!(Instruction::get_sc(zero_instr), 0);
    }

    #[test]
    fn test_return_instruction_k_bit() {
        // RETURN instruction should have k=1 for final return
        let ret = Instruction::create_abck(OpCode::Return, 12, 2, 1, true);
        assert_eq!(Instruction::get_opcode(ret), OpCode::Return);
        assert_eq!(Instruction::get_a(ret), 12);
        assert_eq!(Instruction::get_b(ret), 2);
        assert_eq!(Instruction::get_c(ret), 1);
        assert_eq!(Instruction::get_k(ret), true);
    }
}
