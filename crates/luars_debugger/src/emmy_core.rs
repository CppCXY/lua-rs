//! `require "emmy_core"` module interface.
//!
//! Provides Lua-facing functions: `tcpListen`, `tcpConnect`, `waitIDE`,
//! `breakHere`, `tcpSharedListen`, `tcpSharedConnect`.
//!
//! The module returns a table with these functions.

use std::sync::{Arc, OnceLock};

use luars::{CFunction, LUA_MASKLINE, LuaResult, LuaState, LuaValue};

use crate::debugger::Debugger;
use crate::debugger::hook::should_break;

/// Global debugger instance. Set once during library installation.
/// CFunction callbacks can't capture state, so we use a global.
static DEBUGGER: OnceLock<Arc<Debugger>> = OnceLock::new();

/// Get the global debugger (panics if not initialized).
fn get_debugger() -> &'static Arc<Debugger> {
    DEBUGGER.get().expect("debugger not initialized")
}

/// Set the global debugger instance. Called once during library installation.
pub(crate) fn set_debugger(dbg: Arc<Debugger>) {
    DEBUGGER.set(dbg).ok();
}

// ============ CFunction callbacks ============

/// `emmy_core.tcpListen(host, port)` — start TCP listener.
fn emmy_tcp_listen(l: &mut LuaState) -> LuaResult<usize> {
    let host = l
        .get_arg(1)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "localhost".to_string());
    let port = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(9966) as u16;

    let dbg = get_debugger();
    match dbg.tcp_listen(&host, port) {
        Ok(()) => {
            dbg.start_receiver();
            // Install the line hook
            install_hook(l);
            l.push_value(LuaValue::boolean(true))?;
            Ok(1)
        }
        Err(e) => {
            eprintln!("[debugger] tcpListen error: {e}");
            l.push_value(LuaValue::boolean(false))?;
            Ok(1)
        }
    }
}

/// `emmy_core.tcpConnect(host, port)` — connect to IDE.
fn emmy_tcp_connect(l: &mut LuaState) -> LuaResult<usize> {
    let host = l
        .get_arg(1)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "localhost".to_string());
    let port = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(9966) as u16;

    let dbg = get_debugger();
    match dbg.tcp_connect(&host, port) {
        Ok(()) => {
            dbg.start_receiver();
            install_hook(l);
            l.push_value(LuaValue::boolean(true))?;
            Ok(1)
        }
        Err(e) => {
            eprintln!("[debugger] tcpConnect error: {e}");
            l.push_value(LuaValue::boolean(false))?;
            Ok(1)
        }
    }
}

/// `emmy_core.waitIDE()` — block until IDE sends ReadyReq.
fn emmy_wait_ide(_l: &mut LuaState) -> LuaResult<usize> {
    let dbg = get_debugger();
    dbg.wait_ide(false);
    Ok(0)
}

/// `emmy_core.breakHere()` — programmatic breakpoint.
fn emmy_break_here(l: &mut LuaState) -> LuaResult<usize> {
    let dbg = get_debugger();
    let is_ready = {
        let s = dbg.state.lock().unwrap();
        s.ide_ready
    };

    if is_ready {
        dbg.enter_debug_mode(l);
    }
    Ok(0)
}

/// `emmy_core.stop()` — stop the debugger.
fn emmy_stop(l: &mut LuaState) -> LuaResult<usize> {
    let dbg = get_debugger();
    dbg.transporter.disconnect();
    // Remove hook
    l.set_hook(LuaValue::nil(), 0, 0);
    Ok(0)
}

// ============ Hook installation ============

/// The line hook callback used by the debugger.
fn debugger_hook(l: &mut LuaState) -> LuaResult<usize> {
    let dbg = get_debugger();

    // Get line from hook arg (2nd argument, already provided by run_hook — free, no debug info)
    let line = l.get_arg(2).and_then(|v| v.as_integer()).unwrap_or(-1) as i32;
    if line < 0 {
        return Ok(0);
    }

    let should = {
        let mut s = dbg.state.lock().unwrap();
        if !s.ide_ready || !s.started {
            return Ok(0);
        }
        let hs = s.hook_state.clone();
        should_break(l, line, &hs, &mut s.bp_manager)
    };

    if should {
        dbg.enter_debug_mode(l);
    }

    Ok(0)
}

/// Install the debugger line hook on the LuaState.
fn install_hook(l: &mut LuaState) {
    l.set_hook(LuaValue::cfunction(debugger_hook), LUA_MASKLINE, 0);
}

// ============ Module loader ============

/// The CFunction that serves as the `require "emmy_core"` loader.
/// Returns a table with all debugger functions.
pub fn luaopen_emmy_core(l: &mut LuaState) -> LuaResult<usize> {
    let table = l.create_table(0, 8)?;

    let set_field =
        |l: &mut LuaState, t: &LuaValue, name: &str, func: CFunction| -> LuaResult<()> {
            let key = l.create_string(name)?;
            let val = LuaValue::cfunction(func);
            l.raw_set(t, key, val);
            Ok(())
        };

    set_field(l, &table, "tcpListen", emmy_tcp_listen)?;
    set_field(l, &table, "tcpConnect", emmy_tcp_connect)?;
    set_field(l, &table, "waitIDE", emmy_wait_ide)?;
    set_field(l, &table, "breakHere", emmy_break_here)?;
    set_field(l, &table, "stop", emmy_stop)?;

    l.push_value(table)?;
    Ok(1)
}
