// Coroutine library - Full implementation
// Implements: create, resume, yield, status, running, wrap, isyieldable

use crate::lib_registry::{LibraryModule, get_args, require_arg};
use crate::lua_value::{CoroutineStatus, LuaValue, MultiValue};
use crate::lua_vm::{LuaResult, LuaVM};

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
fn coroutine_create(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let func = require_arg(vm, 1, "coroutine.create")?;

    if !func.is_function() && !func.is_cfunction() {
        return Err(vm.error("coroutine.create requires a function argument".to_string()));
    }

    // Use new ThreadId-based API
    let thread_val = vm.create_thread_value(func);

    Ok(MultiValue::single(thread_val))
}

/// coroutine.resume(co, ...) - Resume a coroutine
fn coroutine_resume(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let thread_val = require_arg(vm, 1, "coroutine.resume")?;

    if !thread_val.is_thread() {
        return Err(vm.error("coroutine.resume requires a thread argument".to_string()));
    }

    // Get arguments
    let all_args = get_args(vm);
    let args: Vec<LuaValue> = if all_args.len() > 1 {
        all_args[1..].to_vec()
    } else {
        Vec::new()
    };

    // Resume the thread (pass LuaValue directly)
    let (success, results) = vm.resume_thread(thread_val, args)?;

    // Return success status and results
    let mut return_values = vec![LuaValue::boolean(success)];
    return_values.extend(results);

    Ok(MultiValue::multiple(return_values))
}

/// coroutine.yield(...) - Yield from current coroutine
fn coroutine_yield(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let args = get_args(vm);

    // Check if we're in a coroutine (use new thread_id based check)
    if vm.current_thread_id.is_none() {
        return Err(vm.error("attempt to yield from outside a coroutine".to_string()));
    }

    // Yield with values - this will store the values and mark as suspended
    vm.yield_thread(args)?;

    // When yielding for the first time, we don't return anything here
    // The return values will be set when resume() is called with new values
    // For now, return empty (but this won't actually be used due to yielding flag)
    Ok(MultiValue::multiple(vm.return_values.clone()))
}

/// coroutine.status(co) - Get coroutine status
fn coroutine_status(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let thread_val = require_arg(vm, 1, "coroutine.status")?;

    if !thread_val.is_thread() {
        return Err(vm.error("coroutine.status requires a thread argument".to_string()));
    }

    // Get thread status using thread_id and pre-cached StringIds
    let status_sid = if let Some(thread_id) = thread_val.as_thread_id() {
        if let Some(thread) = vm.object_pool.get_thread(thread_id) {
            match thread.status {
                CoroutineStatus::Main => vm.object_pool.str_main,
                CoroutineStatus::Suspended => vm.object_pool.str_suspended,
                CoroutineStatus::Running => vm.object_pool.str_running,
                CoroutineStatus::Normal => vm.object_pool.str_normal,
                CoroutineStatus::Dead => vm.object_pool.str_dead,
            }
        } else {
            vm.object_pool.str_dead
        }
    } else {
        vm.object_pool.str_dead
    };

    Ok(MultiValue::single(LuaValue::string(status_sid)))
}

/// coroutine.running() - Get currently running coroutine
fn coroutine_running(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    if let Some(thread_val) = &vm.current_thread_value {
        // Return the stored thread value for proper comparison
        Ok(MultiValue::multiple(vec![
            thread_val.clone(),
            LuaValue::boolean(false),
        ]))
    } else {
        // Main thread - create a dummy thread representation if not exists
        if vm.main_thread_value.is_none() {
            // Create a dummy thread for main thread representation using new API
            let dummy_func = LuaValue::nil();
            let main_thread_val = vm.create_thread_value(dummy_func);
            vm.main_thread_value = Some(main_thread_val);
        }

        Ok(MultiValue::multiple(vec![
            vm.main_thread_value.as_ref().unwrap().clone(),
            LuaValue::boolean(true),
        ]))
    }
}

/// coroutine.wrap(f) - Create a wrapped coroutine
/// OPTIMIZED: Uses C closure with inline upvalue for ultra-fast access
fn coroutine_wrap(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let func = require_arg(vm, 1, "coroutine.wrap")?;

    if !func.is_function() && !func.is_cfunction() {
        return Err(vm.error("coroutine.wrap requires a function argument".to_string()));
    }

    // Create the coroutine
    let thread_val = vm.create_thread_value(func);

    // Get the ThreadId from the thread value
    let Some(thread_id) = thread_val.as_thread_id() else {
        return Err(vm.error("Failed to create coroutine".to_string()));
    };

    // ULTRA-OPTIMIZED: Create C closure with inline upvalue (no indirection)
    // This is the fastest possible path for coroutine.wrap
    let thread_val = LuaValue::thread(thread_id);
    let wrapper = vm.create_c_closure_inline1(coroutine_wrap_call, thread_val);

    Ok(MultiValue::single(wrapper))
}

/// Helper function for coroutine.wrap - called when the wrapper is invoked
/// ULTRA-OPTIMIZED: Thread is stored as inline upvalue, direct value access
fn coroutine_wrap_call(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Get the thread from inline upvalue (no indirection, direct value)
    let thread_val = vm.get_c_closure_inline_upvalue();

    if !thread_val.is_thread() {
        return Err(vm.error("invalid wrapped coroutine".to_string()));
    }

    // Collect arguments (all args go to resume, no self)
    let args = get_args(vm);

    // Resume the coroutine
    let (success, results) = vm.resume_thread(thread_val, args)?;

    if !success {
        // If resume failed, propagate the error
        if !results.is_empty() {
            if let Some(string_id) = results[0].as_string_id() {
                if let Some(err_msg) = vm.object_pool.get_string(string_id) {
                    return Err(vm.error(err_msg.as_str().to_string()));
                }
            }
        }
        return Err(vm.error("coroutine error".to_string()));
    }

    // Return results as MultiValue
    Ok(MultiValue::multiple(results))
}

/// coroutine.isyieldable() - Check if current position can yield
fn coroutine_isyieldable(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let can_yield = vm.current_thread_id.is_some();
    Ok(MultiValue::single(LuaValue::boolean(can_yield)))
}

/// coroutine.close(co) - Close a coroutine, marking it as dead
fn coroutine_close(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let thread_val = require_arg(vm, 1, "coroutine.close")?;

    if !thread_val.is_thread() {
        return Err(vm.error("coroutine.close requires a thread argument".to_string()));
    }

    // Get thread using thread_id
    let Some(thread_id) = thread_val.as_thread_id() else {
        return Err(vm.error("invalid thread".to_string()));
    };

    // Check status first (immutable borrow)
    let status = {
        let Some(thread) = vm.object_pool.get_thread(thread_id) else {
            return Err(vm.error("cannot close dead coroutine".to_string()));
        };
        thread.status
    };

    // Check if already dead
    if matches!(status, CoroutineStatus::Dead) {
        return Err(vm.error("cannot close dead coroutine".to_string()));
    }

    // Check if running
    if matches!(status, CoroutineStatus::Running) {
        return Err(vm.error("cannot close running coroutine".to_string()));
    }

    // Mark as dead (mutable borrow)
    let Some(thread) = vm.object_pool.get_thread_mut(thread_id) else {
        return Err(vm.error("invalid thread".to_string()));
    };
    thread.status = CoroutineStatus::Dead;

    Ok(MultiValue::multiple(vec![LuaValue::boolean(true)]))
}
