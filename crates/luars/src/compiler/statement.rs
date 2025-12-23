use crate::compiler::expr_parser::{body, expr, suffixedexp};
// Statement parsing - Port from lparser.c (Lua 5.4.8)
// This file corresponds to statement parsing parts of lua-5.4.8/src/lparser.c
use crate::Instruction;
use crate::compiler::func_state::{BlockCnt, FuncState, LabelDesc};
use crate::compiler::parser::LuaTokenKind;
use crate::compiler::{BlockCntId, ExpDesc, ExpKind, VarKind, code};
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

    // Port of lparser.c:1912: Free registers after statement
    // "ls->fs->freereg = luaY_nvarstack(ls->fs);"
    // luaY_nvarstack returns reglevel(fs, fs->nactvar)
    fs.freereg = nvarstack(fs);

    // leavelevel(fs.lexer);
    Ok(())
}

// Port of reglevel from lparser.c (lines 323-330)
// Returns the register level (number of registers used) for the first 'nvar' variables
fn reglevel(fs: &FuncState, mut nvar: u8) -> u8 {
    while nvar > 0 {
        nvar -= 1;
        if let Some(var) = fs.actvar.get(nvar as usize) {
            if var.kind != VarKind::RDKCTC {
                // Variable is in a register
                return (var.ridx + 1) as u8;
            }
        }
    }
    0
}

// Port of luaY_nvarstack from lparser.c (line 332)
// Returns the number of registers used by active variables
fn nvarstack(fs: &FuncState) -> u8 {
    reglevel(fs, fs.nactvar)
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
    let msg = format!("expected '{}'", token);
    Err(fs.token_error(&msg))
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
            return Err(fs.syntax_error(&format!(
                "expected '{}' (to close '{}' at line {})",
                what, who, where_
            )));
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

// Port of enterblock from lparser.c:640-651
// Creates a new block and pushes it onto the block stack
pub fn enterblock(fs: &mut FuncState, bl_id: BlockCntId, isloop: bool) {
    let prev_id = fs.block_cnt_id.take();
    if let Some(bl) = fs.compiler_state.get_blockcnt_mut(bl_id) {
        bl.previous = prev_id;
        bl.first_label = fs.labels.len();
        bl.first_goto = fs.pending_gotos.len();
        bl.nactvar = fs.nactvar;
        bl.upval = false;
        bl.is_loop = isloop;
        bl.in_scope = true;
    }

    // Create a clone and put it in fs.block_list
    fs.block_cnt_id = Some(bl_id);
}

// Port of leaveblock from lparser.c
// Port of leaveblock from lparser.c:672-692
// Port of solvegoto from lparser.c:525-539
// Solves the goto at index 'g' to given 'label' and removes it from the list
fn solvegoto(fs: &mut FuncState, g: usize, label: &LabelDesc) {
    let gt = &fs.pending_gotos[g];

    // Check if goto jumps into scope (would be an error in full implementation)
    // For now we skip the scope check

    // Patch the jump
    code::patchlist(fs, gt.pc as isize, label.pc as isize);

    // Remove goto from pending list
    fs.pending_gotos.remove(g);
}

// Port of solvegotos from lparser.c:582-596
// Solves forward jumps. Check whether new label matches any pending gotos
// in current block and solves them. Return true if any of the gotos need to close upvalues.
fn solvegotos(fs: &mut FuncState, lb: &LabelDesc) -> bool {
    let first_goto = if let Some(bl) = &fs.current_block_cnt() {
        bl.first_goto
    } else {
        0
    };

    let mut i = first_goto;
    let mut needsclose = false;

    while i < fs.pending_gotos.len() {
        if fs.pending_gotos[i].name == lb.name {
            needsclose |= fs.pending_gotos[i].close;
            solvegoto(fs, i, lb);
            // solvegoto removes item at i, so don't increment
        } else {
            i += 1;
        }
    }

    needsclose
}

// Port of createlabel from lparser.c:608-621
// Create a new label with the given 'name' at the given 'line'.
// Solves all pending gotos to this new label and adds a close instruction if necessary.
// Returns true iff it added a close instruction.
fn createlabel(fs: &mut FuncState, name: &str, line: usize, last: bool) -> bool {
    let pc = code::get_label(fs);

    // Create label descriptor
    let mut label = LabelDesc {
        name: name.to_string(),
        pc,
        line,
        nactvar: fs.nactvar,
        close: false,
    };

    if last {
        // Label is last no-op statement in the block
        // Assume that locals are already out of scope
        if let Some(bl) = &fs.current_block_cnt() {
            label.nactvar = bl.nactvar;
        }
    }

    // Add to label list before solving gotos
    let label_for_list = label.clone();

    // Solve pending gotos
    let needsclose = solvegotos(fs, &label);

    // Now add to label list
    fs.labels.push(label_for_list);

    if needsclose {
        // Need a close instruction
        let stklevel = fs.reglevel(fs.nactvar);
        code::code_abc(fs, OpCode::Close, stklevel as u32, 0, 0);
        return true;
    }

    false
}

// Port of movegotosout from lparser.c:627-637
// Adjust pending gotos to outer level of a block
fn movegotosout(fs: &mut FuncState, bl: &BlockCnt) {
    let bl_nactvar = bl.nactvar;
    let bl_upval = bl.upval;
    let first_goto = bl.first_goto;

    for i in first_goto..fs.pending_gotos.len() {
        let gt_nactvar = fs.pending_gotos[i].nactvar;

        // Check if leaving a variable scope
        let gt_stklevel = fs.reglevel(gt_nactvar);
        let bl_stklevel = fs.reglevel(bl_nactvar);

        // Now we can modify the goto
        let gt = &mut fs.pending_gotos[i];
        if gt_stklevel > bl_stklevel {
            gt.close |= bl_upval; // Jump may need a close
        }
        gt.nactvar = bl_nactvar; // Update goto level
    }
}

// Port of leaveblock from lparser.c:673-695
pub fn leaveblock(fs: &mut FuncState) {
    if let Some(bl) = fs.take_block_cnt() {
        // Port of lparser.c:675-677
        let mut hasclose = false;
        let stklevel = fs.reglevel(bl.nactvar);

        // Remove block locals
        fs.remove_vars(bl.nactvar);

        // Port of lparser.c:678-679: handle loop break labels
        if bl.is_loop {
            hasclose = createlabel(fs, "break", 0, false);
        }

        // Port of lparser.c:680: still need a 'close'?
        if !hasclose && bl.previous.is_some() && bl.upval {
            code::code_abc(fs, OpCode::Close, stklevel as u32, 0, 0);
        }

        // Port of lparser.c:681: free registers
        fs.freereg = stklevel;

        // Port of lparser.c:682: remove local labels
        fs.labels.truncate(bl.first_label);

        // Save values needed after moving bl.previous
        let first_label = bl.first_label;
        let first_goto = bl.first_goto;
        let has_previous = bl.previous.is_some();

        // Port of lparser.c:683: current block now is previous one
        fs.block_cnt_id = bl.previous;

        // Port of lparser.c:684-689: move gotos out or check for undefined gotos
        if has_previous {
            // Need to reconstruct bl info for movegotosout
            let bl_info = BlockCnt {
                previous: None, // not used in movegotosout
                first_label,
                first_goto,
                nactvar: bl.nactvar,
                upval: bl.upval,
                is_loop: bl.is_loop,
                in_scope: bl.in_scope,
            };
            movegotosout(fs, &bl_info);
        } else {
            // At function level, check for undefined gotos
            if first_goto < fs.pending_gotos.len() {
                // In full implementation, this would raise an error
                // For now, we'll just leave them (they'll be caught later or ignored)
            }
        }
    }
}

// Port of block from lparser.c
fn block(fs: &mut FuncState) -> Result<(), String> {
    let bl_id = fs.compiler_state.alloc_blockcnt(BlockCnt::default());
    enterblock(fs, bl_id, false);
    statlist(fs)?;
    leaveblock(fs);
    Ok(())
}

// Port of retstat from lparser.c:1812-1843
// static void retstat (LexState *ls)
// stat -> RETURN [explist] [';']
fn retstat(fs: &mut FuncState) -> Result<(), String> {
    use ExpKind;
    // Port of lparser.c:1816: int first = luaY_nvarstack(fs);
    let mut first = nvarstack(fs);
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

// Port of newgotoentry from lparser.c:575-577
// Adds a new goto entry to the pending gotos list
fn newgotoentry(fs: &mut FuncState, name: String, line: usize, pc: usize) {
    let label = LabelDesc {
        name,
        pc,
        line,
        nactvar: fs.nactvar,
        close: false,
    };
    fs.pending_gotos.push(label);
}

// Port of findlabel from lparser.c:540-548
// Find a label with the given name in the current function
// Returns the label info (pc, nactvar) if found
fn findlabel(fs: &FuncState, name: &str) -> Option<(usize, u8)> {
    // Search backwards to find most recent label
    fs.labels
        .iter()
        .rev()
        .find(|lb| lb.name == name)
        .map(|lb| (lb.pc, lb.nactvar))
}

// Port of breakstat from lparser.c:1437-1440
// Break statement. Semantically equivalent to "goto break"
fn breakstat(fs: &mut FuncState) -> Result<(), String> {
    let line = fs.lexer.line;
    // breakstat is simply a goto to a label named "break"
    // The actual loop checking is done by createlabel when the break label is created
    let jmp = code::jump(fs);
    newgotoentry(fs, "break".to_string(), line, jmp);
    Ok(())
}

// Port of gotostat from lparser.c:1415-1433
// Goto statement. Either creates a forward jump or resolves a backward jump
fn gotostat(fs: &mut FuncState) -> Result<(), String> {
    let line = fs.lexer.line;
    let name = str_checkname(fs)?;

    // Check if label already exists (backward jump)
    if let Some((lb_pc, lb_nactvar)) = findlabel(fs, &name) {
        // Backward jump - resolve immediately
        let lblevel = fs.reglevel(lb_nactvar);
        let current_level = nvarstack(fs);

        // If leaving scope of variables, need CLOSE instruction
        if current_level > lblevel {
            code::code_abc(fs, OpCode::Close, lblevel as u32, 0, 0);
        }

        // Create jump and patch to label
        let jmp = code::jump(fs);
        code::patchlist(fs, jmp as isize, lb_pc as isize);
    } else {
        // Forward jump - will be resolved when label is declared
        let jmp = code::jump(fs);
        newgotoentry(fs, name, line, jmp);
    }

    Ok(())
}

// Port of labelstat from lparser.c
fn labelstat(fs: &mut FuncState) -> Result<(), String> {
    let name = str_checkname(fs)?;
    check(fs, LuaTokenKind::TkDbColon)?;
    fs.lexer.bump(); // skip '::'

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

    // Parse condition - Port of lparser.c:1474: condexit = cond(ls);
    let mut v = expr(fs)?;
    if v.kind == ExpKind::VNIL {
        v.kind = ExpKind::VFALSE; // 'falses' are all equal here
    }
    code::goiftrue(fs, &mut v);
    let condexit = v.f; // false list

    let bl_id = fs.compiler_state.alloc_blockcnt(BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: 0,
        upval: false,
        is_loop: true,
        in_scope: true,
    });
    enterblock(fs, bl_id, true);
    check(fs, LuaTokenKind::TkDo)?;
    fs.lexer.bump(); // skip 'do'
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

    // Patch exit jump - false conditions finish the loop
    code::patchtohere(fs, condexit);

    Ok(())
}

// Port of repeatstat from lparser.c
fn repeatstat(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // repeatstat -> REPEAT block UNTIL cond
    fs.lexer.bump(); // skip REPEAT
    let repeat_init = fs.pc;

    let bl_id = fs.compiler_state.alloc_blockcnt(BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: 0,
        upval: false,
        is_loop: true,
        in_scope: true,
    });
    enterblock(fs, bl_id, true);
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
// Port of forstat from lparser.c:1617-1636
fn forstat(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // forstat -> FOR (fornum | forlist) END
    // lparser.c:1623: enterblock(fs, &bl, 1);  /* scope for loop and control variables */
    let bl_id = fs.compiler_state.alloc_blockcnt(BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: fs.nactvar,
        upval: false,
        is_loop: true,
        in_scope: true,
    });
    enterblock(fs, bl_id, true);

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
            return Err(fs.token_error("'=' or 'in' expected"));
        }
    }

    check_match(fs, LuaTokenKind::TkEnd, LuaTokenKind::TkFor, line)?;

    // lparser.c:1633: leaveblock(fs);  /* loop scope ('break' jumps to this point) */
    leaveblock(fs);

    Ok(())
}

fn fornum(fs: &mut FuncState, varname: String, _line: usize) -> Result<(), String> {
    // fornum -> NAME = exp, exp [,exp] forbody
    // Port of lparser.c:1568-1590

    // Reserve registers for internal loop variables (must be done before parsing expressions)
    let base = fs.freereg;

    // Create 3 internal control variables: (for state), (for state), (for state)
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);

    // Create the loop variable
    fs.new_localvar(varname, VarKind::VDKREG);

    check(fs, LuaTokenKind::TkAssign)?; // check '='
    fs.lexer.bump(); // skip '='

    // Parse initial, limit, step (exp1 = expr + exp2nextreg)
    let mut e = expr(fs)?;
    code::exp2nextreg(fs, &mut e);
    check(fs, LuaTokenKind::TkComma)?;
    fs.lexer.bump(); // skip ','

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

    // Generate FORPREP with initial jump offset 0
    let prep_pc = code::code_asbx(fs, OpCode::ForPrep, base as u32, 0);

    check(fs, LuaTokenKind::TkDo)?;
    fs.lexer.bump(); // skip 'do'

    // Port of forbody from lparser.c:1552
    // Note: enterblock is called with isloop=0 (not a loop block)
    // The loop control is handled by FORPREP/FORLOOP, not by break labels
    let bl_id = fs.compiler_state.alloc_blockcnt(BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: fs.nactvar,
        upval: false,
        is_loop: false, // Important: inner block is NOT a loop block
        in_scope: true,
    });
    enterblock(fs, bl_id, false); // isloop = false

    // Activate the loop variable (4th variable)
    // lparser.c:1553-1554: adjustlocalvars + luaK_reserveregs
    fs.adjust_local_vars(1);
    code::reserve_regs(fs, 1);

    block(fs)?;

    leaveblock(fs);

    // Generate FORLOOP with initial Bx=0, will be fixed by fix_for_jump
    let loop_pc = code::code_abx(fs, OpCode::ForLoop, base as u32, 0);

    // Fix FORPREP: jump forward to FORLOOP position (loop_pc)
    // This matches lparser.c: fixforjump(fs, prep, luaK_getlabel(fs), 0)
    // where luaK_getlabel(fs) returns the position where FORLOOP was just generated
    fix_for_jump(fs, prep_pc, loop_pc, false)?;

    // Fix FORLOOP: jump back to after FORPREP (prep_pc + 1, loop body start)
    // This matches lparser.c: fixforjump(fs, endfor, prep + 1, 1)
    // back=true means the distance will be stored as positive (absolute value)
    fix_for_jump(fs, loop_pc, prep_pc + 1, true)?;

    // Don't remove variables here - the outer forstat's leaveblock will handle it
    // fs.remove_vars(fs.nactvar - 1);

    Ok(())
}

fn forlist(fs: &mut FuncState, indexname: String) -> Result<(), String> {
    // forlist -> NAME {,NAME} IN explist forbody
    // Port of lparser.c:1591-1616
    let mut nvars = 5; // gen, state, control, toclose, 'indexname'

    let base = fs.freereg;

    // Create 4 internal control variables
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);

    // Create declared variables (starting with indexname)
    fs.new_localvar(indexname, VarKind::VDKREG);

    while testnext(fs, LuaTokenKind::TkComma) {
        let varname = str_checkname(fs)?;
        fs.new_localvar(varname, VarKind::VDKREG);
        nvars += 1;
    }

    check(fs, LuaTokenKind::TkIn)?;
    fs.lexer.bump(); // skip IN

    // Parse iterator expressions
    let mut e = ExpDesc::new_void();
    let nexps = explist(fs, &mut e)?;

    // Adjust to 4 values (generator, state, control, toclose)
    adjust_assign(fs, 4, nexps, &mut e);

    // Activate the 4 control variables
    fs.adjust_local_vars(4);

    // lparser.c:1612: marktobeclosed(fs); /* last control var. must be closed */
    mark_to_be_closed(fs);

    check(fs, LuaTokenKind::TkDo)?;
    fs.lexer.bump(); // skip 'do'

    // lparser.c:1552: Generate TFORPREP with Bx=0, will be fixed later
    let prep_pc = code::code_abx(fs, OpCode::TForPrep, base as u32, 0);

    // lparser.c:1552: enterblock(fs, &bl, 0) - NOT a loop block (for scope of declared vars)
    let bl_id = fs.compiler_state.alloc_blockcnt(BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: fs.nactvar,
        upval: false,
        is_loop: false, // NOT a loop block - this is just for variable scope
        in_scope: true,
    });
    enterblock(fs, bl_id, false); // false = not a loop

    // lparser.c:1554: adjustlocalvars(ls, nvars); /* activate declared variables */
    fs.adjust_local_vars((nvars - 4) as u8);
    code::reserve_regs(fs, (nvars - 4) as u8);

    block(fs)?;

    leaveblock(fs);

    // lparser.c:1558: fixforjump(fs, prep, luaK_getlabel(fs), 0);
    // Fix TFORPREP to jump to current position (after leaveblock)
    let label_after_block = fs.pc;
    fix_for_jump(fs, prep_pc, label_after_block, false)?;

    // lparser.c:1559-1561: Generate TFORCALL for generic for
    code::code_abc(fs, OpCode::TForCall, base as u32, 0, (nvars - 4) as u32);

    // lparser.c:1562: Generate TFORLOOP with Bx=0, will be fixed later
    let endfor_pc = code::code_abx(fs, OpCode::TForLoop, base as u32, 0);

    // lparser.c:1563: fixforjump(fs, endfor, prep + 1, 1);
    // Fix TFORLOOP to jump back to prep+1 (back jump)
    fix_for_jump(fs, endfor_pc, prep_pc + 1, true)?;

    // Don't remove variables here - the outer forstat's leaveblock will handle it
    // fs.remove_vars(fs.nactvar - nvars as u8);

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
        crate::compiler::expr_parser::fieldsel(fs, &mut base)?;
    }

    // Handle method definition: function t:method()
    let is_method = fs.lexer.current_token() == LuaTokenKind::TkColon;
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
    fs.new_localvar(name, VarKind::VDKREG);
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
        fs.lexer.bump(); // skip '>'

        if attr == "const" {
            Ok(VarKind::RDKCONST)
        } else if attr == "close" {
            Ok(VarKind::RDKTOCLOSE)
        } else {
            Err(fs.syntax_error(&format!("unknown attribute '{}'", attr)))
        }
    } else {
        Ok(VarKind::VDKREG) // regular variable
    }
}

// Port of fixforjump from lparser.c:1530-1538
// Fix for instruction at position 'pc' to jump to 'dest'
// back=true means a back jump (negative offset)
fn fix_for_jump(fs: &mut FuncState, pc: usize, dest: usize, back: bool) -> Result<(), String> {
    // Port of lparser.c:1529 fixforjump
    // static void fixforjump (FuncState *fs, int pc, int dest, int back) {
    //   Instruction *jmp = &fs->f->code[pc];
    //   int offset = dest - (pc + 1);
    //   if (back)
    //     offset = -offset;
    //   if (l_unlikely(offset > MAXARG_Bx))
    //     luaX_syntaxerror(fs->ls, "control structure too long");
    //   SETARG_Bx(*jmp, offset);
    // }

    let mut offset = (dest as isize) - (pc as isize) - 1;
    if back {
        // For back jumps, negate to get positive distance
        // This is stored in Bx, and VM will subtract it (pc -= Bx)
        offset = -offset;
    }
    // For forward jumps, offset is already positive
    // VM will add it (pc += Bx + 1)

    // Validate range
    if offset < 0 || offset > Instruction::MAX_BX as isize {
        return Err(format!(
            "Warning: for-loop jump offset out of range: offset={}",
            offset
        ));
    }

    // Set Bx field directly (unsigned distance)
    Instruction::set_bx(&mut fs.chunk.code[pc], offset as u32);
    Ok(())
}

// Port of markupval from lparser.c:411-417
// Mark block where variable at given level was defined (to emit close instructions later)
pub fn mark_upval(fs: &mut FuncState, level: u8) {
    let mut bl_id_opt = fs.block_cnt_id;
    while let Some(bl_id) = bl_id_opt {
        if let Some(bl) = fs.compiler_state.get_blockcnt_mut(bl_id) {
            if bl.nactvar <= level {
                bl.upval = true;
                fs.needclose = true;
                break;
            }
            bl_id_opt = bl.previous;
        }
    }
}

// Port of marktobeclosed from lparser.c:423-429
// Mark that current block has a to-be-closed variable
fn mark_to_be_closed(fs: &mut FuncState) {
    if let Some(bl) = fs.current_block_cnt() {
        bl.upval = true;
    }
    fs.needclose = true;
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
        e.kind = ExpKind::VVOID;
        0
    };

    // Check for compile-time constant optimization
    // Get last variable
    let last_vidx = (fs.nactvar + nvars - 1) as u16;

    // First check if optimization is possible and get variable info for debugging
    let can_optimize = if let Some(var_desc) = fs.get_local_var_desc(last_vidx) {
        nvars as usize == nexps && var_desc.kind == VarKind::RDKCONST
    } else {
        false
    };

    let const_value = if can_optimize {
        code::exp2const(fs, &e)
    } else {
        None
    };

    if let Some(value) = const_value {
        // Variable is a compile-time constant
        if let Some(var_desc) = fs.get_local_var_desc(last_vidx) {
            var_desc.kind = VarKind::RDKCTC;
            var_desc.const_value = Some(value); // Save the constant value
        }
        fs.adjust_local_vars(nvars - 1); // exclude last variable
        fs.nactvar += 1; // but count it
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
fn vkisvar(k: ExpKind) -> bool {
    use ExpKind;
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
fn vkisindexed(k: ExpKind) -> bool {
    use ExpKind;
    matches!(
        k,
        ExpKind::VINDEXED | ExpKind::VINDEXUP | ExpKind::VINDEXI | ExpKind::VINDEXSTR
    )
}

// Port of check_conflict from lparser.c
fn check_conflict(fs: &mut FuncState, lh: &LhsAssign, v: &ExpDesc) {
    use ExpKind;

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
    use ExpKind;

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
    use ExpKind;

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
            // Use code_abrk to support RK operand (register or constant)
            code::code_abrk(
                fs,
                OpCode::SetTable,
                unsafe { var.u.ind.t as u32 },
                unsafe { var.u.ind.idx as u32 },
                ex,
            );
        }
        ExpKind::VINDEXUP => {
            // Use code_abrk to support RK operand (register or constant)
            code::code_abrk(
                fs,
                OpCode::SetTabUp,
                unsafe { var.u.ind.t as u32 },
                unsafe { var.u.ind.idx as u32 },
                ex,
            );
        }
        ExpKind::VINDEXI => {
            // Use code_abrk to support RK operand (register or constant)
            code::code_abrk(
                fs,
                OpCode::SetI,
                unsafe { var.u.ind.t as u32 },
                unsafe { var.u.ind.idx as u32 },
                ex,
            );
        }
        ExpKind::VINDEXSTR => {
            // Use code_abrk to support RK operand (register or constant)
            code::code_abrk(
                fs,
                OpCode::SetField,
                unsafe { var.u.ind.t as u32 },
                unsafe { var.u.ind.idx as u32 },
                ex,
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
    e.kind = ExpKind::VNONRELOC;
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
    use ExpKind;

    let varname: Option<String> = match e.kind {
        ExpKind::VCONST => {
            // Get variable name from actvar array
            let vidx = unsafe { e.u.info } as usize;
            if let Some(var_desc) = fs.actvar.get(vidx) {
                Some(var_desc.name.clone())
            } else {
                Some("<const>".to_string())
            }
        }
        ExpKind::VLOCAL => {
            // Check if local variable is const or compile-time const
            // Note: RDKTOCLOSE variables are NOT readonly!
            let vidx = unsafe { e.u.var.vidx } as u16;
            if let Some(var_desc) = fs.get_local_var_desc(vidx) {
                // Only RDKCONST and RDKCTC are truly readonly
                match var_desc.kind {
                    VarKind::RDKCONST | VarKind::RDKCTC => Some(var_desc.name.clone()),
                    _ => None,
                }
            } else {
                None
            }
        }
        ExpKind::VUPVAL => {
            // Check if upvalue is const or compile-time const
            // Note: RDKTOCLOSE upvalues are NOT readonly!
            let upval_idx = unsafe { e.u.info } as usize;
            if upval_idx < fs.upvalues.len() {
                let upval = &fs.upvalues[upval_idx];
                // Only RDKCONST and RDKCTC are truly readonly
                match upval.kind {
                    VarKind::RDKCONST | VarKind::RDKCTC => Some(upval.name.clone()),
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    };

    if let Some(name) = varname {
        let msg = format!("attempt to assign to const variable '{}'", name);
        return Err(fs.syntax_error(&msg));
    }

    Ok(())
}
