// Coroutine library - Full implementation
// Implements: create, resume, yield, status, running, wrap, isyieldable

use crate::ThreadId;
use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaResult, LuaState};

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
    let thread_val = vm.create_thread(func);

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
            // Error occurred during resume - get detailed error message
            let error_msg = {
                if let Some(thread) = thread_val.as_thread_mut() {
                    thread.error_msg().to_string()
                } else {
                    String::new()
                }
            };
            let error_str = if error_msg.is_empty() {
                l.create_string(&format!("{:?}", e))
            } else {
                l.create_string(&error_msg)
            };
            l.push_value(LuaValue::boolean(false))?; // success=false
            l.push_value(error_str)?;
            Ok(2)
        }
    }
}

/// coroutine.yield(...) - Yield from current coroutine
fn coroutine_yield(l: &mut LuaState) -> LuaResult<usize> {
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

    if let Some(thread_id) = thread_val.as_thread_id() {
        if thread_id.is_main() {
            // Main thread is always running
            let status_val = l.create_string("running");
            l.push_value(status_val)?;
            return Ok(1);
        }
    }

    // Check if thread exists and get status
    let status_str = if let Some(thread) = thread_val.as_thread_mut() {
        // Thread is suspended if it has frames or stack content
        if thread.call_depth() > 0 {
            "suspended"
        } else if !thread.stack().is_empty() {
            // Has stack but no frames - initial state
            "suspended"
        } else {
            "dead"
        }
    } else {
        "dead"
    };

    let status_val = l.create_string(status_str);
    l.push_value(status_val)?;
    Ok(1)
}

/// coroutine.running() - Get currently running coroutine
fn coroutine_running(l: &mut LuaState) -> LuaResult<usize> {
    // In the main thread, return nil and true
    let thread_id = l.get_thread_id();
    if thread_id.is_main() {
        let main_ptr = l as *mut LuaState;
        l.push_value(LuaValue::thread(ThreadId::main_id(), main_ptr))?;
        l.push_value(LuaValue::boolean(true))?;
        return Ok(2);
    }

    let thread_value = l.vm_mut().object_pool.get_thread_value(thread_id).unwrap();
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
    let thread_val = vm.create_thread(func);

    // Create a C closure with the thread as upvalue
    let wrapper_func = vm.create_c_closure(coroutine_wrap_call, vec![thread_val]);

    l.push_value(wrapper_func)?;
    Ok(1)
}

/// Helper function for coroutine.wrap - called when the wrapper is invoked
fn coroutine_wrap_call(l: &mut LuaState) -> LuaResult<usize> {
    // Get the thread from upvalue
    let mut thread_val = LuaValue::nil();
    if let Some(frame) = l.current_frame() {
        if let Some(func) = frame.func.as_lua_function() {
            if let Some(upval) = func.cached_upvalues().get(0) {
                // Upvalue should be closed with the thread value
                thread_val = upval.get_value(l);
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
        Err(e) => {
            // Error occurred - propagate it
            let error_msg = format!("coroutine error: {:?}", e);
            Err(l.error(error_msg))
        }
    }
}

/// coroutine.isyieldable() - Check if current position can yield
fn coroutine_isyieldable(l: &mut LuaState) -> LuaResult<usize> {
    let id = l.get_thread_id();
    l.push_value(LuaValue::boolean(!id.is_main()))?;
    Ok(1)
}

/// coroutine.close(co) - Close a coroutine, marking it as dead
fn coroutine_close(l: &mut LuaState) -> LuaResult<usize> {
    let thread_val = match l.get_arg(1) {
        Some(t) => t,
        None => {
            return Err(l.error("coroutine.close requires a thread argument".to_string()));
        }
    };

    if !thread_val.is_thread() {
        return Err(l.error("coroutine.close requires a thread argument".to_string()));
    }

    let Some(thread_id) = thread_val.as_thread_id() else {
        return Err(l.error("invalid thread".to_string()));
    };

    if thread_id.is_main() {
        return Err(l.error("cannot close the main thread".to_string()));
    }

    // Clear the thread's stack and frames to mark it as closed
    if let Some(thread) = thread_val.as_thread_mut() {
        thread.stack_truncate(0);
        while thread.call_depth() > 0 {
            thread.pop_frame();
        }
    }

    l.push_value(LuaValue::boolean(true))?;
    Ok(1)
}
