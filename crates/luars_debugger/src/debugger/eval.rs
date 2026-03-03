//! Expression evaluation support.
//!
//! Eval requests are queued while the debugger is paused and executed
//! on the Lua thread (which owns the LuaState).

use luars::LuaState;

use crate::proto::{EvalReqBody, EvalRspBody, MessageCMD};
use crate::transporter::Transporter;

use super::variables::make_variable;

/// Process a single eval request on the Lua thread.
/// Must be called while the Lua thread is paused (in debug mode).
pub fn handle_eval(state: &mut LuaState, req: &EvalReqBody, transporter: &Transporter) {
    let result = eval_expression(state, &req.expr, req.stack_level, req.depth);
    let rsp = match result {
        Ok(var) => EvalRspBody {
            seq: req.seq,
            success: true,
            value: Some(var),
            error: None,
        },
        Err(e) => EvalRspBody {
            seq: req.seq,
            success: false,
            value: None,
            error: Some(e),
        },
    };
    let body = serde_json::to_string(&rsp).unwrap_or_default();
    let _ = transporter.send(MessageCMD::EvalRsp, &body);
}

/// Evaluate a Lua expression string at the given stack level.
fn eval_expression(
    state: &mut LuaState,
    expr: &str,
    _stack_level: i32,
    depth: i32,
) -> Result<crate::proto::VariableProto, String> {
    // First try: look up local/upvalue by name at the given stack level
    let level = _stack_level as usize;

    // Check locals
    let local_count = state.local_count(level);
    for i in 1..=local_count {
        if let Some((name, value)) = state.get_local(level, i) {
            if name == expr {
                let mut cache_id = 0;
                return Ok(make_variable(&name, &value, depth, &mut cache_id));
            }
        }
    }

    // Check upvalues
    let upvalue_count = state.upvalue_count(level);
    for i in 1..=upvalue_count {
        if let Some((name, value)) = state.get_upvalue(level, i) {
            if name == expr {
                let mut cache_id = 0;
                return Ok(make_variable(&name, &value, depth, &mut cache_id));
            }
        }
    }

    // Fallback: try to compile and execute the expression as `return <expr>`
    let chunk_src = format!("return {expr}");
    match state.execute(&chunk_src) {
        Ok(results) => {
            if let Some(val) = results.first() {
                let mut cache_id = 0;
                Ok(make_variable(expr, val, depth, &mut cache_id))
            } else {
                let mut cache_id = 0;
                let nil = luars::LuaValue::nil();
                Ok(make_variable(expr, &nil, depth, &mut cache_id))
            }
        }
        Err(_) => {
            // Try as statement
            match state.execute(expr) {
                Ok(_) => {
                    let mut cache_id = 0;
                    let nil = luars::LuaValue::nil();
                    Ok(make_variable(expr, &nil, depth, &mut cache_id))
                }
                Err(_) => Err(format!("Failed to evaluate: {expr}")),
            }
        }
    }
}
