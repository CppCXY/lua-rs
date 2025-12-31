use crate::compiler::expr_parser::buildglobal;
use expr_parser::{body, expr, suffixedexp};
// Statement parsing - Port from lparser.c (Lua 5.4.8)
// This file corresponds to statement parsing parts of lua-5.4.8/src/lparser.c
use crate::Instruction;
use crate::compiler::func_state::{BlockCnt, FuncState, LabelDesc, LhsAssign, LhsAssignId};
use crate::compiler::parser::LuaTokenKind;
use crate::compiler::{BlockCntId, ExpDesc, ExpKind, ExpUnion, VarKind, code, expr_parser};
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
            let mut is_global_stat = false;
            if fs.lexer.current_token_text() == "global" {
                let next = fs.lexer.peek_next_token();
                if matches!(
                    next,
                    LuaTokenKind::TkMul
                        | LuaTokenKind::TkLt
                        | LuaTokenKind::TkName
                        | LuaTokenKind::TkFunction
                ) {
                    is_global_stat = true;
                    // Lua 5.5: global statement
                    globalstatfunc(fs, line)?;
                }
            }

            if !is_global_stat {
                exprstat(fs)?;
            }
        }
    }

    // Port of lparser.c:2136-2137: Free registers after statement
    // "ls->fs->freereg = luaY_nvarstack(ls->fs);"
    // luaY_nvarstack returns reglevel(fs, fs->nactvar)
    // Use the reglevel method from FuncState which correctly handles all variable kinds
    fs.freereg = fs.nvarstack();

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
pub fn enterblock(fs: &mut FuncState, bl_id: BlockCntId, isloop: u8) {
    let prev_id = fs.block_cnt_id.take();

    // Port of lparser.c:656-664: inherit insidetbc from previous block
    let parent_in_scope = if let Some(prev_id) = prev_id {
        fs.compiler_state
            .get_blockcnt_mut(prev_id)
            .map(|bl| bl.in_scope)
            .unwrap_or(false)
    } else {
        false
    };

    // Get nactvar before the mutable borrow
    let nactvar = fs.nactvar;

    if let Some(bl) = fs.compiler_state.get_blockcnt_mut(bl_id) {
        bl.previous = prev_id;
        bl.first_label = fs.labels.len();
        bl.first_goto = fs.pending_gotos.len();
        bl.nactvar = nactvar;
        bl.upval = false;
        bl.is_loop = isloop;
        bl.in_scope = parent_in_scope; // Inherit from parent
    }

    // Create a clone and put it in fs.block_list
    fs.block_cnt_id = Some(bl_id);
}

// Port of leaveblock from lparser.c
// Port of leaveblock from lparser.c:672-692
// Port of closegoto from lparser.c:597-621
// Solves the goto at index 'g' to given 'label' and removes it from the list
fn solvegoto(fs: &mut FuncState, g: usize, label: &LabelDesc, bl_upval: bool) {
    let gt = &fs.pending_gotos[g].clone(); // Clone to avoid borrow issues

    // lparser.c:606-614: Check if goto needs a CLOSE instruction
    // gt->close means the goto jumps out of the scope of some variables
    // (label->nactvar < gt->nactvar && bup) means label is in outer scope and block has upvalues
    let needs_close = gt.close || (label.nactvar < gt.nactvar && bl_upval);

    let mut pc = gt.pc;

    if needs_close {
        // lparser.c:608-613: Need CLOSE instruction
        // CRITICAL: Use label.stklevel computed when label was created,
        // not reglevel(label.nactvar), because actvar may have been modified by removevars
        let stklevel = label.stklevel;

        // Move jump to CLOSE position (pc + 1)
        let jmp_instr = fs.chunk.code[pc];
        fs.chunk.code[pc + 1] = jmp_instr;
        // Put real CLOSE instruction at original position
        fs.chunk.code[pc] = Instruction::create_abc(OpCode::Close, stklevel as u32, 0, 0);
        pc += 1; // Must point to jump instruction now
    }

    // lparser.c:615: Patch the jump to label
    code::patchlist(fs, pc as isize, label.pc as isize);

    // lparser.c:616-619: Remove goto from pending list
    fs.pending_gotos.remove(g);
}

// Port of findlabel from lparser.c:544-557
// Search for an active label with the given name
fn findlabel<'a>(fs: &'a FuncState, name: &str) -> Option<&'a LabelDesc> {
    fs.labels.iter().find(|lb| lb.name == name)
}

// Port of checkrepeated from lparser.c:1445-1454
// Check whether there is already a label with the given 'name'
fn checkrepeated(fs: &FuncState, name: &str) -> Result<(), String> {
    if let Some(lb) = findlabel(fs, name) {
        return Err(format!(
            "label '{}' already defined on line {}",
            name, lb.line
        ));
    }
    Ok(())
}

// Port of createlabel from lparser.c:608-621
// Create a new label with the given 'name' at the given 'line'.
// 'last' tells whether label is the last non-op statement in its block.
// Note: In Lua 5.5, createlabel does NOT solve gotos or generate CLOSE instructions.
// Gotos are solved later in solvegotos() called from leaveblock().
fn createlabel(fs: &mut FuncState, name: &str, line: usize, last: bool) {
    let pc = code::get_label(fs);

    // Create label descriptor
    let mut label = LabelDesc {
        name: name.to_string(),
        pc,
        line,
        nactvar: fs.nactvar,
        stklevel: 0, // Will be computed below
        close: false,
    };

    if last {
        // Label is last no-op statement in the block
        // Assume that locals are already out of scope
        if let Some(bl) = &fs.current_block_cnt() {
            label.nactvar = bl.nactvar;
        }
    }

    // CRITICAL: Compute stklevel AFTER nactvar is finalized
    // This ensures stklevel matches the correct nactvar value
    label.stklevel = fs.reglevel(label.nactvar);

    // Add to label list
    // Note: We do NOT solve gotos here, unlike our old implementation.
    // Gotos will be solved in solvegotos() called from leaveblock().
    fs.labels.push(label);
}

// Port of solvegotos from lparser.c:696-717 (Lua 5.5)
// Solve pending gotos when leaving a block
fn solvegotos_on_leaveblock(
    fs: &mut FuncState,
    bl: &BlockCnt,
    outlevel: u8,
    goto_levels: &[(usize, u8)],
) {
    let mut igt = bl.first_goto; // first goto in the finishing block
    let mut level_idx = 0;

    while igt < fs.pending_gotos.len() {
        let gt_name = fs.pending_gotos[igt].name.clone();

        // Search for a matching label in the current block
        let label_opt = fs.labels[bl.first_label..]
            .iter()
            .find(|lb| lb.name == gt_name)
            .cloned();

        if let Some(label) = label_opt {
            // Found a match - close and remove goto
            let bl_upval = bl.upval;
            solvegoto(fs, igt, &label, bl_upval);
            // solvegoto removes the goto, so don't increment igt or level_idx
        } else {
            // Adjust goto for outer block
            // lparser.c:710-711: if block has upvalues and goto escapes scope, mark for close
            //
            // NOTE: Known issue with goto.lua test:
            // When a goto jumps out of a block containing local variables (not upvalues),
            // the official Lua 5.5 somehow generates a CLOSE instruction, but our
            // implementation doesn't because bl.upval is false for regular local variables.
            //
            // The condition `bl.upval && gt_level > outlevel` correctly follows Lua 5.5
            // lparser.c:710-711, but there may be another mechanism in Lua 5.5 that
            // marks bl->upval=true in cases we haven't identified, or the CLOSE is
            // generated through a different path (possibly in closegoto when the label
            // is found in an outer block with upvalues).
            //
            // Removing the bl.upval check causes other tests to fail, so the current
            // implementation matches Lua 5.5 source code but may not match its behavior
            // in all cases.
            //
            // Use pre-computed level from goto_levels
            let gt_level = if level_idx < goto_levels.len() && goto_levels[level_idx].0 == igt {
                let level = goto_levels[level_idx].1;
                level_idx += 1;
                level
            } else {
                // Fallback: this shouldn't happen if we computed levels correctly
                0
            };

            if bl.upval && gt_level > outlevel {
                fs.pending_gotos[igt].close = true;
            }
            // lparser.c:712: correct level for outer block
            fs.pending_gotos[igt].nactvar = bl.nactvar;
            igt += 1; // go to next goto
        }
    }

    // lparser.c:716: remove local labels
    fs.labels.truncate(bl.first_label);
}

// Port of leaveblock from lparser.c:745-762 (Lua 5.5)
pub fn leaveblock(fs: &mut FuncState) {
    if let Some(bl_id) = fs.block_cnt_id {
        // Get block info
        let (nactvar, first_goto, is_loop, has_previous, previous_id, upval) = {
            if let Some(bl) = fs.compiler_state.get_blockcnt_mut(bl_id) {
                (
                    bl.nactvar,
                    bl.first_goto,
                    bl.is_loop,
                    bl.previous.is_some(),
                    bl.previous,
                    bl.upval,
                )
            } else {
                return;
            }
        };

        // lparser.c:748: level outside block
        let stklevel = fs.reglevel(nactvar);

        // Pre-compute reglevel for all pending gotos before removevars
        // This is needed because removevars will remove actvar entries
        let mut goto_levels = Vec::new();
        for i in first_goto..fs.pending_gotos.len() {
            let gt_nactvar = fs.pending_gotos[i].nactvar;
            let gt_level = fs.reglevel(gt_nactvar);
            goto_levels.push((i, gt_level));
        }

        // lparser.c:749-750: need a 'close'?
        if has_previous && upval {
            code::code_abc(fs, OpCode::Close, stklevel as u32, 0, 0);
        }

        // lparser.c:751: free registers
        fs.freereg = stklevel;

        // lparser.c:752: remove block locals
        fs.remove_vars(nactvar);

        // lparser.c:754-755: has to fix pending breaks?
        if is_loop == 2 {
            createlabel(fs, "break", 0, false);
        }

        // lparser.c:756: solve gotos
        // Need to reconstruct bl for solvegotos_on_leaveblock
        if let Some(bl) = fs.compiler_state.get_blockcnt_mut(bl_id) {
            let bl_info = BlockCnt {
                previous: bl.previous,
                first_label: bl.first_label,
                first_goto: bl.first_goto,
                nactvar: bl.nactvar,
                upval: bl.upval,
                is_loop: bl.is_loop,
                in_scope: bl.in_scope,
            };
            solvegotos_on_leaveblock(fs, &bl_info, stklevel, &goto_levels);
        }

        // lparser.c:757-760: check for undefined gotos at function level
        if !has_previous {
            if let Some(bl) = fs.compiler_state.get_blockcnt_mut(bl_id) {
                if bl.first_goto < fs.pending_gotos.len() {
                    // In full implementation, this would raise an error
                    // For now, we'll just leave them
                }
            }
        }

        // lparser.c:761: current block now is previous one
        fs.block_cnt_id = previous_id;
    }
}

// Port of block from lparser.c
fn block(fs: &mut FuncState) -> Result<(), String> {
    let bl_id = fs.compiler_state.alloc_blockcnt(BlockCnt::default());
    enterblock(fs, bl_id, 0);
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
    let mut first = fs.nvarstack();
    let mut nret: i32;
    let mut e = ExpDesc::new_void();

    // Port of lparser.c:2085: Check insidetbc before explist
    // Must capture this before calling explist which may mutate block state
    let insidetbc = fs
        .current_block_cnt()
        .map(|bl| bl.in_scope)
        .unwrap_or(false);

    if block_follow(fs, true) || fs.lexer.current_token() == LuaTokenKind::TkSemicolon {
        // return with no values
        nret = 0;
    } else {
        nret = explist(fs, &mut e)? as i32;
        // Check if expression has multiple returns (VCALL or VVARARG)
        if matches!(e.kind, ExpKind::VCALL | ExpKind::VVARARG) {
            code::setmultret(fs, &mut e);
            // Port of lparser.c:2085: if (e.k == VCALL && nret == 1 && !fs->bl->insidetbc)
            // Tail call optimization: only if not inside a to-be-closed scope
            if e.kind == ExpKind::VCALL && nret == 1 && !insidetbc {
                // Tail call optimization
                let pc = e.u.info() as usize;
                if pc < fs.chunk.code.len() {
                    let instr = &mut fs.chunk.code[pc];
                    *instr =
                        Instruction::from_u32((instr.as_u32() & !0x7F) | (OpCode::TailCall as u32));
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
    use expr_parser::expr_internal;

    let mut n = 1;
    expr_internal(fs, e)?;

    while testnext(fs, LuaTokenKind::TkComma) {
        code::exp2nextreg(fs, e);
        *e = ExpDesc::new_void(); // Reset ExpDesc for next expression
        expr_internal(fs, e)?;
        n += 1;
    }

    Ok(n)
}

// Port of newgotoentry from lparser.c:663-667
// Creates a new goto entry with JMP and placeholder CLOSE instruction
// Returns the pc of the JMP instruction
fn newgotoentry(fs: &mut FuncState, name: String, line: usize) -> usize {
    // lparser.c:665: int pc = luaK_jump(fs); /* create jump */
    let pc = code::jump(fs);
    // lparser.c:666: luaK_codeABC(fs, OP_CLOSE, 0, 1, 0); /* spaceholder, marked as dead */
    code::code_abc(fs, OpCode::Close, 0, 1, 0);

    let stklevel = fs.reglevel(fs.nactvar);
    let label = LabelDesc {
        name,
        pc,
        line,
        nactvar: fs.nactvar,
        stklevel,
        close: false,
    };
    fs.pending_gotos.push(label);
    pc
}

// Port of breakstat from lparser.c:1547-1559
// Break statement. Semantically equivalent to "goto break"
fn breakstat(fs: &mut FuncState) -> Result<(), String> {
    let line = fs.lexer.line;
    // lparser.c:1602-1613: Find enclosing loop and mark it as having pending breaks
    let mut found_loop = false;
    let mut current_bl_id = fs.block_cnt_id;
    while let Some(bl_id) = current_bl_id {
        if let Some(bl) = fs.compiler_state.get_blockcnt_mut(bl_id) {
            if bl.is_loop > 0 {
                bl.is_loop = 2; // Signal that block has pending breaks
                found_loop = true;
                break;
            }
            current_bl_id = bl.previous;
        } else {
            break;
        }
    }
    if !found_loop {
        return Err("break outside loop".to_string());
    }
    // lparser.c:1611: newgotoentry(ls, ls->brkn, line);
    newgotoentry(fs, "break".to_string(), line);
    Ok(())
}

// Port of gotostat from lparser.c:1415-1433
// Goto statement. Either creates a forward jump or resolves a backward jump
fn gotostat(fs: &mut FuncState) -> Result<(), String> {
    let line = fs.lexer.line;
    let name = str_checkname(fs)?;

    // lparser.c:1538-1541: All gotos (forward and backward) use newgotoentry
    // They are all resolved later via solvegotos mechanism
    newgotoentry(fs, name.clone(), line);

    Ok(())
}

// Port of labelstat from lparser.c:1457-1466
// labelstat -> '::' NAME '::' {no-op-stat}
fn labelstat(fs: &mut FuncState) -> Result<(), String> {
    let line = fs.lexer.line;
    let name = str_checkname(fs)?;
    check(fs, LuaTokenKind::TkDbColon)?;
    fs.lexer.bump(); // skip second '::'

    // Skip other no-op statements (lparser.c:1461-1462)
    while fs.lexer.current_token() == LuaTokenKind::TkSemicolon
        || fs.lexer.current_token() == LuaTokenKind::TkDbColon
    {
        statement(fs)?;
    }

    // Check for repeated labels (lparser.c:1463)
    checkrepeated(fs, &name)?;

    // Create label and solve pending gotos (lparser.c:1464)
    let is_last = block_follow(fs, false);
    createlabel(fs, &name, line, is_last);

    Ok(())
}

// Port of test_then_block from lparser.c:1752-1764 (Lua 5.5)
fn test_then_block(fs: &mut FuncState, escapelist: &mut isize) -> Result<(), String> {
    // test_then_block -> [IF | ELSEIF] cond THEN block
    // lparser.c:1756: luaX_next(ls); /* skip IF or ELSEIF */
    fs.lexer.bump();

    // lparser.c:1757: condtrue = cond(ls); /* read condition */
    let condtrue = cond(fs)?;

    // lparser.c:1758: checknext(ls, TK_THEN);
    check(fs, LuaTokenKind::TkThen)?;
    fs.lexer.bump();

    // lparser.c:1759: block(ls); /* 'then' part */
    block(fs)?;

    // lparser.c:1760-1762: if followed by else/elseif, jump over it
    if fs.lexer.current_token() == LuaTokenKind::TkElse
        || fs.lexer.current_token() == LuaTokenKind::TkElseIf
    {
        // lparser.c:1762: luaK_concat(fs, escapelist, luaK_jump(fs));
        let jmp = code::jump(fs) as isize;
        code::concat(fs, escapelist, jmp);
    }

    // lparser.c:1763: luaK_patchtohere(fs, condtrue);
    code::patchtohere(fs, condtrue);

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

// Port of cond from lparser.c:1405-1412
// static int cond (LexState *ls)
// cond -> exp
fn cond(fs: &mut FuncState) -> Result<isize, String> {
    let mut v = expr(fs)?;
    if v.kind == ExpKind::VNIL {
        v.kind = ExpKind::VFALSE; // 'falses' are all equal here
    }
    code::goiftrue(fs, &mut v);
    Ok(v.f)
}

// Port of whilestat from lparser.c:1467-1483
fn whilestat(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // whilestat -> WHILE cond DO block END
    fs.lexer.bump(); // skip WHILE
    let whileinit = code::getlabel(fs);
    let condexit = cond(fs)?;

    let bl_id = fs.compiler_state.alloc_blockcnt(BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: 0,
        upval: false,
        is_loop: 1,
        in_scope: true,
    });
    enterblock(fs, bl_id, 1);
    check(fs, LuaTokenKind::TkDo)?;
    fs.lexer.bump(); // skip 'do'
    block(fs)?; // Official Lua uses block(ls), not statlist
    code::jumpto(fs, whileinit);
    check_match(fs, LuaTokenKind::TkEnd, LuaTokenKind::TkWhile, line)?;
    leaveblock(fs);

    // Patch exit jump - false conditions finish the loop
    code::patchtohere(fs, condexit);

    Ok(())
}

// Port of repeatstat from lparser.c:1486-1507
fn repeatstat(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // repeatstat -> REPEAT block UNTIL cond
    let repeat_init = code::getlabel(fs);

    // lparser.c:1491-1492: enterblock(fs, &bl1, 1); enterblock(fs, &bl2, 0);
    let bl1_id = fs.compiler_state.alloc_blockcnt(BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: 0,
        upval: false,
        is_loop: 1, // loop block
        in_scope: true,
    });
    enterblock(fs, bl1_id, 1);

    let bl2_id = fs.compiler_state.alloc_blockcnt(BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: 0,
        upval: false,
        is_loop: 0, // scope block
        in_scope: true,
    });
    enterblock(fs, bl2_id, 0);

    fs.lexer.bump(); // skip REPEAT
    statlist(fs)?; // repeat body
    check_match(fs, LuaTokenKind::TkUntil, LuaTokenKind::TkRepeat, line)?;

    // Parse until condition (inside scope block)
    let mut condexit = cond(fs)?;

    // lparser.c:1498: leaveblock(fs); /* finish scope */
    // Read bl2 upval before leaveblock
    let bl2_upval = fs
        .compiler_state
        .get_blockcnt_mut(bl2_id)
        .map(|bl| bl.upval)
        .unwrap_or(false);
    let bl2_nactvar = fs
        .compiler_state
        .get_blockcnt_mut(bl2_id)
        .map(|bl| bl.nactvar)
        .unwrap_or(0);
    leaveblock(fs);

    // lparser.c:1499-1505: handle upvalues
    if bl2_upval {
        let exit = code::jump(fs); // normal exit must jump over fix
        code::patchtohere(fs, condexit); // repetition must close upvalues
        code::code_abc(fs, OpCode::Close, fs.reglevel(bl2_nactvar) as u32, 0, 0);
        condexit = code::jump(fs) as isize; // repeat after closing upvalues
        code::patchtohere(fs, exit as isize); // normal exit comes to here
    }

    // lparser.c:1506: luaK_patchlist(fs, condexit, repeat_init);
    code::patchlist(fs, condexit, repeat_init as isize);

    // lparser.c:1507: leaveblock(fs); /* finish loop */
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
        is_loop: 1,
        in_scope: true,
    });
    enterblock(fs, bl_id, 1);

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
    // Port of lparser.c:1685-1704

    // Reserve registers for internal loop variables (must be done before parsing expressions)
    let base = fs.freereg;

    // Create 2 internal control variables (Lua 5.5: only 2, not 3!)
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);

    // Create the loop variable (read-only control variable in Lua 5.5)
    fs.new_localvar(varname, VarKind::RDKCONST);

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

    // Adjust local variables (2 internal control variables only!)
    // The loop variable itself will be adjusted in forbody
    fs.adjust_local_vars(2);

    // Generate FORPREP with initial jump offset 0
    let prep_pc = code::code_asbx(fs, OpCode::ForPrep, base as u32, 0);
    // lparser.c:1668: both 'forprep' remove one register from the stack
    fs.freereg -= 1;

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
        is_loop: 0, // Important: inner block is NOT a loop block
        in_scope: true,
    });
    enterblock(fs, bl_id, 0); // isloop = 0

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
    // Port of lparser.c:1707-1731
    let mut nvars = 4; // function, state, closing, control (indexname)

    let base = fs.freereg;

    // Create 3 internal control variables (Lua 5.5: iterator function, state, closing var)
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);
    fs.new_localvar("(for state)".to_string(), VarKind::VDKREG);

    // Create declared variables (starting with indexname) - read-only in Lua 5.5
    fs.new_localvar(indexname, VarKind::RDKCONST);

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

    // Adjust to 4 values (iterator function, state, closing, control) per lparser.c:1726
    adjust_assign(fs, 4, nexps, &mut e);

    // Activate the 3 internal control variables (not the control variable yet!)
    // Per lparser.c:1727: adjustlocalvars(ls, 3);
    fs.adjust_local_vars(3);

    // lparser.c:1728: marktobeclosed(fs); /* last internal var. must be closed */
    mark_to_be_closed(fs);

    check(fs, LuaTokenKind::TkDo)?;
    fs.lexer.bump(); // skip 'do'

    // lparser.c:1667: Generate TFORPREP with Bx=0, will be fixed later
    let prep_pc = code::code_abx(fs, OpCode::TForPrep, base as u32, 0);
    // lparser.c:1668: fs->freereg--; both 'forprep' remove one register from the stack
    fs.freereg -= 1;

    // lparser.c:1669: enterblock(fs, &bl, 0) - NOT a loop block (for scope of declared vars)
    let bl_id = fs.compiler_state.alloc_blockcnt(BlockCnt {
        previous: None,
        first_label: 0,
        first_goto: 0,
        nactvar: fs.nactvar,
        upval: false,
        is_loop: 0, // NOT a loop block - this is just for variable scope
        in_scope: true,
    });
    enterblock(fs, bl_id, 0); // 0 = not a loop

    // lparser.c:1670: adjustlocalvars(ls, nvars); /* activate declared variables */
    // nvars-3 because we already adjusted 3 internal variables
    fs.adjust_local_vars((nvars - 3) as u8);
    code::reserve_regs(fs, (nvars - 3) as u8);

    block(fs)?;

    leaveblock(fs);

    // lparser.c:1674: fixforjump(fs, prep, luaK_getlabel(fs), 0);
    // Fix TFORPREP to jump to current position (after leaveblock)
    let label_after_block = fs.pc;
    fix_for_jump(fs, prep_pc, label_after_block, false)?;

    // lparser.c:1676: Generate TFORCALL for generic for
    // nvars-3 because the first 3 are internal variables
    code::code_abc(fs, OpCode::TForCall, base as u32, 0, (nvars - 3) as u32);

    // lparser.c:1679: Generate TFORLOOP with Bx=0, will be fixed later
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
    expr_parser::singlevar(fs, &mut base)?;

    // Handle field selections: t.a.b.c
    while fs.lexer.current_token() == LuaTokenKind::TkDot {
        expr_parser::fieldsel(fs, &mut base)?;
    }

    // Handle method definition: function t:method()
    let is_method = fs.lexer.current_token() == LuaTokenKind::TkColon;
    if is_method {
        expr_parser::fieldsel(fs, &mut base)?;
    }

    // Parse function body
    let mut func_val = ExpDesc::new_void();
    expr_parser::body(fs, &mut func_val, is_method)?;

    // Check if variable is readonly
    check_readonly(fs, &mut base)?;

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

// Port of getvarattribute from lparser.c:1793-1810
// attrib -> ['<' NAME '>']
fn getvarattribute(fs: &mut FuncState, default: VarKind) -> Result<VarKind, String> {
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
        Ok(default) // return default value
    }
}

// Port of getglobalattribute from lparser.c:1858-1869
// Get attribute for global variable, adapting local attributes to global equivalents
fn getglobalattribute(fs: &mut FuncState, default: VarKind) -> Result<VarKind, String> {
    let kind = getvarattribute(fs, default)?;
    match kind {
        VarKind::RDKTOCLOSE => Err(fs.syntax_error("global variables cannot be to-be-closed")),
        VarKind::RDKCONST => Ok(VarKind::GDKCONST), // adjust kind for global variable
        _ => Ok(kind),
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
        bl.in_scope = true; // Port of lparser.c:610: bl->insidetbc = 1;
    }
    fs.needclose = true;
}

// Port of checktoclose from lparser.c
// Port of checktoclose from lparser.c:1717-1722
fn check_to_close(fs: &mut FuncState, level: isize) {
    if level != -1 {
        // lparser.c:1812: marktobeclosed(fs);
        mark_to_be_closed(fs);
        // lparser.c:1813: luaK_codeABC(fs, OP_TBC, reglevel(fs, level), 0, 0);
        let reg_level = fs.reglevel(level as u8);
        code::code_abc(fs, OpCode::Tbc, reg_level as u32, 0, 0);
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

    // Get prefixed attribute (if any); default is regular local variable
    let defkind = getvarattribute(fs, VarKind::VDKREG)?;

    // Parse variable list: NAME attrib { ',' NAME attrib }
    loop {
        let vname = str_checkname(fs)?;

        // Get postfixed attribute (if any)
        let kind = getvarattribute(fs, defkind)?;

        // Create variable with determined kind
        fs.new_localvar(vname, kind);

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
        // Official Lua lparser.c:1903-1908
        let ridx = fs.reglevel(fs.nactvar);
        if let Some(var_desc) = fs.get_local_var_desc(last_vidx) {
            var_desc.kind = VarKind::RDKCTC;
            var_desc.const_value = Some(value); // Save the constant value
            // Set register index before adjustlocalvars (lparser.c:1905)
            var_desc.ridx = ridx as i16;
        }
        // adjustlocalvars(ls, nvars - 1) - exclude last variable (lparser.c:1906)
        fs.adjust_local_vars(nvars - 1);
        // fs->nactvar++ - but count it (lparser.c:1907)
        fs.nactvar += 1;
        // NOTE: Don't adjust freereg here! Official Lua doesn't do it either.
        // The RDKCTC variable has a valid ridx, and when first used, discharge_vars
        // will load its value to that register. freereg is managed by reserv_regs/adjust_local_vars.
        check_to_close(fs, toclose);
        return Ok(());
    }

    adjust_assign(fs, nvars as usize, nexps, &mut e);
    fs.adjust_local_vars(nvars);

    check_to_close(fs, toclose);

    Ok(())
}

// Port of vkisvar from lparser.h:74
// #define vkisvar(k)	(VLOCAL <= (k) && (k) <= VINDEXSTR)
fn vkisvar(k: ExpKind) -> bool {
    use ExpKind;
    matches!(
        k,
        ExpKind::VLOCAL
            | ExpKind::VVARGVAR
            | ExpKind::VGLOBAL
            | ExpKind::VUPVAL
            | ExpKind::VCONST
            | ExpKind::VINDEXED
            | ExpKind::VVARGIND  // Lua 5.5: vararg indexed is a variable
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
        ExpKind::VINDEXED
            | ExpKind::VINDEXUP
            | ExpKind::VINDEXI
            | ExpKind::VINDEXSTR
            | ExpKind::VVARGIND
    )
}

// Port of check_conflict from lparser.c
fn check_conflict(fs: &mut FuncState, lh_id: LhsAssignId, v: &ExpDesc) {
    use ExpKind;

    let mut conflict = false;
    let extra = fs.freereg;

    // lparser.c:1453-1482: Check all previous assignments and update conflicting nodes
    let mut nodes_to_update: Vec<(LhsAssignId, bool, bool)> = Vec::new(); // (id, update_t, update_idx)

    let mut current = Some(lh_id);
    while let Some(node_id) = current {
        if let Some(node) = fs.compiler_state.get_lhs_assign(node_id) {
            let mut update_t = false;
            let mut update_idx = false;

            if vkisindexed(node.v.kind) {
                // assignment to table field?
                // lparser.c:1456-1463: Check if table is an upvalue
                if node.v.kind == ExpKind::VINDEXUP {
                    // lparser.c:1457-1461: Table is upvalue being assigned
                    if v.kind == ExpKind::VUPVAL && node.v.u.ind().t == v.u.info() as i16 {
                        conflict = true;
                        update_t = true;
                        // lparser.c:1459: lh->v.k = VINDEXSTR
                    }
                } else {
                    // lparser.c:1464-1478: table is a register
                    // lparser.c:1465-1468: Is table the local being assigned?
                    if v.kind == ExpKind::VLOCAL && node.v.u.ind().t == v.u.var().ridx {
                        conflict = true;
                        update_t = true; // lparser.c:1467: lh->v.u.ind.t = extra
                    }
                    // lparser.c:1469-1474: Is index the local being assigned?
                    if node.v.kind == ExpKind::VINDEXED
                        && v.kind == ExpKind::VLOCAL
                        && node.v.u.ind().idx == v.u.var().ridx
                    {
                        conflict = true;
                        update_idx = true; // lparser.c:1472: lh->v.u.ind.idx = extra
                    }
                }
            }

            if update_t || update_idx {
                nodes_to_update.push((node_id, update_t, update_idx));
            }

            current = node.prev;
        } else {
            break;
        }
    }

    // lparser.c:1480-1486: If conflict, copy local/upvalue to temporary
    if conflict {
        if v.kind == ExpKind::VLOCAL {
            code::code_abc(fs, OpCode::Move, extra as u32, v.u.var().ridx as u32, 0);
        } else {
            code::code_abc(fs, OpCode::GetUpval, extra as u32, v.u.info() as u32, 0);
        }
        code::reserve_regs(fs, 1);

        // Now update all conflicting nodes to use the safe copy
        for (node_id, update_t, update_idx) in nodes_to_update {
            if let Some(node) = fs.compiler_state.get_lhs_assign_mut(node_id) {
                if update_t {
                    // lparser.c:1459-1461, 1467
                    if node.v.kind == ExpKind::VINDEXUP {
                        node.v.kind = ExpKind::VINDEXSTR;
                    }
                    node.v.u.ind_mut().t = extra as i16;
                }
                if update_idx {
                    // lparser.c:1472
                    node.v.u.ind_mut().idx = extra as i16;
                }
            }
        }
    }
}

// Port of adjust_assign from lparser.c:482-498
fn adjust_assign(fs: &mut FuncState, nvars: usize, nexps: usize, e: &mut ExpDesc) {
    use ExpKind;

    let needed = nvars as isize - nexps as isize;

    // Check if last expression has multiple returns
    if matches!(e.kind, ExpKind::VCALL | ExpKind::VVARARG) {
        let mut extra = needed + 1;
        // lparser.c:488-489: if (extra < 0) extra = 0;
        if extra < 0 {
            extra = 0;
        }
        // lparser.c:489: luaK_setreturns(fs, e, extra);
        code::setreturns(fs, e, extra as u8);
    } else {
        // lparser.c:492-493: if (e->k != VVOID) luaK_exp2nextreg(fs, e);
        if e.kind != ExpKind::VVOID {
            code::exp2nextreg(fs, e);
        }
        // lparser.c:494-495: if (needed > 0) luaK_nil(fs, fs->freereg, needed);
        if needed > 0 {
            code::nil(fs, fs.freereg, needed as u8);
        }
    }

    // lparser.c:496-500: Adjust freereg
    if needed > 0 {
        code::reserve_regs(fs, needed as u8);
    } else {
        // lparser.c:500: adding 'needed' is actually a subtraction
        fs.freereg = (fs.freereg as isize + needed) as u8;
    }
}

// Port of luaK_storevar from lcode.c
fn storevar(fs: &mut FuncState, var: &ExpDesc, ex: &mut ExpDesc) {
    use ExpKind;

    match var.kind {
        ExpKind::VLOCAL => {
            code::free_exp(fs, ex);
            code::exp2reg(fs, ex, var.u.var().ridx as u8);
        }
        ExpKind::VUPVAL => {
            let e = code::exp2anyreg(fs, ex);
            code::code_abc(fs, OpCode::SetUpval, e as u32, var.u.info() as u32, 0);
        }
        ExpKind::VINDEXED => {
            // Use code_abrk to support RK operand (register or constant)
            code::code_abrk(
                fs,
                OpCode::SetTable,
                var.u.ind().t as u32,
                var.u.ind().idx as u32,
                ex,
            );
        }
        ExpKind::VVARGIND => {
            // Lua 5.5: assignment to indexed vararg parameter
            // Mark that function needs a vararg table (needvatab in lcode.c)
            fs.chunk.needs_vararg_table = true;
            // Now, assignment is to a regular table (SETTABLE instruction)
            code::code_abrk(
                fs,
                OpCode::SetTable,
                var.u.ind().t as u32,
                var.u.ind().idx as u32,
                ex,
            );
        }
        ExpKind::VINDEXUP => {
            // Use code_abrk to support RK operand (register or constant)
            code::code_abrk(
                fs,
                OpCode::SetTabUp,
                var.u.ind().t as u32,
                var.u.ind().idx as u32,
                ex,
            );
        }
        ExpKind::VINDEXI => {
            // Use code_abrk to support RK operand (register or constant)
            code::code_abrk(
                fs,
                OpCode::SetI,
                var.u.ind().t as u32,
                var.u.ind().idx as u32,
                ex,
            );
        }
        ExpKind::VINDEXSTR => {
            // Use code_abrk to support RK operand (register or constant)
            code::code_abrk(
                fs,
                OpCode::SetField,
                var.u.ind().t as u32,
                var.u.ind().idx as u32,
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
fn restassign(fs: &mut FuncState, lh_id: LhsAssignId, nvars: usize) -> Result<(), String> {
    let mut e = ExpDesc::new_void();

    // Get a copy of the LhsAssign data for checking
    let mut lh_v_for_check = {
        let lh = fs
            .compiler_state
            .get_lhs_assign(lh_id)
            .ok_or_else(|| "invalid LhsAssign id".to_string())?;
        lh.v.clone()
    };

    // Check if variable is read-only before assignment
    check_readonly(fs, &mut lh_v_for_check)?;

    // Get the LhsAssign data again for actual assignment
    let lh_v = {
        let lh = fs
            .compiler_state
            .get_lhs_assign(lh_id)
            .ok_or_else(|| "invalid LhsAssign id".to_string())?;
        lh.v.clone()
    };

    if !vkisvar(lh_v.kind) {
        return Err(fs.syntax_error("syntax error").to_string());
    }

    if testnext(fs, LuaTokenKind::TkComma) {
        // restassign -> ',' suffixedexp restassign
        let mut nv_v = ExpDesc::new_void();

        // Parse next suffixed expression
        expr_parser::suffixedexp(fs, &mut nv_v)?;

        if !vkisindexed(nv_v.kind) {
            check_conflict(fs, lh_id, &nv_v);
        }

        // Get updated lh_v after check_conflict for chain building
        let updated_lh_v = {
            let lh = fs
                .compiler_state
                .get_lhs_assign(lh_id)
                .ok_or_else(|| "invalid LhsAssign id".to_string())?;
            lh.v.clone()
        };

        // Get the prev id from current lh before creating new one
        let prev_id = fs
            .compiler_state
            .get_lhs_assign(lh_id)
            .map(|lh| lh.prev)
            .flatten();

        // Build chain: new node points to a copy of current lh (updated)
        let new_prev_id = fs.compiler_state.alloc_lhs_assign(LhsAssign {
            prev: prev_id,
            v: updated_lh_v,
        });

        // Create new LhsAssign with the chain
        let nv_id = fs.compiler_state.alloc_lhs_assign(LhsAssign {
            prev: Some(new_prev_id),
            v: nv_v,
        });

        restassign(fs, nv_id, nvars + 1)?;
    } else {
        // restassign -> '=' explist
        check(fs, LuaTokenKind::TkAssign)?;
        fs.lexer.bump(); // consume '='
        let nexps = explist(fs, &mut e)?;

        if nexps != nvars {
            adjust_assign(fs, nvars, nexps, &mut e);
        } else {
            code::setoneret(fs, &mut e);
            // Get updated lh_v after potential check_conflict updates
            let updated_lh_v = fs
                .compiler_state
                .get_lhs_assign(lh_id)
                .map(|lh| lh.v.clone())
                .unwrap_or(lh_v.clone());
            storevar(fs, &updated_lh_v, &mut e);
            return Ok(());
        }
    }

    // Default assignment
    e.kind = ExpKind::VNONRELOC;
    e.u = ExpUnion::Info((fs.freereg - 1) as i32);
    // Get updated lh_v after potential check_conflict updates
    let updated_lh_v = fs
        .compiler_state
        .get_lhs_assign(lh_id)
        .map(|lh| lh.v.clone())
        .unwrap_or(lh_v);
    storevar(fs, &updated_lh_v, &mut e);

    Ok(())
}

// Port of exprstat from lparser.c
fn exprstat(fs: &mut FuncState) -> Result<(), String> {
    // exprstat -> func | assignment
    let mut lh_v = ExpDesc::new_void();

    suffixedexp(fs, &mut lh_v)?;

    if fs.lexer.current_token() == LuaTokenKind::TkAssign
        || fs.lexer.current_token() == LuaTokenKind::TkComma
    {
        // It's an assignment - allocate LhsAssign in pool
        let lh_id = fs.compiler_state.alloc_lhs_assign(LhsAssign {
            prev: None,
            v: lh_v,
        });
        restassign(fs, lh_id, 1)?;
    } else {
        // It's a function call
        if lh_v.kind != ExpKind::VCALL {
            return Err("syntax error".to_string());
        }
        // Set to use no results
        let pc = lh_v.u.info() as usize;
        if pc < fs.chunk.code.len() {
            Instruction::set_c(&mut fs.chunk.code[pc], 1);
        }
    }

    Ok(())
}

// Port of check_readonly from lparser.c (lines 277-304)
fn check_readonly(fs: &mut FuncState, e: &mut ExpDesc) -> Result<(), String> {
    use ExpKind;

    // lparser.c:307-310: Handle VVARGIND - needs vararg table and convert to VINDEXED
    if e.kind == ExpKind::VVARGIND {
        fs.chunk.needs_vararg_table = true;
        e.kind = ExpKind::VINDEXED;
        // Fall through to VINDEXED check
    }

    let varname: Option<String> = match e.kind {
        ExpKind::VCONST => {
            // Get variable name from actvar array
            let vidx = e.u.info() as usize;
            if let Some(var_desc) = fs.actvar.get(vidx) {
                Some(var_desc.name.clone())
            } else {
                Some("<const>".to_string())
            }
        }
        ExpKind::VLOCAL | ExpKind::VVARGVAR => {
            // Check if local variable is const or compile-time const
            // Note: RDKTOCLOSE variables are NOT readonly!
            let vidx = e.u.var().vidx as u16;
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
            let upval_idx = e.u.info() as usize;
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
        ExpKind::VINDEXUP | ExpKind::VINDEXSTR | ExpKind::VINDEXED => {
            // Check if indexed access is to a read-only table field
            // For now, we don't support the `ro` flag completely
            // Just check if the flag is set
            if e.u.ind().ro {
                Some("<readonly field>".to_string())
            } else {
                None
            }
        }
        ExpKind::VINDEXI => {
            // Integer index cannot be read-only
            return Ok(());
        }
        _ => None,
    };

    if let Some(name) = varname {
        let msg = format!("attempt to assign to const variable '{}'", name);
        return Err(fs.syntax_error(&msg));
    }

    Ok(())
}

// ============ Lua 5.5: Global statement support ============

// Port of globalstatfunc from lparser.c:1962-1969
// static void globalstatfunc (LexState *ls, int line)
fn globalstatfunc(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // stat -> GLOBAL globalfunc | GLOBAL globalstat
    fs.lexer.bump(); // skip 'global'

    if testnext(fs, LuaTokenKind::TkFunction) {
        globalfunc(fs, line)?;
    } else {
        globalstat(fs)?;
    }

    Ok(())
}

// Port of globalstat from lparser.c:1931-1943
// static void globalstat (LexState *ls)
fn globalstat(fs: &mut FuncState) -> Result<(), String> {
    // globalstat -> (GLOBAL) attrib '*'
    // globalstat -> (GLOBAL) attrib NAME attrib {',' NAME attrib}

    // Get prefixed attribute (if any); default is regular global variable (GDKREG)
    let defkind = getglobalattribute(fs, VarKind::GDKREG)?;

    if testnext(fs, LuaTokenKind::TkMul) {
        // global * - collective declaration (voids implicit global-by-default)
        // Use empty string name to represent '*' entries
        fs.new_localvar(String::new(), defkind);
        fs.nactvar += 1; // Activate declaration
        return Ok(());
    }

    // global name [, name ...] [= explist]
    globalnames(fs, defkind)?;

    Ok(())
}

// Port of globalnames from lparser.c:1908-1928
// static void globalnames (LexState *ls, lu_byte defkind)
fn globalnames(fs: &mut FuncState, defkind: VarKind) -> Result<(), String> {
    // Parse: NAME attrib {',' NAME attrib} ['=' explist]
    let mut nvars = 0;
    let mut lastidx: u16;

    loop {
        // Check for NAME token
        if fs.lexer.current_token() != LuaTokenKind::TkName {
            return Err(fs.syntax_error("name expected"));
        }

        // Get variable name
        let vname = fs.lexer.current_token_text().to_string();
        fs.lexer.bump();

        // Get postfixed attribute (if any)
        let kind = getglobalattribute(fs, defkind)?;

        // Create the global variable entry and save last index for initialization
        lastidx = fs.new_localvar(vname, kind);
        nvars += 1;

        if !testnext(fs, LuaTokenKind::TkComma) {
            break;
        }
    }

    // Check for initialization: = explist
    if testnext(fs, LuaTokenKind::TkAssign) {
        // Initialize globals: calls initglobal recursively
        // lastidx points to the last variable, so first variable is at (lastidx - nvars + 1)
        let line = fs.lexer.line; // Current line number for error reporting
        initglobal(fs, nvars, (lastidx - nvars as u16 + 1) as usize, 0, line)?;
    }

    // Activate all declared globals
    fs.nactvar = (fs.nactvar as i32 + nvars) as u8;

    Ok(())
}

// Port of initglobal from lparser.c:1886-1912
// Recursively traverse list of globals to be initialized
fn initglobal(
    fs: &mut FuncState,
    nvars: i32,
    firstidx: usize,
    n: i32,
    line: usize,
) -> Result<(), String> {
    if n == nvars {
        // Traversed all variables? Read list of expressions
        let mut e = ExpDesc::new_void();
        let nexps = explist(fs, &mut e)?;
        adjust_assign(fs, nvars as usize, nexps, &mut e);
    } else {
        // Handle variable 'n'
        let var_desc = &fs.actvar[firstidx + n as usize];
        let varname = var_desc.name.clone();
        let mut var = ExpDesc::new_void();
        buildglobal(fs, &varname, &mut var)?;

        // Control recursion depth - in Lua 5.5 this calls enterlevel/leavelevel
        initglobal(fs, nvars, firstidx, n + 1, line)?;

        checkglobal(fs, &varname, line)?;
        storevartop(fs, &var);
    }

    Ok(())
}

// Port of globalfunc from lparser.c:1946-1960
// static void globalfunc (LexState *ls, int line)
fn globalfunc(fs: &mut FuncState, line: usize) -> Result<(), String> {
    // globalfunc -> (GLOBAL FUNCTION) NAME body

    // Check for function name
    if fs.lexer.current_token() != LuaTokenKind::TkName {
        return Err(fs.syntax_error("function name expected"));
    }

    let fname = fs.lexer.current_token_text().to_string();
    fs.lexer.bump();

    // Declare as global variable (GDKREG)
    fs.new_localvar(fname.clone(), VarKind::GDKREG);
    fs.nactvar += 1; // Enter its scope

    // Build global variable expression
    let mut var = ExpDesc::new_void();
    buildglobal(fs, &fname, &mut var)?;

    // Parse function body
    let mut b = ExpDesc::new_void();
    body(fs, &mut b, false)?;

    checkglobal(fs, &fname, line)?;

    // Store function in global variable
    storevar(fs, &var, &mut b);
    code::fixline(fs, line);

    Ok(())
}

// Port of checkglobal from lparser.c:1876-1883
fn checkglobal(fs: &mut FuncState, varname: &str, line: usize) -> Result<(), String> {
    let mut var = ExpDesc::new_void();
    // lparser.c:1879: create global variable in 'var'
    buildglobal(fs, varname, &mut var)?;

    // lparser.c:1880-1881: get index of global name in 'k'
    let k = var.u.ind().keystr;

    // lparser.c:1882: luaK_codecheckglobal(fs, &var, k, line)
    code::codecheckglobal(fs, &mut var, k, line);

    Ok(())
}

// Helper: Store value at top of stack to variable (used by initglobal)
// Inline implementation based on luaK_storevar
fn storevartop(fs: &mut FuncState, var: &ExpDesc) {
    // The value is already at freereg-1 (top of stack)
    // Just need to generate the store instruction
    let mut e = ExpDesc::new_void();
    e.kind = ExpKind::VNONRELOC;
    e.u = ExpUnion::Info((fs.freereg - 1) as i32);
    storevar(fs, var, &mut e);
}
