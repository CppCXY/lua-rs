// Expression parsing - Port from lparser.c (Lua 5.4.8)
// This file corresponds to expression parsing parts of lua-5.4.8/src/lparser.c
use crate::compiler::expression::{ExpDesc, ExpKind};
use crate::compiler::func_state::FuncState;
use crate::compiler::parse_literal::{
    NumberResult, parse_float_token_value, parse_int_token_value, parse_string_token_value,
};
use crate::compiler::parser::{
    BinaryOperator, LuaTokenKind, UNARY_PRIORITY, UnaryOperator, to_binary_operator,
    to_unary_operator,
};
use crate::compiler::{VarKind, code, string_k};
use crate::lua_vm::OpCode;

// From lopcodes.h - maximum list items per flush
const LFIELDS_PER_FLUSH: u32 = 50;

// Port of init_exp from lparser.c
fn init_exp(e: &mut ExpDesc, kind: ExpKind, info: i32) {
    e.kind = kind;
    e.u.info = info;
    e.t = -1;
    e.f = -1;
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

fn get_unary_opcode(op: UnaryOperator) -> OpCode {
    match op {
        UnaryOperator::OpBNot => OpCode::BNot,
        UnaryOperator::OpNot => OpCode::Not,
        UnaryOperator::OpLen => OpCode::Len,
        UnaryOperator::OpUnm => OpCode::Unm,
        UnaryOperator::OpNop => unreachable!("No opcode for OpNop"),
    }
}

// Port of subexpr from lparser.c
fn subexpr(fs: &mut FuncState, v: &mut ExpDesc, limit: i32) -> Result<(), String> {
    let uop = to_unary_operator(fs.lexer.current_token());
    if uop != UnaryOperator::OpNop {
        let op = get_unary_opcode(uop);
        fs.lexer.bump();
        subexpr(fs, v, UNARY_PRIORITY)?;
        code::prefix(fs, op, v);
    } else {
        simpleexp(fs, v)?;
    }

    // Expand while operators have priorities higher than limit
    // Port of lparser.c:1274-1282
    let mut op = to_binary_operator(fs.lexer.current_token());
    while op != BinaryOperator::OpNop && op.get_priority().left > limit {
        fs.lexer.bump();

        // lcode.c:1637-1676: luaK_infix handles special cases like 'and', 'or'
        code::infix(fs, op, v);

        let mut v2 = ExpDesc::new_void();
        subexpr(fs, &mut v2, op.get_priority().right)?;

        // lcode.c:1706-1783: luaK_posfix
        // 'and' and 'or' don't generate opcodes - they use control flow
        code::posfix(fs, op, v, &mut v2);

        op = to_binary_operator(fs.lexer.current_token());
    }

    Ok(())
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
                    return Err(fs.syntax_error(&format!("invalid integer literal: {}", e)));
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
                    return Err(
                        fs.syntax_error(&format!("invalid float literal '{}': {}", num_text, e))
                    );
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
                    return Err(fs.syntax_error(&format!("invalid string literal: {}", e)));
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
            // lparser.c:1169-1173: vararg
            // Check if inside vararg function
            if !fs.is_vararg {
                return Err(fs.syntax_error("cannot use '...' outside a vararg function"));
            }
            // lparser.c:1173: init_exp(v, VVARARG, luaK_codeABC(fs, OP_VARARG, 0, 0, 1));
            let pc = code::code_abc(fs, OpCode::Vararg, 0, 0, 1);
            *v = ExpDesc::new_void();
            v.kind = ExpKind::VVARARG;
            v.u.info = pc as i32;
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
            return Err(fs.token_error("unexpected symbol"));
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

// Port of fieldsel from lparser.c:811-819
// fieldsel -> ['.' | ':'] NAME
pub fn fieldsel(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    // lparser.c:815: luaK_exp2anyregup(fs, v);
    code::exp2anyregup(fs, v);

    // lparser.c:816: luaX_next(ls);  /* skip the dot or colon */
    fs.lexer.bump();

    // lparser.c:817: codename(ls, &key);
    if fs.lexer.current_token() != LuaTokenKind::TkName {
        return Err(fs.token_error("expected field name"));
    }

    let field = fs.lexer.current_token_text().to_string();
    fs.lexer.bump();

    // lparser.c:818: luaK_indexed(fs, v, &key);
    let idx = string_k(fs, field);
    *v = ExpDesc::new_indexed(unsafe { v.u.info as u8 }, idx as u8);

    Ok(())
}

// Port of ConsControl from lparser.c
struct ConsControl {
    v: ExpDesc,      // last list item read
    table_reg: u8,   // table register
    na: u32,         // number of array elements already stored
    nh: u32,         // total number of record elements
    tostore: u32,    // number of array elements pending to be stored
}

impl ConsControl {
    fn new(table_reg: u8) -> Self {
        Self {
            v: ExpDesc::new_void(),
            table_reg,
            na: 0,
            nh: 0,
            tostore: 0,
        }
    }
}

// Port of closelistfield from lparser.c
fn closelistfield(fs: &mut FuncState, cc: &mut ConsControl) {
    if cc.v.kind == ExpKind::VVOID {
        return; // there is no list item
    }
    code::exp2nextreg(fs, &mut cc.v);
    cc.v.kind = ExpKind::VVOID;
    if cc.tostore == LFIELDS_PER_FLUSH {
        code::setlist(fs, cc.table_reg, cc.na, cc.tostore); // flush
        cc.na += cc.tostore;
        cc.tostore = 0; // no more items pending
    }
}

// Port of lastlistfield from lparser.c
fn lastlistfield(fs: &mut FuncState, cc: &mut ConsControl) {
    if cc.tostore == 0 {
        return;
    }
    if code::hasmultret(&cc.v) {
        code::setmultret(fs, &mut cc.v);
        code::setlist(fs, cc.table_reg, cc.na, code::LUA_MULTRET);
        cc.na -= 1; // do not count last expression (unknown number of elements)
    } else {
        if cc.v.kind != ExpKind::VVOID {
            code::exp2nextreg(fs, &mut cc.v);
        }
        code::setlist(fs, cc.table_reg, cc.na, cc.tostore);
    }
    cc.na += cc.tostore;
}

// Port of listfield from lparser.c
fn listfield(fs: &mut FuncState, cc: &mut ConsControl) -> Result<(), String> {
    expr_internal(fs, &mut cc.v)?;
    cc.tostore += 1;
    Ok(())
}

// Port of field from lparser.c
fn field(fs: &mut FuncState, cc: &mut ConsControl) -> Result<(), String> {
    use crate::compiler::expression::ExpKind;

    if fs.lexer.current_token() == LuaTokenKind::TkLeftBracket {
        // [exp] = exp (general field)
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
            code::code_abc(
                fs,
                OpCode::SetField,
                cc.table_reg as u32,
                key_idx,
                val_reg as u32,
            );
        }
        // Check if key is integer constant for SetI optimization
        else if key.kind == ExpKind::VKINT {
            let key_int = unsafe { key.u.ival as u32 };
            let val_reg = code::exp2anyreg(fs, &mut val);
            code::code_abc(
                fs,
                OpCode::SetI,
                cc.table_reg as u32,
                key_int,
                val_reg as u32,
            );
        } else {
            // General case: SetTable instruction
            let key_reg = code::exp2anyreg(fs, &mut key);
            let val_reg = code::exp2anyreg(fs, &mut val);
            code::code_abc(
                fs,
                OpCode::SetTable,
                cc.table_reg as u32,
                key_reg as u32,
                val_reg as u32,
            );
        }
        cc.nh += 1;
    } else if fs.lexer.current_token() == LuaTokenKind::TkName {
        // Check if it's name = exp (record field) or just a list item
        let next = fs.lexer.peek_next_token();
        if next == LuaTokenKind::TkAssign {
            // name = exp (record field)
            let field_name = fs.lexer.current_token_text().to_string();
            fs.lexer.bump();
            fs.lexer.bump(); // skip =

            let field_idx = string_k(fs, field_name);
            let mut val = ExpDesc::new_void();
            expr_internal(fs, &mut val)?;

            // t[field] = val -> SetField instruction
            let val_reg = code::exp2anyreg(fs, &mut val);
            code::code_abc(
                fs,
                OpCode::SetField,
                cc.table_reg as u32,
                field_idx as u32,
                val_reg as u32,
            );
            cc.nh += 1;
        } else {
            // Just a list item
            listfield(fs, cc)?;
        }
    } else {
        // List item
        listfield(fs, cc)?;
    }

    Ok(())
}

// Port of constructor from lparser.c
fn constructor(fs: &mut FuncState, v: &mut ExpDesc) -> Result<(), String> {
    expect(fs, LuaTokenKind::TkLeftBrace)?;

    let table_reg = fs.freereg;
    code::reserve_regs(fs, 1);
    let pc = code::code_abc(fs, OpCode::NewTable, table_reg as u32, 0, 0);
    code::code_extraarg(fs, 0); // space for extra arg
    *v = ExpDesc::new_nonreloc(table_reg);

    let mut cc = ConsControl::new(table_reg);

    // Parse table fields
    loop {
        if fs.lexer.current_token() == LuaTokenKind::TkRightBrace {
            break;
        }
        closelistfield(fs, &mut cc);
        field(fs, &mut cc)?;

        if !matches!(
            fs.lexer.current_token(),
            LuaTokenKind::TkComma | LuaTokenKind::TkSemicolon
        ) {
            break;
        }
        fs.lexer.bump();
    }

    expect(fs, LuaTokenKind::TkRightBrace)?;
    lastlistfield(fs, &mut cc);
    code::settablesize(fs, pc, table_reg, cc.na, cc.nh);
    Ok(())
}

// Port of body from lparser.c
pub fn body(fs: &mut FuncState, v: &mut ExpDesc, is_method: bool) -> Result<(), String> {
    use crate::compiler::func_state::FuncState;
    use crate::compiler::statement;

    // Record the line where function is defined
    let linedefined = fs.lexer.line;

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

    // Port of body function from lparser.c:989-1008
    // body ->  '(' parlist ')' block END

    // lparser.c:993: Create new FuncState for nested function
    // FuncState new_fs; new_fs.f = addprototype(ls); open_func(ls, &new_fs, &bl);
    let fs_ptr = fs as *mut FuncState;
    let mut child_fs = unsafe { FuncState::new_child(&mut *fs_ptr, is_vararg) };

    // lparser.c:994: If vararg, generate VARARGPREP instruction
    if is_vararg {
        code::code_abc(&mut child_fs, OpCode::VarargPrep, 0, 0, 0);
    }

    // lparser.c:996-999: Register parameters as local variables
    // parlist(ls); adjustlocalvars(ls, nparams);
    for param in params {
        child_fs.new_localvar(param, VarKind::VDKREG);
    }
    child_fs.adjust_local_vars(child_fs.actvar.len() as u8);
    
    // lparser.c:982: Set numparams after adjustlocalvars
    // f->numparams = cast_byte(fs->nactvar);
    let param_count = child_fs.nactvar as usize;

    // lparser.c:1002: Parse function body statements
    // statlist(ls);
    statement::statlist(&mut child_fs)?;

    // Record the line where function ends (before consuming END token)
    let lastlinedefined = child_fs.lexer.line;

    // lparser.c:1004: Expect END token
    expect(&mut child_fs, LuaTokenKind::TkEnd)?;

    // Generate final RETURN instruction
    code::ret(&mut child_fs, 0, 0);

    // Get completed child chunk and upvalue information
    let mut child_chunk = child_fs.chunk;
    child_chunk.is_vararg = child_fs.is_vararg; // Set vararg flag on chunk
    // param_count excludes ... (vararg), only counts regular parameters
    child_chunk.param_count = param_count;
    child_chunk.linedefined = linedefined;
    child_chunk.lastlinedefined = lastlinedefined;
    child_chunk.source_name = Some(child_fs.source_name.clone());
    let child_upvalues = child_fs.upvalues;

    // Port of lparser.c:722-726 (codeclosure)
    // In Lua 5.4, upvalue information is stored in Proto.upvalues[], NOT as pseudo-instructions
    // This is different from Lua 5.1 which used pseudo-instructions after OP_CLOSURE
    for upval in &child_upvalues {
        child_chunk
            .upvalue_descs
            .push(crate::lua_value::UpvalueDesc {
                is_local: upval.in_stack, // true if captures parent local
                index: upval.idx as u32,  // index in parent's register or upvalue array
            });
    }
    child_chunk.upvalue_count = child_upvalues.len();

    // lparser.c:1005: Add child proto to parent (addprototype)
    let proto_idx = fs.chunk.child_protos.len();
    fs.chunk.child_protos.push(std::rc::Rc::new(child_chunk));

    // lparser.c:722-726: Generate CLOSURE instruction (codeclosure)
    // static void codeclosure (LexState *ls, expdesc *v) {
    //   FuncState *fs = ls->fs->prev;
    //   init_exp(v, VRELOC, luaK_codeABx(fs, OP_CLOSURE, 0, fs->np - 1));
    //   luaK_exp2nextreg(fs, v);  /* fix it at the last register */
    // }
    let pc = code::code_abx(fs, OpCode::Closure, 0, proto_idx as u32);
    *v = ExpDesc::new_reloc(pc);
    code::exp2nextreg(fs, v);
    Ok(())
}

// Helper: expect a token
fn expect(fs: &mut FuncState, tk: LuaTokenKind) -> Result<(), String> {
    if fs.lexer.current_token() == tk {
        fs.lexer.bump();
        Ok(())
    } else {
        Err(fs.token_error(&format!("expected '{:?}'", tk)))
    }
}
