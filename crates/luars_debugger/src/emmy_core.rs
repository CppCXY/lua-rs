//! Emmy debugger callbacks and macro-backed module descriptor.
//!
//! The callbacks remain low-level `CFunction`s because hook installation and
//! stepping operate directly on `LuaState`. The module registration itself is
//! described through `luars::lua_module!`, so embedders do not need a separate
//! `luaopen_*` entry point or hand-written table construction.

use std::sync::{Arc, OnceLock};

use luars::{LUA_MASKLINE, LuaResult, LuaState, LuaValue};

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

/// `emmy_core.tcpListen(host, port)` starts the debugger listener and installs
/// the line hook on the current state.
pub fn emmy_tcp_listen(l: &mut LuaState) -> LuaResult<usize> {
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

/// `emmy_core.tcpConnect(host, port)` connects to an already-running IDE and
/// installs the line hook on success.
pub fn emmy_tcp_connect(l: &mut LuaState) -> LuaResult<usize> {
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

/// `emmy_core.waitIDE()` blocks until the IDE finishes its handshake.
pub fn emmy_wait_ide(_l: &mut LuaState) -> LuaResult<usize> {
    let dbg = get_debugger();
    dbg.wait_ide(false);
    Ok(0)
}

/// `emmy_core.breakHere()` forces a debugger break when the IDE is ready.
pub fn emmy_break_here(l: &mut LuaState) -> LuaResult<usize> {
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

/// `emmy_core.stop()` disconnects and removes the installed hook.
pub fn emmy_stop(l: &mut LuaState) -> LuaResult<usize> {
    let dbg = get_debugger();
    dbg.transporter.disconnect();
    // Remove hook
    l.set_hook(LuaValue::nil(), 0, 0);
    Ok(0)
}

// ============ Hook installation ============

/// The line hook callback used by the debugger runtime.
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

/// Install the debugger line hook on the active Lua state.
fn install_hook(l: &mut LuaState) {
    l.set_hook(LuaValue::cfunction(debugger_hook), LUA_MASKLINE, 0);
}
