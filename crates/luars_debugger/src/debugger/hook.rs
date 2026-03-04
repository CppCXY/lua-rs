//! Lua hook callback — the bridge between the VM and the debugger.

use luars::LuaState;

use crate::proto::{BreakNotify, Message, Stack};
use crate::transporter::Transporter;

use super::breakpoint::{BreakPointManager, normalize_path, strip_at_prefix};
use super::variables::make_variable;
use crate::hook_state::{HookAction, HookState};

/// Build the stack frames with local/upvalue variables for BreakNotify.
/// Source names have their leading `@` stripped for the IDE.
///
/// Returns `(stacks, level_map)` where:
/// - `stacks` uses contiguous 0-based `level` values (for DAP frame IDs)
/// - `level_map[frame_id]` gives the real Lua stack level (for eval/locals)
pub fn build_stacks(state: &LuaState) -> (Vec<Stack>, Vec<usize>) {
    let mut stacks = Vec::new();
    let mut level_map: Vec<usize> = Vec::new();
    let depth = state.call_depth();
    let mut frame_id: i32 = 0;

    for level in 0..depth {
        let info = match state.get_info_by_level(level, "Sln") {
            Some(i) => i,
            None => continue,
        };

        // Skip C functions (what == "C")
        if info.what == Some("C") {
            continue;
        }

        // Strip `@` prefix from source name for IDE display
        let source = info.source.as_deref().unwrap_or("");
        let file = strip_at_prefix(source).to_string();
        let func_name = info.name.clone().unwrap_or_else(|| "?".to_string());
        let line = info.currentline.unwrap_or(0);

        let mut cache_id = frame_id * 1000;

        // Collect local variables (using real Lua level)
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

        // Collect upvalues (using real Lua level)
        let mut upvalues = Vec::new();
        let upvalue_count = state.upvalue_count(level);
        for i in 1..=upvalue_count {
            if let Some((name, value)) = state.get_upvalue(level, i) {
                upvalues.push(make_variable(&name, &value, 1, &mut cache_id));
            }
        }

        stacks.push(Stack {
            file,
            function_name: func_name,
            line,
            level: frame_id,
            local_variables: locals,
            upvalue_variables: upvalues,
        });
        level_map.push(level);
        frame_id += 1;
    }

    (stacks, level_map)
}

/// Performance-optimized breakpoint check.
///
/// Strategy (cheapest to most expensive):
/// 1. Check if `line` is in the breakpoint line set (O(1) HashSet lookup)
/// 2. Only if the line matches, get the source name (cheap: chunk pointer dereference)
/// 3. Match source + line against the breakpoint map
///
/// `line` is passed directly from the hook callback argument — no debug info needed.
pub fn should_break(
    state: &LuaState,
    line: i32,
    hook_state: &HookState,
    bp_manager: &mut BreakPointManager,
) -> bool {
    // Step 1: fast line-set pre-filter
    if bp_manager.has_line(line) {
        // Step 2: only now get the source (cheap — no full debug info)
        let source = state.get_source(0).unwrap_or_default();

        // Step 3: full source+line match
        if let Some(bp) = bp_manager.find_mut(&source, line) {
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
    }

    // Check stepping state (only if no breakpoint matched)
    match hook_state {
        HookState::Continue => false,
        HookState::Break => true,
        _ => {
            // For step operations we need source and depth
            let source = state.get_source(0).unwrap_or_default();
            let depth = state.call_depth();
            let norm_source = normalize_path(&source);
            hook_state.check(&norm_source, line, depth) == HookAction::Break
        }
    }
}

/// Send a BreakNotify message with the current stacks.
/// Returns the frame-level mapping for eval stack_level translation.
pub fn send_break_notify(state: &LuaState, transporter: &Transporter) -> Vec<usize> {
    let (stacks, level_map) = build_stacks(state);
    let notify = BreakNotify { stacks };
    if let Err(e) = transporter.send(Message::BreakNotify(notify)) {
        eprintln!("[debugger] failed to send BreakNotify: {e}");
    }
    level_map
}
