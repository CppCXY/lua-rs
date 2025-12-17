// Expression parsing - port from lparser.c
use crate::compiler::{code, VarKind};
use crate::compiler::expression::ExpDesc;
use crate::compiler::func_state::FuncState;
use crate::compiler::parse_literal::{
    NumberResult, parse_float_token_value, parse_int_token_value, parse_string_token_value,
};
use crate::compiler::parser::LuaTokenKind;
use crate::lua_vm::OpCode;

// Binary operator priorities (from lparser.c)
const PRIORITY: [(u8, u8); 14] = [
    (10, 10), // +
    (10, 10), // -
    (11, 11), // *
    (11, 11), // /
    (11, 11), // %
    (14, 13), // ^ (right associative)
    (11, 11), // //
    (6, 6),   // &
    (4, 4),   // |
    (5, 5),   // ~
    (7, 7),   // <<
    (7, 7),   // >>
    (9, 8),   // ..  (right associative)
    (3, 3),   // ==, <, <=, ~=, >, >=
];

const UNARY_PRIORITY: u8 = 12;

// Get binary operator from token
fn get_binop(tk: LuaTokenKind) -> Option<usize> {
    match tk {
        LuaTokenKind::TkPlus => Some(0),
        LuaTokenKind::TkMinus => Some(1),
        LuaTokenKind::TkMul => Some(2),
        LuaTokenKind::TkDiv => Some(3),
        LuaTokenKind::TkMod => Some(4),
        LuaTokenKind::TkPow => Some(5),
        LuaTokenKind::TkIDiv => Some(6),
        LuaTokenKind::TkBitAnd => Some(7),
        LuaTokenKind::TkBitOr => Some(8),
        LuaTokenKind::TkBitXor => Some(9),
        LuaTokenKind::TkShl => Some(10),
        LuaTokenKind::TkShr => Some(11),
        LuaTokenKind::TkConcat => Some(12),
        LuaTokenKind::TkEq
        | LuaTokenKind::TkLt
        | LuaTokenKind::TkLe
        | LuaTokenKind::TkNe
        | LuaTokenKind::TkGt
        | LuaTokenKind::TkGe => Some(13),
        _ => None,
    }
}

// Port of expr from lparser.c
pub fn expr(fs: &mut FuncState) -> Result<ExpDesc, String> {
    let mut v = ExpDesc::new_void();
    subexpr(fs, &mut v, 0)?;
    Ok(v)
}

// Internal version that uses mutable reference
fn expr_internal(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    subexpr(fs, v, 0)?;
    Ok(())
}

// Port of subexpr from lparser.c
fn subexpr(fs: &mut FuncState, v: &mut ExpDesc, limit: u8) -> Result<u8, String> {
    let uop = get_unop(fs.lexer.current_token());
    if uop.is_some() {
        fs.lexer.bump();
        subexpr(fs, v, UNARY_PRIORITY)?;
        // code_unary(fs, uop, v)?;
    } else {
        simpleexp(fs, v)?;
    }

    // Expand while operators have priorities higher than limit
    let mut op = get_binop(fs.lexer.current_token());
    while op.is_some() && PRIORITY[op.unwrap()].0 > limit {
        fs.lexer.bump();

        let mut v2 = ExpDesc::new_void();
        let _nextop = subexpr(fs, &mut v2, PRIORITY[op.unwrap()].1)?;

        // code_binop(fs, op, v, &v2)?;
        code::exp2nextreg(fs, v);
        code::exp2nextreg(fs, &mut v2);

        op = get_binop(fs.lexer.current_token());
    }

    Ok(op.map(|o| PRIORITY[o].0).unwrap_or(0))
}

// Get unary operator
fn get_unop(tk: LuaTokenKind) -> Option<OpCode> {
    match tk {
        LuaTokenKind::TkNot => Some(OpCode::Not),
        LuaTokenKind::TkMinus => Some(OpCode::Unm),
        LuaTokenKind::TkBitXor => Some(OpCode::BNot),
        LuaTokenKind::TkLen => Some(OpCode::Len),
        _ => None,
    }
}

// Port of simpleexp from lparser.c
fn simpleexp(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    match fs.lexer.current_token() {
        LuaTokenKind::TkInt => {
            // Parse integer literal using int_token_value
            let text = fs.lexer.current_token_text();
            match parse_int_token_value(text) {
                Ok(NumberResult::Int(val)) => {
                    *v = ExpDesc::new_int(val);
                }
                Ok(NumberResult::Uint(val)) => {
                    // Reinterpret unsigned as signed
                    *v = ExpDesc::new_int(val as i64);
                }
                Ok(NumberResult::Float(val)) => {
                    // Integer overflow, use float
                    *v = ExpDesc::new_float(val);
                }
                Err(e) => {
                    return Err(format!("invalid integer literal: {}", e));
                }
            }
            fs.lexer.bump();
        }
        LuaTokenKind::TkFloat => {
            // Parse float literal
            let num_text = fs.lexer.current_token_text();
            match parse_float_token_value(num_text) {
                Ok(val) => {
                    *v = ExpDesc::new_float(val);
                }
                Err(e) => {
                    return Err(format!("invalid float literal '{}': {}", num_text, e));
                }
            }
            fs.lexer.bump();
        }
        LuaTokenKind::TkString | LuaTokenKind::TkLongString => {
            // String constant - remove quotes
            let text = fs.lexer.current_token_text();
            let string_content = parse_string_token_value(text, fs.lexer.current_token());
            match string_content {
                Ok(s) => {
                    let idx = add_string_constant(fs, s);
                    *v = ExpDesc::new_k(idx);
                }
                Err(e) => {
                    return Err(format!("invalid string literal: {}", e));
                }
            }
            fs.lexer.bump();
        }
        LuaTokenKind::TkNil => {
            *v = ExpDesc::new_nil();
            fs.lexer.bump();
        }
        LuaTokenKind::TkTrue => {
            *v = ExpDesc::new_bool(true);
            fs.lexer.bump();
        }
        LuaTokenKind::TkFalse => {
            *v = ExpDesc::new_bool(false);
            fs.lexer.bump();
        }
        LuaTokenKind::TkDots => {
            // Vararg
            *v = ExpDesc::new_void();
            fs.lexer.bump();
        }
        LuaTokenKind::TkLeftBrace => {
            // Table constructor
            constructor(fs, v)?;
        }
        LuaTokenKind::TkFunction => {
            // Anonymous function
            fs.lexer.bump();
            body(fs, v, false)?;
        }
        _ => {
            // Try suffixed expression (variables, function calls, indexing)
            suffixedexp(fs, v)?;
        }
    }
    Ok(())
}

// Port of suffixedexp from lparser.c
pub fn suffixedexp(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    primaryexp(fs, v)?;

    loop {
        match fs.lexer.current_token() {
            LuaTokenKind::TkDot => {
                // t.field
                fs.lexer.bump();
                fieldsel(fs, v)?;
            }
            LuaTokenKind::TkLeftBracket => {
                // t[exp]
                fs.lexer.bump();
                let _key = ExpDesc::new_void();
                expr(fs)?;
                // indexed(fs, v, &key)?;
                expect(fs, LuaTokenKind::TkRightBracket)?;
            }
            LuaTokenKind::TkColon => {
                // t:method(args)
                fs.lexer.bump();
                fieldsel(fs, v)?;
                funcargs(fs, v)?;
            }
            LuaTokenKind::TkLeftParen | LuaTokenKind::TkString | LuaTokenKind::TkLeftBrace => {
                // Function call
                funcargs(fs, v)?;
            }
            _ => break,
        }
    }

    Ok(())
}

fn primaryexp(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    match fs.lexer.current_token() {
        LuaTokenKind::TkLeftParen => {
            // (expr)
            fs.lexer.bump();
            expr(fs)?;
            expect(fs, LuaTokenKind::TkRightParen)?;
        }
        LuaTokenKind::TkName => {
            // Variable name
            singlevar(fs, v)?;
        }
        _ => {
            return Err(format!(
                "unexpected symbol {}",
                fs.lexer.current_token_text()
            ));
        }
    }
    Ok(())
}

// Port of singlevar from lparser.c
pub fn singlevar(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    let name = fs.lexer.current_token_text().to_string();
    fs.lexer.bump();

    // Try to find local variable
    if let Some(idx) = search_var(fs, &name) {
        *v = ExpDesc::new_local(idx, 0);
    } else {
        // Global variable - load from _ENV
        *v = ExpDesc::new_void();
    }

    Ok(())
}

// Search for local variable
fn search_var(fs: &FuncState, name: &str) -> Option<u8> {
    for i in (0..fs.nactvar).rev() {
        if let Some(var) = fs.actvar.get(i as usize) {
            if var.name == name {
                return Some(i);
            }
        }
    }
    None
}

// Port of fieldsel from lparser.c
pub fn fieldsel(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    code::exp2anyreg(fs, v);

    if fs.lexer.current_token() != LuaTokenKind::TkName {
        return Err("expected field name".to_string());
    }

    let field = fs.lexer.current_token_text().to_string();
    fs.lexer.bump();

    let idx = add_string_constant(fs, field);
    *v = ExpDesc::new_indexed(unsafe { v.u.info as u8 }, idx as u8);

    Ok(())
}

// Port of funcargs from lparser.c
fn funcargs(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    let base = unsafe { v.u.info as u8 };
    let mut nargs = 0;

    match fs.lexer.current_token() {
        LuaTokenKind::TkLeftParen => {
            fs.lexer.bump();
            if fs.lexer.current_token() != LuaTokenKind::TkRightParen {
                nargs = explist(fs)?;
            }
            expect(fs, LuaTokenKind::TkRightParen)?;
        }
        LuaTokenKind::TkLeftBrace => {
            // Single table argument
            let mut e = ExpDesc::new_void();
            constructor(fs, &mut e)?;
            nargs = 1;
        }
        LuaTokenKind::TkString => {
            // Single string argument
            let mut e = ExpDesc::new_void();
            simpleexp(fs, &mut e)?;
            nargs = 1;
        }
        _ => {
            return Err("function arguments expected".to_string());
        }
    }

    let pc = code::code_abc(fs, OpCode::Call, base as u32, (nargs + 1) as u32, 2);
    *v = ExpDesc::new_call(pc);

    Ok(())
}

// Port of explist from lparser.c
fn explist(fs: &mut FuncState) -> Result<usize, String> {
    let mut n = 1;
    expr(fs)?;

    while fs.lexer.current_token() == LuaTokenKind::TkComma {
        fs.lexer.bump();
        expr(fs)?;
        n += 1;
    }

    Ok(n)
}

// Port of constructor from lparser.c
fn constructor(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    use crate::compiler::expression::ExpKind;
    
    expect(fs, LuaTokenKind::TkLeftBrace)?;

    let reg = fs.freereg;
    code::reserve_regs(fs, 1);
    code::code_abc(fs, OpCode::NewTable, reg as u32, 0, 0);
    *v = ExpDesc::new_nonreloc(reg);

    let mut list_items = 0; // Number of list items [1], [2], etc.
    
    // Parse table fields
    while fs.lexer.current_token() != LuaTokenKind::TkRightBrace {
        // Field or list item
        if fs.lexer.current_token() == LuaTokenKind::TkName {
            // Might be field or expression
            let next = fs.lexer.peek_next_token();
            if next == LuaTokenKind::TkAssign {
                // name = exp (record field)
                let field_name = fs.lexer.current_token_text().to_string();
                fs.lexer.bump();
                fs.lexer.bump(); // skip =
                
                let field_idx = add_string_constant(fs, field_name);
                let mut val = ExpDesc::new_void();
                expr_internal(fs, &mut val)?;
                
                // t[field] = val  -> SetField instruction
                let val_reg = code::exp2anyreg(fs, &mut val);
                code::code_abc(fs, OpCode::SetField, reg as u32, field_idx as u32, val_reg as u32);
            } else {
                // Just an expression in list
                list_items += 1;
                let mut val = ExpDesc::new_void();
                expr_internal(fs, &mut val)?;
                
                // t[list_items] = val -> SetI instruction
                let val_reg = code::exp2anyreg(fs, &mut val);
                code::code_abc(fs, OpCode::SetI, reg as u32, list_items, val_reg as u32);
            }
        } else if fs.lexer.current_token() == LuaTokenKind::TkLeftBracket {
            // [exp] = exp (general index)
            fs.lexer.bump();
            let mut key = ExpDesc::new_void();
            expr_internal(fs, &mut key)?;
            expect(fs, LuaTokenKind::TkRightBracket)?;
            expect(fs, LuaTokenKind::TkAssign)?;
            
            let mut val = ExpDesc::new_void();
            expr_internal(fs, &mut val)?;
            
            // Check if key is string constant for SetField optimization
            if key.kind == ExpKind::VKSTR {
                let key_idx = unsafe { key.u.info as u32 };
                let val_reg = code::exp2anyreg(fs, &mut val);
                code::code_abc(fs, OpCode::SetField, reg as u32, key_idx, val_reg as u32);
            } 
            // Check if key is integer constant for SetI optimization
            else if key.kind == ExpKind::VKINT {
                let key_int = unsafe { key.u.ival as u32 };
                let val_reg = code::exp2anyreg(fs, &mut val);
                code::code_abc(fs, OpCode::SetI, reg as u32, key_int, val_reg as u32);
            } else {
                // General case: SetTable instruction
                let key_reg = code::exp2anyreg(fs, &mut key);
                let val_reg = code::exp2anyreg(fs, &mut val);
                code::code_abc(fs, OpCode::SetTable, reg as u32, key_reg as u32, val_reg as u32);
            }
        } else {
            // List item without bracket
            list_items += 1;
            let mut val = ExpDesc::new_void();
            expr_internal(fs, &mut val)?;
            
            // t[list_items] = val -> SetI instruction
            let val_reg = code::exp2anyreg(fs, &mut val);
            code::code_abc(fs, OpCode::SetI, reg as u32, list_items, val_reg as u32);
        }

        // Optional separator
        if !matches!(
            fs.lexer.current_token(),
            LuaTokenKind::TkComma | LuaTokenKind::TkSemicolon
        ) {
            break;
        }
        fs.lexer.bump();
    }

    expect(fs, LuaTokenKind::TkRightBrace)?;
    Ok(())
}

// Port of body from lparser.c
pub fn body(fs: &mut FuncState, v: &mut ExpDesc, is_method: bool) -> Result<(), String> {
    expect(fs, LuaTokenKind::TkLeftParen)?;

    // Parse parameters
    let mut nparams = 0;

    if is_method {
        // Add 'self' parameter for method
        fs.new_localvar("self".to_string(), VarKind::VDKREG);
        nparams = 1;
    }

    if fs.lexer.current_token() != LuaTokenKind::TkRightParen {
        loop {
            if fs.lexer.current_token() == LuaTokenKind::TkName {
                let param_name = fs.lexer.current_token_text().to_string();
                fs.lexer.bump();
                fs.new_localvar(param_name, VarKind::VDKREG);
                nparams += 1;
            } else if fs.lexer.current_token() == LuaTokenKind::TkDots {
                fs.lexer.bump();
                // Mark as vararg function
                break;
            } else {
                return Err("expected parameter".to_string());
            }

            if fs.lexer.current_token() != LuaTokenKind::TkComma {
                break;
            }
            fs.lexer.bump();
        }
    }

    expect(fs, LuaTokenKind::TkRightParen)?;

    // Adjust local variables for parameters
    fs.adjust_local_vars(nparams);

    // Parse body
    use crate::compiler::statement;
    statement::statlist(fs)?;

    expect(fs, LuaTokenKind::TkEnd)?;

    // Generate RETURN 0 0 to return nothing
    code::code_abc(fs, OpCode::Return, 0, 1, 0);

    *v = ExpDesc::new_void();
    Ok(())
}

// Helper: expect a token
fn expect(fs: &mut FuncState, tk: LuaTokenKind) -> Result<(), String> {
    if fs.lexer.current_token() == tk {
        fs.lexer.bump();
        Ok(())
    } else {
        Err(format!(
            "expected {:?}, got {:?}",
            tk,
            fs.lexer.current_token()
        ))
    }
}

// Add string constant to chunk
fn add_string_constant(fs: &mut FuncState, s: String) -> usize {
    // Intern string to ObjectPool and get StringId
    let (string_id, _is_new) = fs.pool.create_string(&s);
    
    // Add LuaValue with StringId to constants
    let value = crate::lua_value::LuaValue::string(string_id);
    fs.chunk.constants.push(value);
    
    fs.chunk.constants.len() - 1
}
