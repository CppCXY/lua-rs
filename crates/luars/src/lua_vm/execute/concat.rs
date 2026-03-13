/*----------------------------------------------------------------------
  String Concatenation - Lua 5.5 Style (luaV_concat)

  Direct port of lua-5.5.0/src/lvm.c luaV_concat.

  Concatenates 'total' values on the stack from top-total to top-1.
  Numbers are converted to strings; __concat metamethod is tried
  for non-string/non-number values.
----------------------------------------------------------------------*/

use crate::{
    lua_value::LuaValue,
    lua_vm::{
        LuaResult, LuaState,
        execute::{
            helper::{self, get_binop_metamethod},
            metamethod::{TmKind, call_tm_res},
        },
    },
    stdlib::{basic::lua_float_to_string, debug::typeerror},
};

/// Check whether a value can be converted to string for concat:
/// strings and numbers (integer/float) are convertible.
#[inline(always)]
fn cvt2str(v: &LuaValue) -> bool {
    v.ttisinteger() || v.ttisfloat()
}

/// Convert a number value on the stack to a string value in-place.
/// Equivalent to C Lua's luaO_tostring.
fn tostring_inplace(lua_state: &mut LuaState, idx: usize) -> LuaResult<bool> {
    let v = unsafe { *lua_state.stack().get_unchecked(idx) };
    if v.ttisstring() {
        return Ok(true);
    }
    if v.ttisinteger() {
        let mut buf = itoa::Buffer::new();
        let s = buf.format(v.ivalue());
        let sv = lua_state.create_string(s)?;
        unsafe { *lua_state.stack_mut().get_unchecked_mut(idx) = sv };
        return Ok(true);
    }
    if v.ttisfloat() {
        let s = lua_float_to_string(v.fltvalue());
        let sv = lua_state.create_string(&s)?;
        unsafe { *lua_state.stack_mut().get_unchecked_mut(idx) = sv };
        return Ok(true);
    }
    Ok(false)
}

/// Check if value is a short empty string (optimization: skip empty operands).
#[inline(always)]
fn isemptystr(v: &LuaValue) -> bool {
    v.is_short_string()
        && v.as_str().is_some_and(|s| s.is_empty())
}

/// Copy string bytes from stack positions [top-n .. top-1] into buffer.
/// All values at these positions must already be strings.
fn copy2buff(stack: &[LuaValue], top: usize, n: usize, buff: &mut Vec<u8>) {
    for i in (1..=n).rev() {
        let v = unsafe { stack.get_unchecked(top - i) };
        if let Some(bytes) = v.as_str_bytes() {
            buff.extend_from_slice(bytes);
        }
    }
}

/// Main operation for concatenation: concat 'total' values in the stack,
/// from `top - total` up to `top - 1`.
///
/// Direct port of Lua 5.5's luaV_concat (lvm.c).
#[inline(never)]
pub fn concat(
    lua_state: &mut LuaState,
    mut total: usize,
) -> LuaResult<()> {
    if total == 1 {
        return Ok(()); // "all" values already concatenated
    }
    loop {
        let top = lua_state.get_top();
        let n;

        let v1 = unsafe { lua_state.stack().get_unchecked(top - 2) };

        if (!v1.ttisstring() && !cvt2str(v1)) || !tostring_inplace(lua_state, top - 1)? {
            // Cannot convert to string — try __concat metamethod
            tryconcattm(lua_state, top)?;
            n = 2;
        } else if isemptystr(unsafe { lua_state.stack().get_unchecked(top - 1) }) {
            // Second operand is empty string — result is first operand (just convert it)
            tostring_inplace(lua_state, top - 2)?;
            n = 2;
        } else if isemptystr(unsafe { lua_state.stack().get_unchecked(top - 2) }) {
            // First operand is empty string — result is second operand
            helper::setobjs2s(lua_state, top - 2, top - 1);
            n = 2;
        } else {
            // At least two string values; collect as many consecutive convertible values as possible
            let mut tl: usize = {
                let s = unsafe { lua_state.stack().get_unchecked(top - 1) };
                s.as_str_bytes().map_or(0, |b| b.len())
            };

            let mut nn = 1usize;
            while nn < total && tostring_inplace(lua_state, top - nn - 1)? {
                let l = unsafe { lua_state.stack().get_unchecked(top - nn - 1) }
                    .as_str_bytes()
                    .map_or(0, |b| b.len());
                if l >= usize::MAX - tl {
                    return Err(lua_state.error("string length overflow".to_string()));
                }
                tl += l;
                nn += 1;
            }

            // Build the concatenated string
            let mut buff = Vec::with_capacity(tl);
            copy2buff(lua_state.stack(), top, nn, &mut buff);

            // Create the result string (interned if short, long otherwise)
            let result = if let Ok(s) = std::str::from_utf8(&buff) {
                lua_state.create_string(s)?
            } else {
                lua_state.create_binary(buff)?
            };
            unsafe {
                *lua_state.stack_mut().get_unchecked_mut(top - nn) = result;
            }
            n = nn;
        }

        total -= n - 1; // got 'n' strings to create one new
        lua_state.set_top_raw(top - (n - 1)); // popped 'n' strings and pushed one

        if total <= 1 {
            break;
        }
    }
    Ok(())
}

/// Try __concat metamethod on the top two stack values.
/// Equivalent to C Lua's luaT_tryconcatTM.
/// `top` is the stack top at the time of the call (operands at top-2 and top-1).
fn tryconcattm(lua_state: &mut LuaState, top: usize) -> LuaResult<()> {
    let p1 = unsafe { *lua_state.stack().get_unchecked(top - 2) };
    let p2 = unsafe { *lua_state.stack().get_unchecked(top - 1) };

    if let Some(mm) = get_binop_metamethod(lua_state, &p1, &p2, TmKind::Concat) {
        let result = call_tm_res(lua_state, mm, p1, p2)?;
        // Store result at top - 2 (replaces first operand)
        unsafe {
            *lua_state.stack_mut().get_unchecked_mut(top - 2) = result;
        }
        Ok(())
    } else {
        // No metamethod found — generate error
        let bad = if p1.ttisstring() || cvt2str(&p1) { &p2 } else { &p1 };
        Err(typeerror(lua_state, bad, "concatenate"))
    }
}
