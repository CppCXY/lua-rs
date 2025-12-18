use crate::compiler::expr_parser::{body, expr, suffixedexp};
// Statement parsing - Port from lparser.c (Lua 5.4.8)
// This file corresponds to statement parsing parts of lua-5.4.8/src/lparser.c
use crate::Instruction;
use crate::compiler::expression::ExpDesc;
use crate::compiler::func_state::{BlockCnt, FuncState, LabelDesc};
use crate::compiler::parser::LuaTokenKind;
use crate::compiler::{ExpKind, VarKind, code};
use crate::lua_vm::OpCode;

// Port of statlist from lparser.c:1529-1536
// static void statlist (LexState *ls)
pub fn statlist(fs: &mut FuncState) -> Result<(), String> {
    // statlist -> { stat [';'] }
    while !block_follow(fs, true) {
        if fs.lexer.current_token() == LuaTokenKind::TkReturn {
            statement(fs)?;
            return Ok(()); // 'return' must be last statement
        }
        statement(fs)?;
    }
    Ok(())
}

// Port of block_follow from lparser.c:1504-1510
// static int block_follow (LexState *ls, int withuntil)
fn block_follow(fs: &FuncState, withuntil: bool) -> bool {
    match fs.lexer.current_token() {
        LuaTokenKind::TkElse
        | LuaTokenKind::TkElseIf
        | LuaTokenKind::TkEnd
        | LuaTokenKind::TkEof => true,
        LuaTokenKind::TkUntil => withuntil,
        _ => false,
    }
}

// Port of statement from lparser.c
fn statement(fs: &mut FuncState) -> Result<(), String> {
    let line = fs.lexer.line;
    // enterlevel(fs.lexer);

    match fs.lexer.current_token() {
        LuaTokenKind::TkSemicolon => {
            // Empty statement
            fs.lexer.bump();
        }
        LuaTokenKind::TkIf => {
            ifstat(fs, line)?;
        }
        LuaTokenKind::TkWhile => {
            whilestat(fs, line)?;
        }
        LuaTokenKind::TkDo => {
            fs.lexer.bump(); // skip DO
            block(fs)?;
            check_match(fs, LuaTokenKind::TkEnd, LuaTokenKind::TkDo, line)?;
        }
        LuaTokenKind::TkFor => {
            forstat(fs, line)?;
        }
        LuaTokenKind::TkRepeat => {
            repeatstat(fs, line)?;
        }
        LuaTokenKind::TkFunction => {
            funcstat(fs, line)?;
        }
        LuaTokenKind::TkLocal => {
            fs.lexer.bump(); // skip LOCAL
            if testnext(fs, LuaTokenKind::TkFunction) {
                localfunc(fs)?;
            } else {
                localstat(fs)?;
            }
        }
        LuaTokenKind::TkDbColon => {
            fs.lexer.bump(); // skip ::
            labelstat(fs)?;
        }
        LuaTokenKind::TkReturn => {
            fs.lexer.bump(); // skip RETURN
            retstat(fs)?;
        }
        LuaTokenKind::TkBreak => {
            fs.lexer.bump(); // skip BREAK
            breakstat(fs)?;
        }
        LuaTokenKind::TkGoto => {
            fs.lexer.bump(); // skip GOTO
            gotostat(fs)?;
        }
        _ => {
            exprstat(fs)?;
        }
    }

    // leavelevel(fs.lexer);
    Ok(())
}

// Port of testnext from lparser.c
fn testnext(fs: &mut FuncState, expected: LuaTokenKind) -> bool {
    if fs.lexer.current_token() == expected {
        fs.lexer.bump();
        true
    } else {
        false
    }
}

// Port of check from lparser.c
fn check(fs: &mut FuncState, expected: LuaTokenKind) -> Result<(), String> {
    if fs.lexer.current_token() != expected {
        return error_expected(fs, expected);
    }
    Ok(())
}

// Port of error_expected from lparser.c
fn error_expected(fs: &mut FuncState, token: LuaTokenKind) -> Result<(), String> {
    Err(format!(
        "{}:{}: syntax error: expected '{}', got '{}'",
        fs.source_name,
        fs.lexer.line,
        token,
        fs.lexer.current_token()
    ))
}

fn check_match(
    fs: &mut FuncState,
    what: LuaTokenKind,
    who: LuaTokenKind,
    where_: usize,
) -> Result<(), String> {
    if !testnext(fs, what) {
        if where_ == fs.lexer.line {
            error_expected(fs, what)?;
        } else {
            return Err(format!(
                "{}:{}: syntax error: expected '{}' (to close '{}' at line {})",
                fs.source_name, fs.lexer.line, what, who, where_
            ));
        }
    }
    Ok(())
}

// Port of str_checkname from lparser.c
fn str_checkname(fs: &mut FuncState) -> Result<String, String> {
    check(fs, LuaTokenKind::TkName)?;
    let name = fs.lexer.current_token_text().to_string();
    fs.lexer.bump();
    Ok(name)
}

// Port of enterblock from lparser.c
fn enterblock(fs: &mut FuncState, bl: &mut BlockCnt, isloop: bool) {
    bl.nactvar = fs.nactvar;
    bl.first_label = fs.labels.len();
    bl.first_goto = fs.pending_gotos.len();
    bl.upval = false;
    bl.is_loop = isloop;
    bl.in_scope = true;
    bl.previous = fs.block_list.take();
    fs.block_list = Some(Box::new(BlockCnt {
        previous: bl.previous.take(),
        first_label: bl.first_label,
        first_goto: bl.first_goto,
        nactvar: bl.nactvar,
        upval: bl.upval,
        is_loop: bl.is_loop,
        in_scope: bl.in_scope,
    }));
}

// Port of leaveblock from lparser.c
fn leaveblock(fs: &mut FuncState) {
    if let Some(bl) = fs.block_list.take() {
        fs.block_list = bl.previous;
        // Remove labels and gotos from this block
        fs.labels.truncate(bl.first_label);
        fs.nactvar = bl.nactvar;
    }
}

// Port of block from lparser.c
fn block(fs: &mut FuncState) -> Result<(), String> {
    let mut bl = BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: 0,
        upval: false,
        is_loop: false,
        in_scope: true,
    };
    enterblock(fs, &mut bl, false);
    statlist(fs)?;
    leaveblock(fs);
    Ok(())
}

// Port of retstat from lparser.c:1812-1843
// static void retstat (LexState *ls)
// stat -> RETURN [explist] [';']
fn retstat(fs: &mut FuncState) -> Result<(), String> {
    use crate::compiler::expression::ExpKind;
    let mut first = fs.freereg;
    let mut nret: i32;
    let mut e = ExpDesc::new_void();

    if block_follow(fs, true) || fs.lexer.current_token() == LuaTokenKind::TkSemicolon {
        // return with no values
        nret = 0;
    } else {
        nret = explist(fs, &mut e)? as i32;
        // Check if expression has multiple returns (VCALL or VVARARG)
        if matches!(e.kind, ExpKind::VCALL | ExpKind::VVARARG) {
            code::setmultret(fs, &mut e);
            if e.kind == ExpKind::VCALL && nret == 1 {
                // Tail call optimization
                let pc = unsafe { e.u.info as usize };
                if pc < fs.chunk.code.len() {
                    let instr = &mut fs.chunk.code[pc];
                    *instr = (*instr & !0x7F) | (OpCode::TailCall as u32);
                }
            }
            nret = -1; // LUA_MULTRET
        } else {
            if nret == 1 {
                // Only one single value - can use original slot
                first = code::exp2anyreg(fs, &mut e);
            } else {
                // Values must go to the top of the stack
                code::exp2nextreg(fs, &mut e);
                // nret == fs->freereg - first
            }
        }
    }

    code::ret(fs, first, nret as u8);
    testnext(fs, LuaTokenKind::TkSemicolon);
    Ok(())
}

// Port of explist from lparser.c
// static int explist (LexState *ls, expdesc *v) {
//   int n = 1;
//   expr(ls, v);
//   while (testnext(ls, ',')) {
//     luaK_exp2nextreg(ls->fs, v);
//     expr(ls, v);
//     n++;
//   }
//   return n;
// }
pub fn explist(fs: &mut FuncState, e: &mut ExpDesc) -> Result<usize, String> {
    use crate::compiler::expr_parser::expr_internal;
    let mut n = 1;
    expr_internal(fs, e)?;
    while testnext(fs, LuaTokenKind::TkComma) {
        code::exp2nextreg(fs, e);
        expr_internal(fs, e)?;
        n += 1;
    }
    Ok(n)
}

// Port of breakstat from lparser.c
fn breakstat(fs: &mut FuncState) -> Result<(), String> {
    // Check if we're inside a loop
    let mut bl = fs.block_list.as_ref();
    let mut _upval = false;

    while let Some(block) = bl {
        if block.is_loop {
            break;
        }
        _upval = _upval || block.upval;
        bl = block.previous.as_ref();
    }

    if bl.is_none() {
        return Err(format!(
            "{}:{}: no loop to break",
            "<source>", fs.lexer.line
        ));
    }

    // Generate break jump - will be patched later
    let jmp = code::jump(fs);

    // Add to pending breaks list
    let label = LabelDesc {
        name: "break".to_string(),
        pc: jmp,
        line: fs.lexer.line,
        nactvar: fs.nactvar,
        close: false,
    };
    fs.pending_gotos.push(label);

    Ok(())
}

// Port of gotostat from lparser.c
fn gotostat(fs: &mut FuncState) -> Result<(), String> {
    let name = str_checkname(fs)?;
    let label = LabelDesc {
        name,
        pc: fs.pc,
        line: fs.lexer.line,
        nactvar: fs.nactvar,
        close: false,
    };
    fs.pending_gotos.push(label);
    Ok(())
}

// Port of labelstat from lparser.c
fn labelstat(fs: &mut FuncState) -> Result<(), String> {
    let name = str_checkname(fs)?;
    check(fs, LuaTokenKind::TkDbColon)?;
    fs.lexer.bump();  // skip '::'

    let label = LabelDesc {
        name,
        pc: fs.pc,
        line: fs.lexer.line,
        nactvar: fs.nactvar,
        close: false,
    };
    fs.labels.push(label);
    Ok(())
}

// Port of test_then_block from lparser.c:1635-1668
fn test_then_block(fs: &mut FuncState, escapelist: &mut isize) -> Result<(), String> {
    // test_then_block -> [IF | ELSEIF] cond THEN block
    fs.lexer.bump(); // skip IF or ELSEIF
    
    // lparser.c:1642: expr(ls, &v);
    let mut v = expr(fs)?;

    // lparser.c:1643: checknext(ls, TK_THEN);
    check(fs, LuaTokenKind::TkThen)?;
    fs.lexer.bump(); // consume THEN

    // Regular case (not a break)
    // lparser.c:1657: luaK_goiftrue(ls->fs, &v); /* skip over block if condition is false */
    code::goiftrue(fs, &mut v);

    // lparser.c:1659: jf = v.f;
    let jf = v.f;

    // lparser.c:1661: statlist(ls); /* 'then' part */
    block(fs)?;

    // lparser.c:1663-1665: Jump to end after then block if followed by else/elseif
    if fs.lexer.current_token() == LuaTokenKind::TkElseIf
        || fs.lexer.current_token() == LuaTokenKind::TkElse
    {
        let jmp = code::jump(fs) as isize;
        code::concat(fs, escapelist, jmp);
    }

    // lparser.c:1666: luaK_patchtohere(fs, jf);
    code::patchtohere(fs, jf);

    Ok(())
}

// Port of ifstat from lparser.c
fn ifstat(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // ifstat -> IF cond THEN block {ELSEIF cond THEN block} [ELSE block] END
    let mut escapelist: isize = -1;
    test_then_block(fs, &mut escapelist)?;

    while fs.lexer.current_token() == LuaTokenKind::TkElseIf {
        test_then_block(fs, &mut escapelist)?;
    }

    if testnext(fs, LuaTokenKind::TkElse) {
        block(fs)?;
    }

    check_match(fs, LuaTokenKind::TkEnd, LuaTokenKind::TkIf, line)?;

    // Patch all escape jumps to here
    code::patchtohere(fs, escapelist);

    Ok(())
}

// Port of whilestat from lparser.c
fn whilestat(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // whilestat -> WHILE cond DO block END
    fs.lexer.bump(); // skip WHILE
    let whileinit = fs.pc;

    // Parse condition
    let mut v = expr(fs)?;
    code::exp2nextreg(fs, &mut v);

    // Jump out if condition is false
    let condexit = code::jump(fs);

    let mut bl = BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: 0,
        upval: false,
        is_loop: true,
        in_scope: true,
    };
    enterblock(fs, &mut bl, true);
    check(fs, LuaTokenKind::TkDo)?;
    fs.lexer.bump();  // skip 'do'
    block(fs)?;

    // Jump back to condition
    code::code_asbx(
        fs,
        OpCode::Jmp,
        0,
        (whileinit as isize - fs.pc as isize - 1) as i32,
    );

    check_match(fs, LuaTokenKind::TkEnd, LuaTokenKind::TkWhile, line)?;
    leaveblock(fs);

    // Patch exit jump
    code::patchtohere(fs, condexit as isize);

    Ok(())
}

// Port of repeatstat from lparser.c
fn repeatstat(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // repeatstat -> REPEAT block UNTIL cond
    fs.lexer.bump(); // skip REPEAT
    let repeat_init = fs.pc;

    let mut bl = BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: 0,
        upval: false,
        is_loop: true,
        in_scope: true,
    };
    enterblock(fs, &mut bl, true);
    block(fs)?;
    check_match(fs, LuaTokenKind::TkUntil, LuaTokenKind::TkRepeat, line)?;

    // Parse until condition
    let mut v = expr(fs)?;
    code::exp2nextreg(fs, &mut v);

    // Jump back if condition is false
    code::code_asbx(
        fs,
        OpCode::Jmp,
        0,
        (repeat_init as isize - fs.pc as isize - 1) as i32,
    );

    leaveblock(fs);
    Ok(())
}

// Port of forstat from lparser.c
fn forstat(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // forstat -> FOR (fornum | forlist) END
    fs.lexer.bump(); // skip FOR
    let varname = str_checkname(fs)?;

    match fs.lexer.current_token() {
        LuaTokenKind::TkAssign => {
            // Numeric for
            fornum(fs, varname, line)?;
        }
        LuaTokenKind::TkComma | LuaTokenKind::TkIn => {
            // Generic for
            forlist(fs, varname)?;
        }
        _ => {
            return Err(format!(
                "{}:{}: '=' or 'in' expected",
                fs.source_name, fs.lexer.line
            ));
        }
    }

    check_match(fs, LuaTokenKind::TkEnd, LuaTokenKind::TkFor, line)?;
    Ok(())
}

fn fornum(fs: &mut FuncState, varname: String, _line: usize) -> Result<(), String> {
    // fornum -> NAME = exp, exp [,exp] forbody
    // Port of lparser.c:1568-1590
    
    // Reserve registers for internal loop variables (must be done before parsing expressions)
    let base = fs.freereg;
    
    // Create 3 internal control variables: (for state), (for state), (for state)
    fs.new_localvar("(for state)".to_string(), crate::compiler::func_state::VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), crate::compiler::func_state::VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), crate::compiler::func_state::VarKind::VDKREG);
    
    // Create the loop variable
    fs.new_localvar(varname, crate::compiler::func_state::VarKind::VDKREG);
    
    check(fs, LuaTokenKind::TkAssign)?;  // check '='
    fs.lexer.bump();  // skip '='

    // Parse initial, limit, step (exp1 = expr + exp2nextreg)
    let mut e = expr(fs)?;
    code::exp2nextreg(fs, &mut e);
    check(fs, LuaTokenKind::TkComma)?;
    fs.lexer.bump();  // skip ','
    
    let mut e = expr(fs)?;
    code::exp2nextreg(fs, &mut e);

    if testnext(fs, LuaTokenKind::TkComma) {
        let mut e = expr(fs)?;
        code::exp2nextreg(fs, &mut e);
    } else {
        // Default step = 1
        code::code_asbx(fs, OpCode::LoadI, fs.freereg as u32, 1);
        code::reserve_regs(fs, 1);
    }

    // Adjust local variables (3 control variables)
    fs.adjust_local_vars(3);

    // Generate FORPREP
    let prep_jump = code::code_asbx(fs, OpCode::ForPrep, base as u32, 0);

    check(fs, LuaTokenKind::TkDo)?;
    fs.lexer.bump();  // skip 'do'

    let mut bl = BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: fs.nactvar,
        upval: false,
        is_loop: true,
        in_scope: true,
    };
    enterblock(fs, &mut bl, true);

    block(fs)?;

    leaveblock(fs);

    // Generate FORLOOP
    code::patchtohere(fs, prep_jump as isize);
    let loop_pc = code::code_asbx(
        fs,
        OpCode::ForLoop,
        base as u32,
        (prep_jump as isize - fs.pc as isize) as i32,
    );
    code::fix_jump(fs, prep_jump, loop_pc + 1);

    fs.remove_vars(fs.nactvar - 1);

    Ok(())
}

fn forlist(fs: &mut FuncState, indexname: String) -> Result<(), String> {
    // forlist -> NAME {,NAME} IN explist forbody
    // Port of lparser.c:1591-1616
    let mut nvars = 5;  // gen, state, control, toclose, 'indexname'
    
    let base = fs.freereg;
    
    // Create 4 internal control variables
    fs.new_localvar("(for state)".to_string(), crate::compiler::func_state::VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), crate::compiler::func_state::VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), crate::compiler::func_state::VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), crate::compiler::func_state::VarKind::VDKREG);
    
    // Create declared variables (starting with indexname)
    fs.new_localvar(indexname, crate::compiler::func_state::VarKind::VDKREG);
    
    while testnext(fs, LuaTokenKind::TkComma) {
        let varname = str_checkname(fs)?;
        fs.new_localvar(varname, crate::compiler::func_state::VarKind::VDKREG);
        nvars += 1;
    }

    check(fs, LuaTokenKind::TkIn)?;
    fs.lexer.bump();  // skip IN

    // Parse iterator expressions
    let mut e = ExpDesc::new_void();
    let nexps = explist(fs, &mut e)?;

    // Adjust to 4 values (generator, state, control, toclose)
    adjust_assign(fs, 4, nexps, &mut e);

    // Activate the 4 control variables
    fs.adjust_local_vars(4);

    check(fs, LuaTokenKind::TkDo)?;
    fs.lexer.bump();  // skip 'do'

    let loop_start = fs.pc;

    // Generate TFORPREP (Lua 5.4)
    code::code_abx(fs, OpCode::TForPrep, base as u32, 0);

    let mut bl = BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: fs.nactvar,
        upval: false,
        is_loop: true,
        in_scope: true,
    };
    enterblock(fs, &mut bl, true);

    block(fs)?;

    leaveblock(fs);

    // Generate TFORLOOP
    code::code_asbx(
        fs,
        OpCode::TForLoop,
        base as u32,
        (loop_start as isize - fs.pc as isize - 1) as i32,
    );

    fs.remove_vars(fs.nactvar - nvars as u8);

    Ok(())
}

// Port of funcstat from lparser.c
fn funcstat(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // funcstat -> FUNCTION funcname body
    fs.lexer.bump(); // skip FUNCTION

    // funcname -> NAME {fieldsel} [':' NAME]
    // Parse first name (base variable)
    let mut base = ExpDesc::new_void();
    crate::compiler::expr_parser::singlevar(fs, &mut base)?;
    
    // Handle field selections: t.a.b.c
    while fs.lexer.current_token() == LuaTokenKind::TkDot {
        fs.lexer.bump();
        crate::compiler::expr_parser::fieldsel(fs, &mut base)?;
    }

    // Handle method definition: function t:method()
    let is_method = testnext(fs, LuaTokenKind::TkColon);
    if is_method {
        crate::compiler::expr_parser::fieldsel(fs, &mut base)?;
    }

    // Parse function body
    let mut func_val = ExpDesc::new_void();
    crate::compiler::expr_parser::body(fs, &mut func_val, is_method)?;

    // Check if variable is readonly
    check_readonly(fs, &base)?;
    
    // Store function: base = func_val
    storevar(fs, &base, &mut func_val);
    
    // Fix line information: definition happens in the first line
    code::fixline(fs, line);

    Ok(())
}

// Port of localfunc from lparser.c
fn localfunc(fs: &mut FuncState) -> Result<(), String> {
    // localfunc -> NAME body
    let name = str_checkname(fs)?;

    // Register local variable
    fs.new_localvar(name, crate::compiler::func_state::VarKind::VDKREG);
    fs.adjust_local_vars(1);

    // Parse function body
    let mut v = ExpDesc::new_void();
    body(fs, &mut v, false)?;

    Ok(())
}

// Port of getlocalattribute from lparser.c
fn get_local_attribute(fs: &mut FuncState) -> Result<VarKind, String> {
    // ATTRIB -> ['<' Name '>']
    if testnext(fs, LuaTokenKind::TkLt) {
        if fs.lexer.current_token() != LuaTokenKind::TkName {
            return Err("expected attribute name".to_string());
        }

        let attr = fs.lexer.current_token_text().to_string();
        fs.lexer.bump();

        check(fs, LuaTokenKind::TkGt)?;
        fs.lexer.bump();  // skip '>'

        if attr == "const" {
            Ok(VarKind::RDKCONST)
        } else if attr == "close" {
            Ok(VarKind::RDKTOCLOSE)
        } else {
            Err(format!("unknown attribute '{}'", attr))
        }
    } else {
        Ok(VarKind::VDKREG) // regular variable
    }
}

// Port of checktoclose from lparser.c
fn check_to_close(fs: &mut FuncState, level: isize) {
    if level != -1 {
        // Mark that this function has to-be-closed variables
        fs.needclose = true;
        // Generate TBC instruction
        code::code_abc(fs, OpCode::Tbc, level as u32, 0, 0);
    }
}

// Port of localstat from lparser.c
// Port of localstat from lparser.c - line 1725
// static void localstat (LexState *ls) {
//   FuncState *fs = ls->fs;
//   int toclose = -1;
//   Vardesc *var;
//   int vidx, kind;
//   int nvars = 0;
//   int nexps;
//   expdesc e;
//   do {
//     vidx = new_localvar(ls, str_checkname(ls));
//     kind = getlocalattribute(ls);
//     getlocalvardesc(fs, vidx)->vd.kind = kind;
//     if (kind == RDKTOCLOSE) {
//       if (toclose != -1)
//         luaK_semerror(ls, "multiple to-be-closed variables in local list");
//       toclose = fs->nactvar + nvars;
//     }
//     nvars++;
//   } while (testnext(ls, ','));
//   if (testnext(ls, '='))
//     nexps = explist(ls, &e);
//   else {
//     e.k = VVOID;
//     nexps = 0;
//   }
//   var = getlocalvardesc(fs, vidx);
//   if (nvars == nexps &&
//       var->vd.kind == RDKCONST &&
//       luaK_exp2const(fs, &e, &var->k)) {
//     var->vd.kind = RDKCTC;
//     adjustlocalvars(ls, nvars - 1);
//     fs->nactvar++;
//   }
//   else {
//     adjust_assign(ls, nvars, nexps, &e);
//     adjustlocalvars(ls, nvars);
//   }
//   checktoclose(fs, toclose);
// }
fn localstat(fs: &mut FuncState) -> Result<(), String> {
    use crate::compiler::func_state::VarKind;

    let mut toclose: isize = -1;
    let mut nvars = 0;
    let mut e = ExpDesc::new_void();

    // Parse variable list
    loop {
        let name = str_checkname(fs)?;
        let vidx = fs.new_localvar(name, VarKind::VDKREG);
        let kind = get_local_attribute(fs)?;
        
        // Set kind for this variable
        if let Some(var) = fs.get_local_var_desc(vidx) {
            var.kind = kind;
        }

        if kind == VarKind::RDKTOCLOSE {
            if toclose != -1 {
                return Err("multiple to-be-closed variables in local list".to_string());
            }
            toclose = (fs.nactvar + nvars) as isize;
        }

        nvars += 1;

        if !testnext(fs, LuaTokenKind::TkComma) {
            break;
        }
    }

    // Parse optional initialization
    let nexps = if testnext(fs, LuaTokenKind::TkAssign) {
        explist(fs, &mut e)?
    } else {
        e.kind = crate::compiler::expression::ExpKind::VVOID;
        0
    };

    // Check for compile-time constant optimization
    // Get last variable
    let last_vidx = (fs.nactvar + nvars - 1) as u16;
    let is_const_opt = if let Some(var_desc) = fs.get_local_var_desc(last_vidx) {
        nvars as usize == nexps && 
        var_desc.kind == VarKind::RDKCONST &&
        code::exp2const(fs, &e).is_some()
    } else {
        false
    };
    
    if is_const_opt {
        // Variable is a compile-time constant
        if let Some(var_desc) = fs.get_local_var_desc(last_vidx) {
            var_desc.kind = VarKind::RDKCTC;
        }
        fs.adjust_local_vars(nvars - 1);  // exclude last variable
        fs.nactvar += 1;  // but count it
        check_to_close(fs, toclose);
        return Ok(());
    }
    
    adjust_assign(fs, nvars as usize, nexps, &mut e);
    fs.adjust_local_vars(nvars);

    check_to_close(fs, toclose);

    Ok(())
}

// Port of LHS_assign from lparser.c
struct LhsAssign {
    prev: Option<Box<LhsAssign>>,
    v: ExpDesc,
}

// Port of vkisvar from lparser.c
fn vkisvar(k: crate::compiler::expression::ExpKind) -> bool {
    use crate::compiler::expression::ExpKind;
    matches!(
        k,
        ExpKind::VLOCAL
            | ExpKind::VUPVAL
            | ExpKind::VINDEXED
            | ExpKind::VINDEXUP
            | ExpKind::VINDEXI
            | ExpKind::VINDEXSTR
    )
}

// Port of vkisindexed from lparser.c
fn vkisindexed(k: crate::compiler::expression::ExpKind) -> bool {
    use crate::compiler::expression::ExpKind;
    matches!(
        k,
        ExpKind::VINDEXED | ExpKind::VINDEXUP | ExpKind::VINDEXI | ExpKind::VINDEXSTR
    )
}

// Port of check_conflict from lparser.c
fn check_conflict(fs: &mut FuncState, lh: &LhsAssign, v: &ExpDesc) {
    use crate::compiler::expression::ExpKind;

    let mut conflict = false;
    let extra = fs.freereg;

    // Check all variables in the chain
    let mut current = Some(lh);
    while let Some(node) = current {
        if vkisindexed(node.v.kind) {
            // If this is indexed and new var is local/upvalue
            if v.kind == ExpKind::VLOCAL || v.kind == ExpKind::VUPVAL {
                // Check if they might conflict
                conflict = true;
                break;
            }
        }
        current = node.prev.as_ref().map(|b| b.as_ref());
    }

    if conflict {
        // Copy local/upvalue to temporary
        if v.kind == ExpKind::VLOCAL {
            code::code_abc(
                fs,
                OpCode::Move,
                extra as u32,
                unsafe { v.u.var.ridx as u32 },
                0,
            );
        } else if v.kind == ExpKind::VUPVAL {
            code::code_abc(
                fs,
                OpCode::GetUpval,
                extra as u32,
                unsafe { v.u.info as u32 },
                0,
            );
        }
        code::reserve_regs(fs, 1);
    }
}

// Port of adjust_assign from lparser.c
fn adjust_assign(fs: &mut FuncState, nvars: usize, nexps: usize, e: &mut ExpDesc) {
    use crate::compiler::expression::ExpKind;

    let needed = nvars as isize - nexps as isize;

    // Check if last expression has multiple returns
    if matches!(e.kind, ExpKind::VCALL | ExpKind::VVARARG) {
        let extra = needed + 1;
        if extra < 0 {
            // Too many expressions, adjust call to return fewer
            code::setoneret(fs, e);
        } else {
            // Adjust to return the right number
            code::setreturns(fs, e, extra as u8);
        }
    } else {
        if e.kind != ExpKind::VVOID {
            code::exp2nextreg(fs, e);
        }
        if needed > 0 {
            code::nil(fs, fs.freereg, needed as u8);
        }
    }

    if needed > 0 {
        code::reserve_regs(fs, needed as u8);
    } else {
        // Remove extra registers
        fs.freereg = (fs.freereg as isize + needed) as u8;
    }
}

// Port of luaK_storevar from lcode.c
fn storevar(fs: &mut FuncState, var: &ExpDesc, ex: &mut ExpDesc) {
    use crate::compiler::expression::ExpKind;

    match var.kind {
        ExpKind::VLOCAL => {
            code::free_exp(fs, ex);
            code::exp2reg(fs, ex, unsafe { var.u.var.ridx as u8 });
        }
        ExpKind::VUPVAL => {
            let e = code::exp2anyreg(fs, ex);
            code::code_abc(
                fs,
                OpCode::SetUpval,
                e as u32,
                unsafe { var.u.info as u32 },
                0,
            );
        }
        ExpKind::VINDEXED => {
            let op = OpCode::SetTable;
            let e = code::exp2anyreg(fs, ex);
            code::code_abc(
                fs,
                op,
                unsafe { var.u.ind.t as u32 },
                unsafe { var.u.ind.idx as u32 },
                e as u32,
            );
        }
        ExpKind::VINDEXUP => {
            let e = code::exp2anyreg(fs, ex);
            code::code_abc(
                fs,
                OpCode::SetTabUp,
                unsafe { var.u.ind.t as u32 },
                unsafe { var.u.ind.idx as u32 },
                e as u32,
            );
        }
        ExpKind::VINDEXI => {
            let e = code::exp2anyreg(fs, ex);
            code::code_abc(
                fs,
                OpCode::SetI,
                unsafe { var.u.ind.t as u32 },
                unsafe { var.u.ind.idx as u32 },
                e as u32,
            );
        }
        ExpKind::VINDEXSTR => {
            let e = code::exp2anyreg(fs, ex);
            code::code_abc(
                fs,
                OpCode::SetField,
                unsafe { var.u.ind.t as u32 },
                unsafe { var.u.ind.idx as u32 },
                e as u32,
            );
        }
        _ => {
            // Should not happen
        }
    }
    code::free_exp(fs, ex);
}

// Port of restassign from lparser.c
fn restassign(fs: &mut FuncState, lh: &mut LhsAssign, nvars: usize) -> Result<(), String> {
    let mut e = ExpDesc::new_void();

    if !vkisvar(lh.v.kind) {
        return Err("syntax error".to_string());
    }

    // Check if variable is readonly (const)
    check_readonly(fs, &lh.v)?;

    if testnext(fs, LuaTokenKind::TkComma) {
        // restassign -> ',' suffixedexp restassign
        let mut nv = LhsAssign {
            prev: None,
            v: ExpDesc::new_void(),
        };

        // Parse next suffixed expression
        crate::compiler::expr_parser::suffixedexp(fs, &mut nv.v)?;

        if !vkisindexed(nv.v.kind) {
            check_conflict(fs, lh, &nv.v);
        }

        // Build chain
        nv.prev = Some(Box::new(LhsAssign {
            prev: lh.prev.take(),
            v: lh.v.clone(),
        }));

        restassign(fs, &mut nv, nvars + 1)?;
    } else {
        // restassign -> '=' explist
        check(fs, LuaTokenKind::TkAssign)?;
        fs.lexer.bump(); // consume '='
        let nexps = explist(fs, &mut e)?;

        if nexps != nvars {
            adjust_assign(fs, nvars, nexps, &mut e);
        } else {
            code::setoneret(fs, &mut e);
            storevar(fs, &lh.v, &mut e);
            return Ok(());
        }
    }

    // Default assignment
    e.kind = crate::compiler::expression::ExpKind::VNONRELOC;
    e.u.info = (fs.freereg - 1) as i32;
    storevar(fs, &lh.v, &mut e);

    Ok(())
}

// Port of exprstat from lparser.c
fn exprstat(fs: &mut FuncState) -> Result<(), String> {
    // exprstat -> func | assignment
    let mut lh = LhsAssign {
        prev: None,
        v: ExpDesc::new_void(),
    };

    suffixedexp(fs, &mut lh.v)?;

    if fs.lexer.current_token() == LuaTokenKind::TkAssign
        || fs.lexer.current_token() == LuaTokenKind::TkComma
    {
        // It's an assignment
        restassign(fs, &mut lh, 1)?;
    } else {
        // It's a function call
        if lh.v.kind != ExpKind::VCALL {
            return Err("syntax error".to_string());
        }
        // Set to use no results
        let pc = unsafe { lh.v.u.info as usize };
        if pc < fs.chunk.code.len() {
            Instruction::set_c(&mut fs.chunk.code[pc], 1);
        }
    }

    Ok(())
}

// Port of check_readonly from lparser.c (lines 277-304)
fn check_readonly(fs: &mut FuncState, e: &ExpDesc) -> Result<(), String> {
    use crate::compiler::expression::ExpKind;
    
    let varname: Option<String> = match e.kind {
        ExpKind::VCONST => {
            // TODO: Get variable name from actvar array
            Some("<const>".to_string())
        }
        ExpKind::VLOCAL => {
            // Check if local variable is const
            if let Some(var_desc) = fs.get_local_var_desc(unsafe { e.u.var.vidx } as u16) {
                if var_desc.kind != VarKind::VDKREG {
                    Some(var_desc.name.clone())
                } else {
                    None
                }
            } else {
                None
            }
        }
        ExpKind::VUPVAL => {
            // Check if upvalue is const
            let upval_idx = unsafe { e.u.info } as usize;
            if upval_idx < fs.upvalues.len() {
                let upval = &fs.upvalues[upval_idx];
                if upval.kind != VarKind::VDKREG {
                    Some(upval.name.clone())
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    };
    
    if let Some(name) = varname {
        return Err(format!("attempt to assign to const variable '{}'", name));
    }
    
    Ok(())
}
