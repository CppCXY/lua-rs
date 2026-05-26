// Coroutine library - Full implementation
// Implements: create, resume, yield, status, running, wrap, isyieldable

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{ErrorMsg, LuaError, LuaResult, LuaState};

pub fn create_coroutine_lib() -> LibraryModule {
    crate::lib_module!("coroutine", {
        "create" => coroutine_create,
        "resume" => coroutine_resume,
        "yield" => coroutine_yield,
        "status" => coroutine_status,
        "running" => coroutine_running,
        "wrap" => coroutine_wrap,
        "isyieldable" => coroutine_isyieldable,
        "close" => coroutine_close,
    })
}

/// coroutine.create(f) - Create a new coroutine
fn coroutine_create(l: &mut LuaState) -> LuaResult<usize> {
    let func = match l.get_arg(1) {
        Some(f) => f,
        None => {
            return Err(l.error("coroutine.create requires a function argument".to_string()));
        }
    };

    if !func.is_function() && !func.is_cfunction() {
        return Err(l.error("coroutine.create requires a function argument".to_string()));
    }

    // Use VM's create_thread which properly sets up the thread with the function
    let vm = l.global_state_mut();
    let thread_val = vm.create_thread(func)?;

    l.push_value(thread_val)?;
    Ok(1)
}

/// coroutine.resume(co, ...) - Resume a coroutine
fn coroutine_resume(l: &mut LuaState) -> LuaResult<usize> {
    let thread_val = match l.get_arg(1) {
        Some(t) => t,
        None => {
            return Err(l.error("coroutine.resume requires a thread argument".to_string()));
        }
    };

    if !thread_val.is_thread() {
        return Err(l.error("coroutine.resume requires a thread argument".to_string()));
    }

    // Get remaining arguments
    let all_args = l.get_args();
    let args: Vec<LuaValue> = if all_args.len() > 1 {
        all_args[1..].to_vec()
    } else {
        Vec::new()
    };

    // Resume the thread
    let vm = l.global_state_mut();
    match vm.resume_thread(thread_val, args) {
        Ok((_finished, results)) => {
            // Success - either yielded (finished=false) or completed (finished=true)
            // Both are successful from pcall perspective
            let result_count = results.len();
            l.push_value(LuaValue::boolean(true))?; // success=true
            for result in results {
                l.push_value(result)?;
            }
            Ok(1 + result_count)
        }
        Err(e) => {
            // Error occurred during resume — return (false, error_object)
            // Like C Lua, return the actual error value (not a string conversion).
            // Dead coroutines archive their terminal error locally; clear the
            // shared active error afterwards so later unrelated calls are not poisoned.
            let error_val = if let Some(thread) = thread_val.as_thread_mut() {
                let active_err_obj = thread.error_object();
                let active_msg = thread.get_error_msg(e);
                if !active_err_obj.is_nil() {
                    active_err_obj
                } else if !active_msg.is_empty() {
                    l.create_raw_string(&active_msg)?
                } else {
                    match thread.dead_error() {
                        ErrorMsg::Object(obj) => *obj,
                        ErrorMsg::Msg(msg) if !msg.is_empty() => l.create_raw_string(msg)?,
                        _ => LuaValue::nil(),
                    }
                }
            } else {
                LuaValue::nil()
            };
            l.clear_error();
            l.push_value(LuaValue::boolean(false))?; // success=false
            l.push_value(error_val)?;
            Ok(2)
        }
    }
}

/// coroutine.yield(...) - Yield from current coroutine
fn coroutine_yield(l: &mut LuaState) -> LuaResult<usize> {
    // Check if yielding is allowed (matches C Lua's lua_yieldk check)
    if l.nny > 0 {
        if l.is_main_thread() {
            return Err(l.error("attempt to yield from outside a coroutine".to_string()));
        } else {
            return Err(l.error("attempt to yield across a C-call boundary".to_string()));
        }
    }

    let args = l.get_args();

    // Yield with values
    l.do_yield(args)?;

    // This return value won't be used because do_yield returns Err(LuaError::Yield)
    Ok(0)
}

/// coroutine.status(co) - Get coroutine status
fn coroutine_status(l: &mut LuaState) -> LuaResult<usize> {
    let thread_val = match l.get_arg(1) {
        Some(t) => t,
        None => {
            return Err(l.error("coroutine.status requires a thread argument".to_string()));
        }
    };

    if !thread_val.is_thread() {
        return Err(l.error("coroutine.status requires a thread argument".to_string()));
    }

    // Check if thread exists and get status
    // Pre-read const strings before mutable borrow of thread
    let cs = &l.global_state_mut().const_strings;
    let str_running = cs.str_running;
    let str_suspended = cs.str_suspended;
    let str_normal = cs.str_normal;
    let str_dead = cs.str_dead;

    let status_val = if let Some(thread) = thread_val.as_thread_mut() {
        if thread.is_main_thread() {
            // Main thread is always running
            str_running
        } else if thread.dead {
            // Dead by error — still has stack/frames for debug.traceback
            str_dead
        } else if thread.call_depth() > 0 {
            if thread.is_yielded() {
                str_suspended
            } else {
                // Thread has frames and is not yielded — it's either running
                // or normal (resumed another coroutine and waiting).
                // Compare identity: if calling thread IS this thread, it's "running".
                let is_self = std::ptr::eq(l as *const LuaState, thread as *const LuaState);
                if is_self { str_running } else { str_normal }
            }
        } else if !thread.stack().is_empty() {
            // Has stack but no frames - initial state
            str_suspended
        } else {
            str_dead
        }
    } else {
        str_dead
    };

    l.push_value(status_val)?;
    Ok(1)
}

/// coroutine.running() - Get currently running coroutine
fn coroutine_running(l: &mut LuaState) -> LuaResult<usize> {
    // In the main thread, return nil and true
    let thread_ptr = l.thread_ptr();
    if l.is_main_thread() {
        l.push_value(LuaValue::thread(thread_ptr))?;
        l.push_value(LuaValue::boolean(true))?;
        return Ok(2);
    }

    let thread_value = LuaValue::thread(thread_ptr);
    l.push_value(thread_value)?;
    l.push_value(LuaValue::boolean(false))?;
    Ok(2)
}

/// coroutine.wrap(f) - Create a wrapped coroutine
fn coroutine_wrap(l: &mut LuaState) -> LuaResult<usize> {
    let func = match l.get_arg(1) {
        Some(f) => f,
        None => {
            return Err(l.error("coroutine.wrap requires a function argument".to_string()));
        }
    };

    if !func.is_function() && !func.is_cfunction() {
        return Err(l.error("coroutine.wrap requires a function argument".to_string()));
    }

    // Create the coroutine
    let vm = l.global_state_mut();
    let thread_val = vm.create_thread(func)?;

    // Create a C closure with the thread as upvalue
    let wrapper_func = vm.create_c_closure(coroutine_wrap_call, vec![thread_val])?;

    l.push_value(wrapper_func)?;
    Ok(1)
}

/// Helper function for coroutine.wrap - called when the wrapper is invoked
fn coroutine_wrap_call(l: &mut LuaState) -> LuaResult<usize> {
    // Get the thread from upvalue
    let mut thread_val = LuaValue::nil();
    if let Some(frame_idx) = l.call_depth().checked_sub(1)
        && let Some(func_val) = l.get_frame_func(frame_idx)
        && let Some(cclosure) = func_val.as_cclosure()
    {
        // Check if it's a C closure (coroutine.wrap creates a C closure)

        if let Some(upval) = cclosure.upvalues().first() {
            // Upvalue should be closed with the thread value
            thread_val = *upval;
        }
    }

    if !thread_val.is_thread() {
        return Err(l.error("invalid wrapped coroutine".to_string()));
    }

    // Collect arguments
    let args = l.get_args();

    // Resume the coroutine
    let vm = l.global_state_mut();
    match vm.resume_thread(thread_val, args) {
        Ok((_finished, results)) => {
            // Success - push all results
            for result in &results {
                l.push_value(*result)?;
            }
            Ok(results.len())
        }
        Err(e) => {
            // Match Lua's coroutine.wrap semantics: propagate the wrapped
            // coroutine's actual error value, including dead-coroutine errors
            // archived on the thread after resume_thread unwinds.
            if let Some(thread) = thread_val.as_thread_mut() {
                let has_active_err_obj = thread.has_error_object();
                let active_err_obj = thread.error_object();
                if has_active_err_obj {
                    let _ = thread.get_error_msg(e);
                    return Err(l.error_with_object(active_err_obj));
                }

                let active_msg = thread.get_error_msg(e);
                if !active_msg.is_empty() {
                    let err_str = l.create_raw_string(&active_msg)?;
                    return Err(l.error_with_object(err_str));
                }

                match thread.dead_error() {
                    ErrorMsg::Object(obj) => return Err(l.error_with_object(*obj)),
                    ErrorMsg::Msg(msg) if !msg.is_empty() => {
                        let err_str = l.create_raw_string(msg)?;
                        return Err(l.error_with_object(err_str));
                    }
                    _ => {}
                }
            }

            Err(LuaError::RuntimeError)
        }
    }
}

/// coroutine.isyieldable([co]) - Check if the given coroutine (or current) can yield
/// Returns true iff nny == 0 (not inside a non-yieldable C call boundary).
fn coroutine_isyieldable(l: &mut LuaState) -> LuaResult<usize> {
    // If a thread argument is given, check that thread; otherwise check current
    let is_yieldable = if let Some(arg) = l.get_arg(1) {
        if let Some(thread) = arg.as_thread_mut() {
            thread.nny == 0
        } else {
            return Err(l.error("value is not a thread".to_string()));
        }
    } else {
        l.nny == 0
    };
    l.push_value(LuaValue::boolean(is_yieldable))?;
    Ok(1)
}

enum CloseStatus {
    Dead = 0,
    Suspended = 1,
    Normal = 2,
    Running = 3,
}

/// coroutine.close([co]) - Close a coroutine, marking it as dead
/// If no argument, closes the calling thread (self).
/// Calls __close on any pending to-be-closed variables, then kills the thread.
fn coroutine_close(l: &mut LuaState) -> LuaResult<usize> {
    // getoptco: if no argument, use the calling thread itself
    let thread_val = match l.get_arg(1) {
        Some(t) if t.is_thread() => t,
        Some(t) if !t.is_nil() => {
            return Err(l.error("bad argument #1 to 'close' (coroutine expected)".to_string()));
        }
        _ => {
            // No argument or nil — close self
            let thread_ptr = l.thread_ptr();
            LuaValue::thread(thread_ptr)
        }
    };

    // Clear the thread's stack and frames to mark it as closed
    if let Some(thread) = thread_val.as_thread_mut() {
        // Determine status (matches C Lua's auxstatus)
        let is_self = std::ptr::eq(l as *const LuaState, thread as *const LuaState);
        // 0 = dead, 1 = suspended, 2 = normal, 3 = running
        let status: CloseStatus = if is_self {
            CloseStatus::Running
        } else if thread.dead {
            CloseStatus::Dead
        } else if thread.is_yielded() {
            CloseStatus::Suspended
        } else if thread.call_depth() > 0 {
            CloseStatus::Normal
        } else if !thread.stack().is_empty() {
            CloseStatus::Suspended
        } else {
            CloseStatus::Dead
        };

        match status {
            CloseStatus::Dead | CloseStatus::Suspended => {
                // OK to close dead or suspended coroutines.
                // For dead-by-error coroutines, preserve the error.
            }
            CloseStatus::Normal => {
                return Err(l.error("cannot close a normal coroutine".to_string()));
            }
            CloseStatus::Running => {
                if thread.is_main_thread() {
                    return Err(l.error("cannot close main thread".to_string()));
                }
                // Check if this is a re-entrant close (from __close handler)
                if l.is_closing {
                    // Nested close during __close processing — return success (no-op).
                    // The outer close is already handling TBC variables.
                    l.push_value(LuaValue::boolean(true))?;
                    return Ok(1);
                }
                // Direct self-close from within the coroutine's code.
                // Equivalent to C Lua's luaE_resetthread + luaD_throwbaselevel:
                // close TBC vars and upvalues, then throw CloseThread which
                // bypasses all pcalls and goes directly to resume().
                l.is_closing = true;
                let _ = l.close_tbc_with_error(0, LuaValue::nil());
                l.close_upvalues(0);
                l.is_closing = false;
                // error_object is set by close_tbc_with_error if __close errored
                // (nil = success, non-nil = __close error value).
                // CloseThread bypasses pcall and propagates to resume.
                return Err(LuaError::CloseThread);
            }
        }

        // Close all pending to-be-closed variables (calls __close metamethods
        // on the coroutine's thread).  The shared VM n_ccalls counter will
        // correctly track recursion depth because all threads share the same
        // LuaVM instance.
        let close_result = thread.close_tbc_with_error(0, LuaValue::nil());

        // Close all upvalues
        thread.close_upvalues(0);

        // Pop all frames and truncate stack
        while thread.call_depth() > 0 {
            thread.pop_frame();
        }
        thread.stack_truncate();

        match close_result {
            Ok(()) => {
                // Check if coroutine had a pending error (dead-by-error)
                // or if __close cascaded an error
                if !matches!(thread.dead_error(), ErrorMsg::None) {
                    let err_val = match thread.take_dead_error() {
                        ErrorMsg::Object(obj) => obj,
                        ErrorMsg::Msg(msg) if !msg.is_empty() => l.create_raw_string(&msg)?,
                        _ => LuaValue::nil(),
                    };
                    l.push_value(LuaValue::boolean(false))?;
                    l.push_value(err_val)?;
                    Ok(2)
                } else if !thread.error_object().is_nil() {
                    let err_obj = thread.take_error_object();
                    l.push_value(LuaValue::boolean(false))?;
                    l.push_value(err_obj)?;
                    Ok(2)
                } else {
                    l.push_value(LuaValue::boolean(true))?;
                    Ok(1)
                }
            }
            Err(LuaError::Yield) => {
                // Yield inside __close during coroutine close — propagate
                Err(LuaError::Yield)
            }
            Err(_e) => {
                // __close caused an error — return (false, error_value)
                let err_obj = thread.take_error_object();
                let error_val = if !err_obj.is_nil() {
                    err_obj
                } else {
                    match thread.take_dead_error() {
                        ErrorMsg::Object(obj) => obj,
                        ErrorMsg::Msg(msg) if !msg.is_empty() => l.create_raw_string(&msg)?,
                        _ => {
                            let msg = thread.take_error_msg_raw();
                            if msg.is_empty() {
                                LuaValue::nil()
                            } else {
                                l.create_raw_string(&msg)?
                            }
                        }
                    }
                };
                l.push_value(LuaValue::boolean(false))?;
                l.push_value(error_val)?;
                Ok(2)
            }
        }
    } else {
        l.push_value(LuaValue::boolean(true))?;
        Ok(1)
    }
}
