//! Core debugger state and message dispatch.

pub mod breakpoint;
pub mod eval;
pub mod hook;
pub mod variables;

use std::sync::{Arc, Condvar, Mutex};

use crate::hook_state::HookState;
use crate::proto::*;
use crate::transporter::Transporter;

use breakpoint::BreakPointManager;

/// Shared debugger state, protected by a mutex.
/// Accessed by both the Lua thread (hook callback) and the receiver thread.
pub struct DebuggerState {
    /// Current hook/stepping state.
    pub hook_state: HookState,
    /// Whether the IDE is ready.
    pub ide_ready: bool,
    /// Whether we are currently paused in debug mode.
    pub in_debug_mode: bool,
    /// Breakpoint manager.
    pub bp_manager: BreakPointManager,
    /// Pending eval requests (queued by receiver thread, consumed by Lua thread).
    pub eval_queue: Vec<EvalReqBody>,
    /// File extensions to consider.
    pub file_extensions: Vec<String>,
    /// Whether debugging has started (InitReq received).
    pub started: bool,
    /// Whether a stop has been requested.
    pub stop_requested: bool,
}

impl DebuggerState {
    fn new() -> Self {
        Self {
            hook_state: HookState::Continue,
            ide_ready: false,
            in_debug_mode: false,
            bp_manager: BreakPointManager::new(),
            eval_queue: Vec::new(),
            file_extensions: vec!["lua".to_string()],
            started: false,
            stop_requested: false,
        }
    }
}

/// The main Debugger struct. Shared via `Arc` between threads.
pub struct Debugger {
    pub transporter: Arc<Transporter>,
    pub state: Arc<Mutex<DebuggerState>>,
    /// Used to wake the Lua thread when an action arrives.
    pub action_cv: Arc<(Mutex<bool>, Condvar)>,
    /// Used to block the Lua thread until IDE is ready.
    pub ide_ready_cv: Arc<(Mutex<bool>, Condvar)>,
}

impl Debugger {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            transporter: Arc::new(Transporter::new()),
            state: Arc::new(Mutex::new(DebuggerState::new())),
            action_cv: Arc::new((Mutex::new(false), Condvar::new())),
            ide_ready_cv: Arc::new((Mutex::new(false), Condvar::new())),
        })
    }

    // ============ Connection ============

    /// Start TCP listener and wait for IDE to connect.
    pub fn tcp_listen(&self, host: &str, port: u16) -> std::io::Result<()> {
        self.transporter.listen(host, port)
    }

    /// Connect to IDE TCP server.
    pub fn tcp_connect(&self, host: &str, port: u16) -> std::io::Result<()> {
        self.transporter.connect(host, port)
    }

    // ============ IDE Ready ============

    /// Block until the IDE sends ReadyReq.
    /// If `skip` is true, don't block (used for tcpConnect mode where IDE is already ready).
    pub fn wait_ide(&self, skip: bool) {
        if skip {
            let mut s = self.state.lock().unwrap();
            s.ide_ready = true;
            return;
        }
        let (lock, cv) = &*self.ide_ready_cv;
        let mut ready = lock.lock().unwrap();
        while !*ready {
            ready = cv.wait(ready).unwrap();
        }
    }

    // ============ Debug mode (pause Lua thread) ============

    /// Enter debug mode: send BreakNotify, then block until an action arrives.
    /// While blocked, process eval requests from the queue.
    pub fn enter_debug_mode(&self, state: &mut luars::LuaState) {
        {
            let mut s = self.state.lock().unwrap();
            s.in_debug_mode = true;
        }

        // Send break notification with stacks
        hook::send_break_notify(state, &self.transporter);

        // Block and process eval queue
        loop {
            // Process any pending evals
            let evals: Vec<EvalReqBody> = {
                let mut s = self.state.lock().unwrap();
                std::mem::take(&mut s.eval_queue)
            };
            for req in &evals {
                eval::handle_eval(state, req, &self.transporter);
            }

            // Check if we should resume
            {
                let s = self.state.lock().unwrap();
                if !s.in_debug_mode {
                    break;
                }
            }

            // Wait for action or eval
            let (lock, cv) = &*self.action_cv;
            let mut signaled = lock.lock().unwrap();
            if !*signaled {
                // Wait with a timeout so we can check for evals periodically
                let result = cv
                    .wait_timeout(signaled, std::time::Duration::from_millis(50))
                    .unwrap();
                signaled = result.0;
            }
            *signaled = false;
        }
    }

    // ============ Message dispatch (called from receiver thread) ============

    /// Dispatch a received message by its cmd number.
    pub fn on_message(&self, cmd: i32, json: &str) {
        let msg_cmd = MessageCMD::from_i32(cmd);
        match msg_cmd {
            MessageCMD::InitReq => self.on_init_req(json),
            MessageCMD::ReadyReq => self.on_ready_req(),
            MessageCMD::AddBreakPointReq => self.on_add_breakpoint(json),
            MessageCMD::RemoveBreakPointReq => self.on_remove_breakpoint(json),
            MessageCMD::ActionReq => self.on_action_req(json),
            MessageCMD::EvalReq => self.on_eval_req(json),
            _ => {
                eprintln!("[debugger] unhandled cmd: {cmd}");
            }
        }
    }

    fn on_init_req(&self, json: &str) {
        if let Ok(req) = serde_json::from_str::<InitReqBody>(json) {
            let mut s = self.state.lock().unwrap();
            s.started = true;
            if !req.ext.is_empty() {
                s.file_extensions = req.ext;
            }
            eprintln!(
                "[debugger] InitReq received, extensions: {:?}",
                s.file_extensions
            );
        }
    }

    fn on_ready_req(&self) {
        {
            let mut s = self.state.lock().unwrap();
            s.ide_ready = true;
        }
        // Wake WaitIDE
        let (lock, cv) = &*self.ide_ready_cv;
        let mut ready = lock.lock().unwrap();
        *ready = true;
        cv.notify_all();
        eprintln!("[debugger] ReadyReq received, IDE is ready");
    }

    fn on_add_breakpoint(&self, json: &str) {
        if let Ok(req) = serde_json::from_str::<AddBreakPointReqBody>(json) {
            let mut s = self.state.lock().unwrap();
            if req.clear {
                s.bp_manager.clear();
            }
            for bp_proto in req.break_points {
                eprintln!(
                    "[debugger] add breakpoint: {}:{}",
                    bp_proto.file, bp_proto.line
                );
                s.bp_manager.add(bp_proto.into());
            }
        }
    }

    fn on_remove_breakpoint(&self, json: &str) {
        if let Ok(req) = serde_json::from_str::<RemoveBreakPointReqBody>(json) {
            let mut s = self.state.lock().unwrap();
            for bp_proto in &req.break_points {
                s.bp_manager.remove(&bp_proto.file, bp_proto.line);
            }
        }
    }

    fn on_action_req(&self, json: &str) {
        if let Ok(req) = serde_json::from_str::<ActionReqBody>(json) {
            let action = DebugAction::from_i32(req.action);
            eprintln!("[debugger] ActionReq: {action:?}");

            let mut s = self.state.lock().unwrap();
            match action {
                DebugAction::Continue => {
                    s.hook_state = HookState::Continue;
                    s.in_debug_mode = false;
                }
                DebugAction::StepIn => {
                    // StepIn: will be configured with origin in the hook
                    s.hook_state = HookState::Break;
                    s.in_debug_mode = false;
                }
                DebugAction::StepOver => {
                    // StepOver: will be configured with origin depth in the hook
                    // For now, set Break and let the hook refine it
                    s.hook_state = HookState::Break;
                    s.in_debug_mode = false;
                }
                DebugAction::StepOut => {
                    // StepOut: will be configured with origin depth in the hook
                    s.hook_state = HookState::Break;
                    s.in_debug_mode = false;
                }
                DebugAction::Stop => {
                    s.hook_state = HookState::Continue;
                    s.stop_requested = true;
                    s.in_debug_mode = false;
                }
                DebugAction::Break => {
                    s.hook_state = HookState::Break;
                    s.in_debug_mode = false;
                }
                DebugAction::None => {}
            }
            drop(s);

            // Wake the Lua thread
            let (lock, cv) = &*self.action_cv;
            let mut signaled = lock.lock().unwrap();
            *signaled = true;
            cv.notify_all();
        }
    }

    fn on_eval_req(&self, json: &str) {
        if let Ok(req) = serde_json::from_str::<EvalReqBody>(json) {
            let mut s = self.state.lock().unwrap();
            s.eval_queue.push(req);
            drop(s);

            // Wake Lua thread to process eval
            let (lock, cv) = &*self.action_cv;
            let mut signaled = lock.lock().unwrap();
            *signaled = true;
            cv.notify_all();
        }
    }

    // ============ Receiver thread ============

    /// Start the message receiver loop in a background thread.
    pub fn start_receiver(self: &Arc<Self>) {
        let dbg = Arc::clone(self);
        std::thread::spawn(move || {
            dbg.transporter.receive_loop(|cmd, json| {
                dbg.on_message(cmd, json);
            });
            eprintln!("[debugger] receiver thread exiting");
            // Connection lost — clean up
            let mut s = dbg.state.lock().unwrap();
            s.ide_ready = false;
            s.in_debug_mode = false;
            s.bp_manager.clear();
            s.started = false;
            drop(s);
            // Wake any blocked threads
            {
                let (lock, cv) = &*dbg.action_cv;
                let mut signaled = lock.lock().unwrap();
                *signaled = true;
                cv.notify_all();
            }
            {
                let (lock, cv) = &*dbg.ide_ready_cv;
                let mut ready = lock.lock().unwrap();
                *ready = true;
                cv.notify_all();
            }
        });
    }
}
