// Lua VM opcodes (simplified instruction set)
// Each instruction is encoded as a 32-bit word
// Format: [opcode: 6][A: 8][B: 9][C: 9] or [opcode: 6][A: 8][Bx: 18]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpCode {
    // Load/Store
    Move = 0, // R(A) := R(B)
    LoadK,    // R(A) := K(Bx)
    LoadNil,  // R(A) := nil
    LoadBool, // R(A) := bool(B); if C then pc++

    // Table operations
    NewTable,  // R(A) := {} (size = B,C)
    GetTable,  // R(A) := R(B)[R(C)]
    SetTable,  // R(A)[R(B)] := R(C)
    GetTableI, // R(A) := R(B)[C]  - Direct integer index (C is literal int)
    SetTableI, // R(A)[B] := R(C)  - Direct integer index (B is literal int)
    GetTableK, // R(A) := R(B)[K(C)] - Direct constant key (K(C) must be string)
    SetTableK, // R(A)[K(B)] := R(C) - Direct constant key (K(B) must be string)

    // Arithmetic operations
    Add, // R(A) := R(B) + R(C)
    Sub, // R(A) := R(B) - R(C)
    Mul, // R(A) := R(B) * R(C)
    Div, // R(A) := R(B) / R(C)
    Mod, // R(A) := R(B) % R(C)
    Pow, // R(A) := R(B) ^ R(C)
    Unm, // R(A) := -R(B)

    // Logical operations
    Not, // R(A) := not R(B)
    Len, // R(A) := length of R(B)

    // Comparison
    Eq, // if (R(B) == R(C)) ~= A then pc++
    Lt, // if (R(B) < R(C)) ~= A then pc++
    Le, // if (R(B) <= R(C)) ~= A then pc++
    Ne, // if (R(B) != R(C)) ~= A then pc++
    Gt, // if (R(B) > R(C)) ~= A then pc++
    Ge, // if (R(B) >= R(C)) ~= A then pc++

    // Logical operations (short-circuit)
    And, // R(A) := R(B) and R(C)
    Or,  // R(A) := R(B) or R(C)

    // Bitwise operations
    BAnd, // R(A) := R(B) & R(C)
    BOr,  // R(A) := R(B) | R(C)
    BXor, // R(A) := R(B) ~ R(C)
    Shl,  // R(A) := R(B) << R(C)
    Shr,  // R(A) := R(B) >> R(C)
    BNot, // R(A) := ~R(B)

    // Integer division
    IDiv, // R(A) := R(B) // R(C)

    // Numeric for loop
    ForPrep, // R(A) -= R(A+2); pc += sBx
    ForLoop, // R(A) += R(A+2); if R(A) <?= R(A+1) then pc += sBx; R(A+3) = R(A)

    // Control flow
    Jmp,     // pc += sBx
    Test,    // if not (R(A) <=> C) then pc++
    TestSet, // if (R(B) <=> C) then R(A) := R(B) else pc++

    // Function calls
    Call,   // R(A), ... ,R(A+C-2) := R(A)(R(A+1), ... ,R(A+B-1))
    Return, // return R(A), ... ,R(A+B-2)

    // Upvalues
    GetUpval, // R(A) := UpValue[B]
    SetUpval, // UpValue[B] := R(A)

    // Closure
    Closure, // R(A) := closure(KPROTO[Bx])

    // Concatenation
    Concat, // R(A) := R(B).. ... ..R(C)

    // Global
    GetGlobal, // R(A) := Gbl[K(Bx)]
    SetGlobal, // Gbl[K(Bx)] := R(A)
}

impl OpCode {
    pub fn from_u8(byte: u8) -> Option<Self> {
        if byte <= OpCode::SetGlobal as u8 {
            Some(unsafe { std::mem::transmute(byte) })
        } else {
            None
        }
    }
}

/// Instruction encoding/decoding utilities
pub struct Instruction;

impl Instruction {
    const OPCODE_BITS: u32 = 6;
    const A_BITS: u32 = 8;
    const B_BITS: u32 = 9;
    const C_BITS: u32 = 9;
    const BX_BITS: u32 = 18;

    const MAX_A: u32 = (1 << Self::A_BITS) - 1;
    const MAX_B: u32 = (1 << Self::B_BITS) - 1;
    const MAX_C: u32 = (1 << Self::C_BITS) - 1;
    const MAX_BX: u32 = (1 << Self::BX_BITS) - 1;

    const A_OFFSET: u32 = Self::OPCODE_BITS;
    const B_OFFSET: u32 = Self::A_OFFSET + Self::A_BITS;
    const C_OFFSET: u32 = Self::B_OFFSET + Self::B_BITS;
    const BX_OFFSET: u32 = Self::A_OFFSET + Self::A_BITS; // Bx comes after A

    // Encode ABC format
    pub fn encode_abc(op: OpCode, a: u32, b: u32, c: u32) -> u32 {
        assert!(
            a <= Self::MAX_A,
            "Instruction {:?}: A={} exceeds MAX_A={}",
            op,
            a,
            Self::MAX_A
        );
        assert!(
            b <= Self::MAX_B,
            "Instruction {:?}: B={} exceeds MAX_B={}",
            op,
            b,
            Self::MAX_B
        );
        assert!(
            c <= Self::MAX_C,
            "Instruction {:?}: C={} exceeds MAX_C={}",
            op,
            c,
            Self::MAX_C
        );

        (op as u32) | (a << Self::A_OFFSET) | (b << Self::B_OFFSET) | (c << Self::C_OFFSET)
    }

    // Encode ABx format
    pub fn encode_abx(op: OpCode, a: u32, bx: u32) -> u32 {
        assert!(a <= Self::MAX_A);
        assert!(bx <= Self::MAX_BX);

        (op as u32) | (a << Self::A_OFFSET) | (bx << Self::BX_OFFSET)
    }

    // Encode AsBx format (signed Bx)
    pub fn encode_asbx(op: OpCode, a: u32, sbx: i32) -> u32 {
        assert!(a <= Self::MAX_A);
        let bx = (sbx + (Self::MAX_BX as i32 / 2)) as u32;
        assert!(bx <= Self::MAX_BX);

        Self::encode_abx(op, a, bx)
    }

    // Decode opcode
    #[inline(always)]
    pub fn get_opcode(instr: u32) -> OpCode {
        let opcode_byte = (instr & ((1 << Self::OPCODE_BITS) - 1)) as u8;
        OpCode::from_u8(opcode_byte).expect("Invalid opcode")
    }

    // Decode A field
    #[inline(always)]
    pub fn get_a(instr: u32) -> u32 {
        (instr >> Self::A_OFFSET) & ((1 << Self::A_BITS) - 1)
    }

    // Decode B field
    #[inline(always)]
    pub fn get_b(instr: u32) -> u32 {
        (instr >> Self::B_OFFSET) & ((1 << Self::B_BITS) - 1)
    }

    // Decode C field
    #[inline(always)]
    pub fn get_c(instr: u32) -> u32 {
        (instr >> Self::C_OFFSET) & ((1 << Self::C_BITS) - 1)
    }

    // Decode Bx field
    #[inline(always)]
    pub fn get_bx(instr: u32) -> u32 {
        (instr >> Self::BX_OFFSET) & ((1 << Self::BX_BITS) - 1)
    }

    // Decode sBx field (signed)
    #[inline(always)]
    pub fn get_sbx(instr: u32) -> i32 {
        Self::get_bx(instr) as i32 - (Self::MAX_BX as i32 / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instruction_encoding() {
        let instr = Instruction::encode_abc(OpCode::Add, 1, 2, 3);
        assert_eq!(Instruction::get_opcode(instr), OpCode::Add);
        assert_eq!(Instruction::get_a(instr), 1);
        assert_eq!(Instruction::get_b(instr), 2);
        assert_eq!(Instruction::get_c(instr), 3);
    }

    #[test]
    fn test_instruction_bx() {
        let instr = Instruction::encode_abx(OpCode::LoadK, 5, 100);
        println!("Encoded instruction: {:#034b}", instr);
        println!("OpCode: {:?}", Instruction::get_opcode(instr));
        println!("A: {}", Instruction::get_a(instr));
        println!("Bx: {}", Instruction::get_bx(instr));

        assert_eq!(Instruction::get_opcode(instr), OpCode::LoadK);
        assert_eq!(Instruction::get_bx(instr), 100);
        assert_eq!(Instruction::get_a(instr), 5);
    }

    #[test]
    fn test_instruction_sbx() {
        let instr = Instruction::encode_asbx(OpCode::Jmp, 0, -50);
        assert_eq!(Instruction::get_opcode(instr), OpCode::Jmp);
        assert_eq!(Instruction::get_sbx(instr), -50);
    }
}
