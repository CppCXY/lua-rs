//! Lua hook callback — the bridge between the VM and the debugger.

use luars::LuaState;

use crate::proto::{BreakNotifyBody, MessageCMD, StackProto};
use crate::transporter::Transporter;

use super::breakpoint::{BreakPointManager, normalize_path};
use super::variables::make_variable;
use crate::hook_state::{HookAction, HookState};

/// Build the stack frames with local/upvalue variables for BreakNotify.
pub fn build_stacks(state: &LuaState) -> Vec<StackProto> {
    let mut stacks = Vec::new();
    let depth = state.call_depth();

    for level in 0..depth {
        let info = match state.get_info_by_level(level, "Sln") {
            Some(i) => i,
            None => continue,
        };

        let source = info.source.clone().unwrap_or_default();
        let func_name = info.name.clone().unwrap_or_else(|| "?".to_string());
        let line = info.currentline.unwrap_or(0);

        // Skip C functions (what == "C")
        if info.what == Some("C") {
            continue;
        }

        let mut cache_id = (level as i32) * 1000;

        // Collect local variables
        let mut locals = Vec::new();
        let local_count = state.local_count(level);
        for i in 1..=local_count {
            if let Some((name, value)) = state.get_local(level, i) {
                // Skip temporaries (names starting with '(')
                if name.starts_with('(') {
                    continue;
                }
                locals.push(make_variable(&name, &value, 1, &mut cache_id));
            }
        }

        // Collect upvalues
        let mut upvalues = Vec::new();
        let upvalue_count = state.upvalue_count(level);
        for i in 1..=upvalue_count {
            if let Some((name, value)) = state.get_upvalue(level, i) {
                upvalues.push(make_variable(&name, &value, 1, &mut cache_id));
            }
        }

        stacks.push(StackProto {
            file: source,
            function_name: func_name,
            line,
            level: level as i32,
            local_variables: locals,
            upvalue_variables: upvalues,
        });
    }

    stacks
}

/// Check if we should break at the current position.
/// Called from the line hook.
pub fn should_break(
    state: &LuaState,
    hook_state: &HookState,
    bp_manager: &mut BreakPointManager,
) -> bool {
    // Get current source and line
    let info = match state.get_info_by_level(0, "Sl") {
        Some(i) => i,
        None => return false,
    };

    let source = info.source.as_deref().unwrap_or("");
    let line = info.currentline.unwrap_or(0);

    // Check breakpoints first
    if let Some(bp) = bp_manager.find_mut(source, line) {
        // Check condition if any
        if !bp.condition.is_empty() {
            // TODO: evaluate condition expression
            // For now, always break
        }
        // Check hit condition
        if !bp.hit_condition.is_empty() {
            bp.hit_count += 1;
            if let Ok(target) = bp.hit_condition.parse::<i32>()
                && bp.hit_count < target
            {
                return false;
            }
        }
        return true;
    }

    // Check stepping state
    let depth = state.call_depth();
    let norm_source = normalize_path(source);
    hook_state.check(&norm_source, line, depth) == HookAction::Break
}

/// Send a BreakNotify message with the current stacks.
pub fn send_break_notify(state: &LuaState, transporter: &Transporter) {
    let stacks = build_stacks(state);
    let notify = BreakNotifyBody {
        cmd: MessageCMD::BreakNotify as i32,
        stacks,
    };
    let body = serde_json::to_string(&notify).unwrap_or_default();
    let _ = transporter.send(MessageCMD::BreakNotify, &body);
}
