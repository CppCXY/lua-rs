// Coroutine library - Full implementation
// Implements: create, resume, yield, status, running, wrap, isyieldable

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaError, LuaResult, LuaState};

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
    let vm = l.vm_mut();
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
    let vm = l.vm_mut();
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
            // Keep the error_object in the thread so coroutine.close can return it too.
            let error_val = if let Some(thread) = thread_val.as_thread_mut() {
                let err_obj = thread.error_object;
                if !err_obj.is_nil() {
                    err_obj
                } else {
                    // Fallback: create string from error message
                    let msg = thread.get_error_msg(e);
                    if msg.is_empty() {
                        LuaValue::nil()
                    } else {
                        l.create_string(&msg)?.into()
                    }
                }
            } else {
                LuaValue::nil()
            };
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
    let status_str = if let Some(thread) = thread_val.as_thread_mut() {
        if thread.is_main_thread() {
            // Main thread is always running
            let status_val = l.create_string("running")?;
            l.push_value(status_val)?;
            return Ok(1);
        }

        if thread.call_depth() > 0 {
            if thread.is_yielded() {
                "suspended"
            } else {
                // Thread has frames and is not yielded — it's either running
                // or normal (resumed another coroutine and waiting).
                // Compare identity: if calling thread IS this thread, it's "running".
                let is_self = std::ptr::eq(l as *const LuaState, thread as *const LuaState);
                if is_self { "running" } else { "normal" }
            }
        } else if !thread.stack().is_empty() {
            // Has stack but no frames - initial state
            "suspended"
        } else {
            "dead"
        }
    } else {
        "dead"
    };

    let status_val = l.create_string(status_str)?;
    l.push_value(status_val)?;
    Ok(1)
}

/// coroutine.running() - Get currently running coroutine
fn coroutine_running(l: &mut LuaState) -> LuaResult<usize> {
    // In the main thread, return nil and true
    let thread_ptr = unsafe { l.thread_ptr() };
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
    let vm = l.vm_mut();
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
    if let Some(frame) = l.current_frame() {
        if let Some(cclosure) = frame.func.as_cclosure() {
            // Check if it's a C closure (coroutine.wrap creates a C closure)

            if let Some(upval) = cclosure.upvalues().get(0) {
                // Upvalue should be closed with the thread value
                thread_val = upval.clone();
            }
        }
    };

    if !thread_val.is_thread() {
        return Err(l.error("invalid wrapped coroutine".to_string()));
    }

    // Collect arguments
    let args = l.get_args();

    // Resume the coroutine
    let vm = l.vm_mut();
    match vm.resume_thread(thread_val, args) {
        Ok((_finished, results)) => {
            // Success - push all results
            for result in &results {
                l.push_value(*result)?;
            }
            Ok(results.len())
        }
        Err(_e) => {
            // Error occurred — propagate the error object from the child thread
            // directly (like Lua 5.5's auxresume → lua_error).
            if let Some(thread) = thread_val.as_thread_mut() {
                // Get the error object from the child thread
                let err_obj = std::mem::replace(&mut thread.error_object, LuaValue::nil());
                if !err_obj.is_nil() {
                    l.error_object = err_obj;
                    let msg = std::mem::take(&mut thread.error_msg);
                    l.error_msg = msg;
                } else {
                    let msg = std::mem::take(&mut thread.error_msg);
                    l.error_msg = msg;
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
            let thread_ptr = unsafe { l.thread_ptr() };
            LuaValue::thread(thread_ptr)
        }
    };

    // Clear the thread's stack and frames to mark it as closed
    if let Some(thread) = thread_val.as_thread_mut() {
        // Determine status (matches C Lua's auxstatus)
        let is_self = std::ptr::eq(l as *const LuaState, thread as *const LuaState);
        let status = if is_self {
            "running" // COS_RUN: L == co
        } else if thread.is_yielded() {
            "suspended" // COS_YIELD
        } else if thread.call_depth() > 0 {
            "normal" // COS_NORM: has active frames, not yielded, not self
        } else if !thread.stack().is_empty() {
            "suspended" // Initial state (not started)
        } else {
            "dead" // COS_DEAD
        };

        match status {
            "dead" | "suspended" => {
                // OK to close dead or suspended coroutines.
                // For dead-by-error coroutines, preserve the error.
            }
            "normal" => {
                return Err(l.error(format!("cannot close a {} coroutine", status)));
            }
            "running" => {
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
            _ => unreachable!(),
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
                if !thread.error_object.is_nil() {
                    let err_obj = std::mem::replace(&mut thread.error_object, LuaValue::nil());
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
                let err_obj = std::mem::replace(&mut thread.error_object, LuaValue::nil());
                let error_val = if !err_obj.is_nil() {
                    err_obj
                } else {
                    let msg = std::mem::take(&mut thread.error_msg);
                    if msg.is_empty() {
                        LuaValue::nil()
                    } else {
                        l.create_string(&msg)?.into()
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
