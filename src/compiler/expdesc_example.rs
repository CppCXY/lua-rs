/// Example: How to use ExpDesc system for assignment statements
/// This demonstrates the correct Lua-style register allocation

use super::exp2reg::*;
use super::expdesc::*;
use super::helpers::*;
use super::Compiler;

/// Example: Compile simple assignment using ExpDesc
/// 
/// For statement: `z = y + 50`
/// 
/// Old way (incorrect):
/// 1. alloc temp_reg for `y + 50` expression → R(2)
/// 2. compile `y` → allocates R(3), emits GETTABUP R(3)
/// 3. compile `+ 50` → uses R(3), result in R(2)
/// Result: Wrong register allocation!
///
/// New way (correct, using ExpDesc):
/// 1. Create ExpDesc for `y` (VIndexUp)
/// 2. Call exp_to_next_reg() → allocates R(2), emits GETTABUP R(2)
/// 3. Create ExpDesc for `+ 50` operation with left=R(2)
/// 4. Result in R(2), emit ADDI R(2) R(2) 50
/// 5. Use R(2) for SETTABUP
/// Result: Correct! Only one register allocated.
///
/// Key insight: ExpDesc delays code generation until we know the target register.
/// This is exactly how Lua works!

pub fn example_compile_global_assign(c: &mut Compiler, var_name: &str, value_exp_desc: &mut ExpDesc) {
    // Step 1: Compile the value expression to next available register
    // This will allocate R(freereg) and generate code
    exp_to_next_reg(c, value_exp_desc);
    
    // Step 2: Now value is in a register, emit SETTABUP
    let value_reg = value_exp_desc.info;
    emit_set_global(c, var_name, value_reg);
    
    // Step 3: Free the temporary register
    free_exp(c, value_exp_desc);
}

/// Example: How to compile binary operation with ExpDesc
pub fn example_compile_add_immediate(c: &mut Compiler, left: &mut ExpDesc, imm: u32) -> ExpDesc {
    // Step 1: Ensure left operand is in a register
    let left_reg = exp_to_any_reg(c, left);
    
    // Step 2: Emit ADDI instruction - result goes to same register as left
    // This is the key: reuse the same register!
    emit(
        c,
        crate::lua_vm::Instruction::encode_abc(
            crate::lua_vm::OpCode::AddI,
            left_reg,
            left_reg,
            imm,
        ),
    );
    
    // Step 3: Return ExpDesc for result (in same register as left)
    ExpDesc::new_nonreloc(left_reg)
}

/// Full example: z = y + 50
pub fn example_full_statement(c: &mut Compiler) {
    // Assume we're in a state where:
    // - freereg = 2 (two locals: x and a)
    // - nactvar = 2
    
    // Step 1: Create ExpDesc for global variable 'y'
    // This doesn't generate code yet, just describes what 'y' is
    let y_name_const = add_constant_dedup(c, create_string_value(c, "y"));
    let mut y_exp = ExpDesc {
        kind: ExpKind::VIndexUp,
        info: 0,
        ind: IndexInfo {
            t: 0,          // _ENV upvalue
            idx: y_name_const,
        },
        ..ExpDesc::new_void()
    };
    
    // Step 2: Convert y to a register
    // This will: allocate R(2), emit GETTABUP R(2) 0 const[y]
    exp_to_next_reg(c, &mut y_exp);
    // Now: y_exp.kind = VNonReloc, y_exp.info = 2, freereg = 3
    
    // Step 3: Add immediate 50
    let mut result_exp = example_compile_add_immediate(c, &mut y_exp, 50);
    // Emitted: ADDI R(2) R(2) 50
    // result_exp.kind = VNonReloc, result_exp.info = 2, freereg still 3
    
    // Step 4: Assign to z
    let z_name_const = add_constant_dedup(c, create_string_value(c, "z"));
    emit(
        c,
        crate::lua_vm::Instruction::create_abck(
            crate::lua_vm::OpCode::SetTabUp,
            0,              // _ENV upvalue
            z_name_const,   // key
            result_exp.info, // value register (R(2))
            false,          // k=0 (value is register, not constant)
        ),
    );
    
    // Step 5: Free the temporary register
    free_exp(c, &result_exp);
    // Now freereg = 2 (back to just the two locals)
    
    // Step 6: Reset freereg at statement end
    reset_freereg(c);
    // Ensures freereg = nactvar = 2
}

// This example shows:
// 1. Only ONE register (R(2)) is allocated for the entire expression
// 2. GETTABUP uses R(2)
// 3. ADDI uses R(2) for both source and destination
// 4. SETTABUP reads from R(2)
// 5. After the statement, freereg resets to 2
//
// This matches Lua's behavior perfectly!
