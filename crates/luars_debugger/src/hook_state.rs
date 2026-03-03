//! Hook state machine for stepping/continuing.
//!
//! Mirrors EmmyLuaDebugger's HookState* classes: each variant decides
//! whether the debugger should break at the current line-hook event.

/// Result of a hook-state check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookAction {
    /// Do nothing, continue execution.
    Continue,
    /// Enter debug mode (break).
    Break,
}

/// The stepping/continue state machine.
#[derive(Debug, Clone)]
pub enum HookState {
    /// Normal execution – only break on breakpoints.
    Continue,
    /// Break on any next line event.
    Break,
    /// Step Into: break when file or line changes from origin.
    StepIn {
        origin_file: String,
        origin_line: i32,
    },
    /// Step Over: break when at same-or-shallower stack depth AND line changes.
    StepOver {
        origin_depth: usize,
        origin_file: String,
        origin_line: i32,
    },
    /// Step Out: break when stack depth is strictly shallower than origin.
    StepOut { origin_depth: usize },
    /// Stop requested – reset to Continue immediately.
    Stop,
}

impl HookState {
    /// Evaluate whether we should break at the current line event.
    ///
    /// `file`  – source of the current line
    /// `line`  – current line number
    /// `depth` – current call-stack depth
    pub fn check(&self, file: &str, line: i32, depth: usize) -> HookAction {
        match self {
            HookState::Continue => HookAction::Continue,
            HookState::Break => HookAction::Break,
            HookState::StepIn {
                origin_file,
                origin_line,
            } => {
                if file != origin_file || line != *origin_line {
                    HookAction::Break
                } else {
                    HookAction::Continue
                }
            }
            HookState::StepOver {
                origin_depth,
                origin_file,
                origin_line,
            } => {
                if depth <= *origin_depth && (file != origin_file || line != *origin_line) {
                    HookAction::Break
                } else {
                    HookAction::Continue
                }
            }
            HookState::StepOut { origin_depth } => {
                if depth < *origin_depth {
                    HookAction::Break
                } else {
                    HookAction::Continue
                }
            }
            HookState::Stop => {
                // Stop is handled at a higher level; treat as continue here.
                HookAction::Continue
            }
        }
    }
}
