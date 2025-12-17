// Expression parsing - port from lparser.c
use crate::compiler::expression::{ExpDesc, ExpKind};
use crate::compiler::func_state::FuncState;
use crate::compiler::parse_literal::{
    NumberResult, parse_float_token_value, parse_int_token_value, parse_string_token_value,
};
use crate::compiler::parser::LuaTokenKind;
use crate::compiler::{VarKind, string_k, code};
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

// Map index to BinOpr for infix processing
const BINOP_FOR_INFIX: [code::BinOpr; 14] = [
    code::BinOpr::Add,    // 0: +
    code::BinOpr::Sub,    // 1: -
    code::BinOpr::Mul,    // 2: *
    code::BinOpr::Div,    // 3: /
    code::BinOpr::Mod,    // 4: %
    code::BinOpr::Pow,    // 5: ^
    code::BinOpr::IDiv,   // 6: //
    code::BinOpr::BAnd,   // 7: &
    code::BinOpr::BOr,    // 8: |
    code::BinOpr::BXor,   // 9: ~
    code::BinOpr::Shl,    // 10: <<
    code::BinOpr::Shr,    // 11: >>
    code::BinOpr::Concat, // 12: ..
    code::BinOpr::Eq,     // 13: ==, <, <=, ~=, >, >=
];

// Map index to OpCode for posfix code generation
const BINOP_OPCODES: [OpCode; 14] = [
    OpCode::Add,    // 0: +
    OpCode::Sub,    // 1: -
    OpCode::Mul,    // 2: *
    OpCode::Div,    // 3: /
    OpCode::Mod,    // 4: %
    OpCode::Pow,    // 5: ^
    OpCode::IDiv,   // 6: //
    OpCode::BAnd,   // 7: &
    OpCode::BOr,    // 8: |
    OpCode::BXor,   // 9: ~
    OpCode::Shl,    // 10: <<
    OpCode::Shr,    // 11: >>
    OpCode::Concat, // 12: ..
    OpCode::Eq,     // 13: ==, <, <=, ~=, >, >=
];

// Port of init_exp from lparser.c
fn init_exp(e: &mut ExpDesc, kind: ExpKind, info: i32) {
    e.kind = kind;
    e.u.info = info;
    e.t = -1;
    e.f = -1;
}

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
pub(crate) fn expr_internal(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    subexpr(fs, v, 0)?;
    Ok(())
}

// Port of subexpr from lparser.c
fn subexpr(fs: &mut FuncState, v: &mut ExpDesc, limit: u8) -> Result<u8, String> {
    let uop = get_unop(fs.lexer.current_token());
    if uop.is_some() {
        let op = uop.unwrap();
        fs.lexer.bump();
        subexpr(fs, v, UNARY_PRIORITY)?;
        code::prefix(fs, op, v);
    } else {
        simpleexp(fs, v)?;
    }

    // Expand while operators have priorities higher than limit
    let mut op = get_binop(fs.lexer.current_token());
    while op.is_some() && PRIORITY[op.unwrap()].0 > limit {
        let op_idx = op.unwrap();
        let bin_opr = BINOP_FOR_INFIX[op_idx];
        let opcode = BINOP_OPCODES[op_idx];
        fs.lexer.bump();

        code::infix(fs, bin_opr, v);

        let mut v2 = ExpDesc::new_void();
        let _nextop = subexpr(fs, &mut v2, PRIORITY[op_idx].1)?;

        code::posfix(fs, opcode, v, &mut v2);

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
                    let idx = string_k(fs, s);
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

// Port of primaryexp from lparser.c (lines 1080-1099)
// primaryexp -> NAME | '(' expr ')'
fn primaryexp(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    match fs.lexer.current_token() {
        LuaTokenKind::TkLeftParen => {
            // (expr)
            fs.lexer.bump();
            expr_internal(fs, v)?;
            expect(fs, LuaTokenKind::TkRightParen)?;
            code::discharge_vars(fs, v);
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

// Port of suffixedexp from lparser.c (lines 1102-1136)
// suffixedexp -> primaryexp { '.' NAME | '[' exp ']' | ':' NAME funcargs | funcargs }
pub fn suffixedexp(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    primaryexp(fs, v)?;

    loop {
        match fs.lexer.current_token() {
            LuaTokenKind::TkDot => {
                // fieldsel
                fieldsel(fs, v)?;
            }
            LuaTokenKind::TkLeftBracket => {
                // [exp]
                let mut key = ExpDesc::new_void();
                code::exp2anyregup(fs, v);
                yindex(fs, &mut key)?;
                code::indexed(fs, v, &mut key);
            }
            LuaTokenKind::TkColon => {
                // : NAME funcargs (method call)
                fs.lexer.bump();
                let method_name = fs.lexer.current_token_text().to_string();
                expect(fs, LuaTokenKind::TkName)?;

                // self:method(...) is sugar for self.method(self, ...)
                // Generate SELF instruction
                let key_idx = string_k(fs, method_name);
                code::self_op(fs, v, key_idx as u8);

                funcargs(fs, v)?;
            }
            LuaTokenKind::TkLeftParen
            | LuaTokenKind::TkString
            | LuaTokenKind::TkLongString
            | LuaTokenKind::TkLeftBrace => {
                // funcargs - must convert to register first
                code::exp2nextreg(fs, v);
                funcargs(fs, v)?;
            }
            _ => {
                return Ok(());
            }
        }
    }
}

// Port of funcargs from lparser.c (lines 1024-1065)
fn funcargs(fs: &mut FuncState, f: &mut ExpDesc) -> Result<(), String> {
    use crate::compiler::expression::ExpKind;

    let mut args = ExpDesc::new_void();

    match fs.lexer.current_token() {
        LuaTokenKind::TkLeftParen => {
            // funcargs -> '(' [ explist ] ')'
            fs.lexer.bump();
            if fs.lexer.current_token() == LuaTokenKind::TkRightParen {
                args.kind = ExpKind::VVOID;
            } else {
                crate::compiler::statement::explist(fs, &mut args)?;
                if matches!(args.kind, ExpKind::VCALL | ExpKind::VVARARG) {
                    code::setmultret(fs, &mut args);
                }
            }
            expect(fs, LuaTokenKind::TkRightParen)?;
        }
        LuaTokenKind::TkLeftBrace => {
            // funcargs -> constructor (table constructor)
            constructor(fs, &mut args)?;
        }
        LuaTokenKind::TkString | LuaTokenKind::TkLongString => {
            // funcargs -> STRING
            let text = fs.lexer.current_token_text();
            let string_content = parse_string_token_value(text, fs.lexer.current_token())?;
            let k_idx = string_k(fs, string_content);
            fs.lexer.bump();
            args = ExpDesc::new_k(k_idx);
        }
        _ => {
            return Err("function arguments expected".to_string());
        }
    }

    // Generate CALL instruction
    if f.kind != ExpKind::VNONRELOC {
        return Err("function must be in register".to_string());
    }

    let base = unsafe { f.u.info as u8 };
    let nparams = if matches!(args.kind, ExpKind::VCALL | ExpKind::VVARARG) {
        0 // LUA_MULTRET represented as 0 in our encoding (will be adjusted in code gen)
    } else {
        if args.kind != ExpKind::VVOID {
            code::exp2nextreg(fs, &mut args);
        }
        fs.freereg - (base + 1)
    };

    let pc = code::code_abc(fs, OpCode::Call, base as u32, (nparams + 1) as u32, 2);
    f.kind = ExpKind::VCALL;
    f.u.info = pc as i32;
    fs.freereg = base + 1; // Call resets freereg to base+1

    Ok(())
}

// Port of yindex from lparser.c
fn yindex(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    fs.lexer.bump(); // skip '['
    expr_internal(fs, v)?;
    code::exp2val(fs, v);
    expect(fs, LuaTokenKind::TkRightBracket)?;
    Ok(())
}

// Port of singlevar from lparser.c (lines 463-474)
pub fn singlevar(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    let name = fs.lexer.current_token_text().to_string();
    fs.lexer.bump();

    let fs_ptr = fs as *mut FuncState;

    // Call singlevaraux with base=1
    singlevaraux(fs_ptr, &name, v, true);

    // If global name (VVOID), access through _ENV
    if v.kind == crate::compiler::expression::ExpKind::VVOID {
        // Get environment variable (_ENV)
        singlevaraux(fs_ptr, "_ENV", v, true);

        // _ENV must exist
        if v.kind == crate::compiler::expression::ExpKind::VVOID {
            return Err("_ENV not found".to_string());
        }

        code::exp2anyregup(fs, v);

        // Key is variable name
        let k_idx = string_k(fs, name);
        let mut key = ExpDesc::new_k(k_idx);

        // env[varname]
        code::indexed(fs, v, &mut key);
    }

    Ok(())
}

// Port of singlevaraux from lparser.c (lines 435-456)
fn singlevaraux(fs: *mut FuncState, name: &str, var: &mut ExpDesc, base: bool) {
    if fs.is_null() {
        init_exp(var, ExpKind::VVOID, 0);
        return;
    }

    let fs_ref = unsafe { &mut *fs };
    let vkind = fs_ref.searchvar(name, var);
    if vkind >= 0 {
        if vkind == ExpKind::VLOCAL as i32 && !base {
            // markupval(fs, var->u.var.vidx);
        }
    } else {
        let vidx = fs_ref.searchupvalue(name);
        if vidx < 0 {
            let prev = fs_ref
                .prev
                .as_ref()
                .map(|p| *p as *const _ as *mut FuncState)
                .unwrap_or(std::ptr::null_mut());
            singlevaraux(prev, name, var, false);
            if var.kind == ExpKind::VLOCAL || var.kind == ExpKind::VUPVAL {
                let idx = fs_ref.newupvalue(name, var) as u8;
                init_exp(var, ExpKind::VUPVAL, idx as i32);
            }
        } else {
            init_exp(var, ExpKind::VUPVAL, vidx as i32);
        }
    }
}

// Port of fieldsel from lparser.c
pub fn fieldsel(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    code::exp2anyreg(fs, v);

    if fs.lexer.current_token() != LuaTokenKind::TkName {
        return Err("expected field name".to_string());
    }

    let field = fs.lexer.current_token_text().to_string();
    fs.lexer.bump();

    let idx = string_k(fs, field);
    *v = ExpDesc::new_indexed(unsafe { v.u.info as u8 }, idx as u8);

    Ok(())
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

                let field_idx = string_k(fs, field_name);
                let mut val = ExpDesc::new_void();
                expr_internal(fs, &mut val)?;

                // t[field] = val  -> SetField instruction
                let val_reg = code::exp2anyreg(fs, &mut val);
                code::code_abc(
                    fs,
                    OpCode::SetField,
                    reg as u32,
                    field_idx as u32,
                    val_reg as u32,
                );
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
                code::code_abc(
                    fs,
                    OpCode::SetTable,
                    reg as u32,
                    key_reg as u32,
                    val_reg as u32,
                );
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
    use crate::compiler::func_state::FuncState;
    use crate::compiler::statement;

    expect(fs, LuaTokenKind::TkLeftParen)?;

    // Determine if vararg before creating child
    let mut is_vararg = false;
    let mut params = Vec::new();

    // Collect parameter names first
    if is_method {
        params.push("self".to_string());
    }

    if fs.lexer.current_token() != LuaTokenKind::TkRightParen {
        loop {
            if fs.lexer.current_token() == LuaTokenKind::TkName {
                let param_name = fs.lexer.current_token_text().to_string();
                fs.lexer.bump();
                params.push(param_name);
            } else if fs.lexer.current_token() == LuaTokenKind::TkDots {
                fs.lexer.bump();
                is_vararg = true;
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

    // Create child FuncState for the function body
    // Use unsafe block to work around borrow checker
    let fs_ptr = fs as *mut FuncState;
    let mut child_fs = unsafe { FuncState::new_child(&mut *fs_ptr, is_vararg) };

    // Add VARARGPREP if vararg
    if is_vararg {
        code::code_abc(&mut child_fs, OpCode::VarargPrep, 0, 0, 0);
    }

    // Register parameters as local variables
    for param in params {
        child_fs.new_localvar(param, VarKind::VDKREG);
    }
    child_fs.adjust_local_vars(child_fs.actvar.len() as u8);

    // Parse body statements
    statement::statlist(&mut child_fs)?;

    expect(&mut child_fs, LuaTokenKind::TkEnd)?;

    // Generate final RETURN
    code::ret(&mut child_fs, 0, 0);

    // Get completed child chunk and upvalue information
    let mut child_chunk = child_fs.chunk;
    let child_upvalues = child_fs.upvalues;

    // Convert upvalues to UpvalueDesc and store in chunk
    // In Lua 5.4, upvalue information is stored in the Proto, not as pseudo-instructions
    for upval in &child_upvalues {
        child_chunk
            .upvalue_descs
            .push(crate::lua_value::UpvalueDesc {
                is_local: upval.in_stack, // true if captures parent local
                index: upval.idx as u32,  // index in parent's register or upvalue array
            });
    }
    child_chunk.upvalue_count = child_upvalues.len();

    // Add child as a prototype in parent
    let proto_idx = fs.chunk.child_protos.len();
    fs.chunk.child_protos.push(std::rc::Rc::new(child_chunk));

    // Generate CLOSURE instruction to create function object
    // CLOSURE A Bx: R[A] := closure(KPROTO[Bx])
    // The upvalue information is already stored in KPROTO[Bx].upvalue_descs
    let reg = fs.freereg;
    code::reserve_regs(fs, 1);
    code::code_abx(fs, OpCode::Closure, reg as u32, proto_idx as u32);

    *v = ExpDesc::new_nonreloc(reg);
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
