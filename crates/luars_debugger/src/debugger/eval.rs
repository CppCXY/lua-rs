//! Expression evaluation support.
//!
//! Eval requests are queued while the debugger is paused and executed
//! on the Lua thread (which owns the LuaState).

use luars::LuaState;

use crate::proto::{EvalReq, EvalRsp, Message, Variable};
use crate::transporter::Transporter;

use super::variables::make_variable;

/// Process a single eval request on the Lua thread.
/// Must be called while the Lua thread is paused (in debug mode).
///
/// `frame_level_map` maps DAP frame_id (0-based) → real Lua stack level.
/// If `req.stack_level` is < 0 or unmapped, we use frame 0's real level.
pub fn handle_eval(
    state: &mut LuaState,
    req: &EvalReq,
    transporter: &Transporter,
    frame_level_map: &[usize],
) {
    // Translate DAP frame_id → real Lua stack level
    let real_level = if req.stack_level >= 0 {
        frame_level_map
            .get(req.stack_level as usize)
            .copied()
            .unwrap_or(0) as i32
    } else {
        // -1 means "current frame" — use frame 0
        frame_level_map.first().copied().unwrap_or(0) as i32
    };

    let result = eval_expression(state, &req.expr, real_level, req.depth);
    let rsp = match result {
        Ok(var) => EvalRsp {
            seq: req.seq,
            success: true,
            value: Some(var),
            error: None,
        },
        Err(e) => EvalRsp {
            seq: req.seq,
            success: false,
            value: None,
            error: Some(e),
        },
    };
    if let Err(e) = transporter.send(Message::EvalRsp(rsp)) {
        eprintln!("[debugger] failed to send EvalRsp: {e}");
    }
}

/// Evaluate a Lua expression string at the given stack level.
fn eval_expression(
    state: &mut LuaState,
    expr: &str,
    stack_level: i32,
    depth: i32,
) -> Result<Variable, String> {
    // First try: look up local/upvalue by name at the given stack level
    let level = stack_level as usize;

    // Check locals
    let local_count = state.local_count(level);
    for i in 1..=local_count {
        if let Some((name, value)) = state.get_local(level, i)
            && name == expr
        {
            let mut cache_id = 0;
            return Ok(make_variable(&name, &value, depth, &mut cache_id));
        }
    }

    // Check upvalues
    let upvalue_count = state.upvalue_count(level);
    for i in 1..=upvalue_count {
        if let Some((name, value)) = state.get_upvalue(level, i)
            && name == expr
        {
            let mut cache_id = 0;
            return Ok(make_variable(&name, &value, depth, &mut cache_id));
        }
    }

    // Fallback: compile and execute via pcall (safe — won't corrupt call stack)
    safe_eval(state, &format!("return {expr}"), expr, depth)
        .or_else(|_| safe_eval(state, expr, expr, depth))
}

/// Compile `source`, run it via pcall, and wrap the first result as a Variable.
/// Uses load + pcall instead of execute to avoid corrupting the call stack
/// when the debugger evaluates expressions mid-pause.
fn safe_eval(
    state: &mut LuaState,
    source: &str,
    display_name: &str,
    depth: i32,
) -> Result<Variable, String> {
    let func = state
        .load(source)
        .map_err(|e| format!("compile error: {e}"))?;

    match state.pcall(func, vec![]) {
        Ok((true, results)) => {
            let mut cache_id = 0;
            if let Some(val) = results.first() {
                Ok(make_variable(display_name, val, depth, &mut cache_id))
            } else {
                let nil = luars::LuaValue::nil();
                Ok(make_variable(display_name, &nil, depth, &mut cache_id))
            }
        }
        Ok((false, err_results)) => {
            let msg = err_results
                .first()
                .map(|v| format!("{v}"))
                .unwrap_or_else(|| "unknown error".to_string());
            Err(msg)
        }
        Err(e) => Err(format!("{e}")),
    }
}
