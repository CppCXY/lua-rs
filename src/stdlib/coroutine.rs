// Coroutine library - Full implementation
// Implements: create, resume, yield, status, running, wrap, isyieldable

use crate::lib_registry::{LibraryModule, get_args, require_arg};
use crate::lua_value::{CoroutineStatus, LuaValue, MultiValue};
use crate::lua_vm::{LuaError, LuaResult, LuaVM};
use std::rc::Rc;

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
    let func = require_arg(vm, 0, "coroutine.create")?;

    if !func.is_function() && !func.is_cfunction() {
        return Err(LuaError::RuntimeError(
            "coroutine.create requires a function argument".to_string(),
        ));
    }

    let thread_rc = vm.create_thread(func);
    let thread_val = LuaValue::thread_ptr(Rc::into_raw(thread_rc));

    Ok(MultiValue::single(thread_val))
}

/// coroutine.resume(co, ...) - Resume a coroutine
fn coroutine_resume(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let thread_val = require_arg(vm, 0, "coroutine.resume")?;

    if !thread_val.is_thread() {
        return Err(LuaError::RuntimeError(
            "coroutine.resume requires a thread argument".to_string(),
        ));
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
    let args = crate::lib_registry::get_args(vm);

    // Check if we're in a coroutine
    if vm.current_thread.is_none() {
        return Err(LuaError::RuntimeError(
            "attempt to yield from outside a coroutine".to_string(),
        ));
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
    let thread_val = require_arg(vm, 0, "coroutine.status")?;

    if !thread_val.is_thread() {
        return Err(LuaError::RuntimeError(
            "coroutine.status requires a thread argument".to_string(),
        ));
    }

    // Get thread from value
    let status_str = unsafe {
        let ptr = thread_val
            .as_thread_ptr()
            .ok_or(LuaError::RuntimeError("invalid thread".to_string()))?;
        if ptr.is_null() {
            "dead"
        } else {
            let thread_rc = Rc::from_raw(ptr);
            let status = thread_rc.borrow().status;
            let result = match status {
                CoroutineStatus::Suspended => "suspended",
                CoroutineStatus::Running => "running",
                CoroutineStatus::Normal => "normal",
                CoroutineStatus::Dead => "dead",
            };
            std::mem::forget(thread_rc); // Don't drop
            result
        }
    };

    let s = vm.create_string(status_str);
    Ok(MultiValue::single(s))
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
            // Create a dummy thread for main thread representation
            let dummy_func = LuaValue::nil();
            let main_thread_rc = vm.create_thread(dummy_func);
            vm.main_thread_value = Some(LuaValue::thread_ptr(Rc::into_raw(main_thread_rc)));
        }

        Ok(MultiValue::multiple(vec![
            vm.main_thread_value.as_ref().unwrap().clone(),
            LuaValue::boolean(true),
        ]))
    }
}

/// coroutine.wrap(f) - Create a wrapped coroutine
/// This is placeholder - the actual implementation is injected as Lua code in lib_registry
fn coroutine_wrap(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let func = require_arg(vm, 0, "coroutine.wrap")?;

    if !func.is_function() && !func.is_cfunction() {
        return Err(LuaError::RuntimeError(
            "coroutine.wrap requires a function argument".to_string(),
        ));
    }

    // Create the coroutine (same as coroutine.create)
    let thread_rc = vm.create_thread(func);
    let thread_val = LuaValue::thread_ptr(Rc::into_raw(thread_rc));
    
    // Create a wrapper table that will act as a callable object
    let wrapper_table = vm.create_table();
    
    // Store the coroutine in the table
    let thread_key = vm.create_string("__thread");
    vm.table_set_with_meta(wrapper_table, thread_key, thread_val)?;
    
    // Create the __call metamethod
    let call_func = LuaValue::cfunction(coroutine_wrap_call);
    
    // Create and set metatable
    let metatable = vm.create_table();
    let call_key = vm.create_string("__call");
    vm.table_set_with_meta(metatable, call_key, call_func)?;
    
    // Set metatable on wrapper table
    let table_ref = vm.get_table(&wrapper_table)
        .ok_or(LuaError::RuntimeError("Invalid table".to_string()))?;
    table_ref.borrow_mut().set_metatable(Some(metatable));
    
    Ok(MultiValue::single(wrapper_table))
}

/// Helper function for coroutine.wrap - called when the wrapper is invoked
fn coroutine_wrap_call(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // First argument is the wrapper table itself (self)
    let frame = vm.frames.last().ok_or_else(|| {
        LuaError::RuntimeError("no active frame".to_string())
    })?;
    let base = frame.base_ptr;
    let top = frame.top;
    
    if top < 1 {
        return Err(LuaError::RuntimeError(
            "coroutine.wrap call requires self argument".to_string(),
        ));
    }
    
    let wrapper_table = vm.register_stack[base];
    
    // Get the stored coroutine
    let thread_key = vm.create_string("__thread");
    let thread_val = vm.table_get_with_meta(&wrapper_table, &thread_key)
        .ok_or_else(|| LuaError::RuntimeError("coroutine not found in wrapper".to_string()))?;
    
    // Collect arguments (skip self at index 0)
    let mut args = Vec::new();
    for i in 1..top {
        args.push(vm.register_stack[base + i]);
    }
    
    // Resume the coroutine
    let (success, results) = vm.resume_thread(thread_val, args)?;
    
    if !success {
        // If resume failed, propagate the error
        if !results.is_empty() {
            if let Some(err_msg) = unsafe { results[0].as_string() } {
                return Err(LuaError::RuntimeError(err_msg.as_str().to_string()));
            }
        }
        return Err(LuaError::RuntimeError("coroutine error".to_string()));
    }
    
    // Return results as MultiValue
    Ok(MultiValue::multiple(results))
}

/// coroutine.isyieldable() - Check if current position can yield
fn coroutine_isyieldable(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let can_yield = vm.current_thread.is_some();
    Ok(MultiValue::single(LuaValue::boolean(can_yield)))
}

/// coroutine.close(co) - Close a coroutine, marking it as dead
fn coroutine_close(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let thread_val = require_arg(vm, 0, "coroutine.close")?;

    if !thread_val.is_thread() {
        return Err(LuaError::RuntimeError(
            "coroutine.close requires a thread argument".to_string(),
        ));
    }

    // Get thread from value
    unsafe {
        let ptr = thread_val
            .as_thread_ptr()
            .ok_or(LuaError::RuntimeError("invalid thread".to_string()))?;
        if ptr.is_null() {
            return Err(LuaError::RuntimeError(
                "cannot close dead coroutine".to_string(),
            ));
        }

        let thread_rc = Rc::from_raw(ptr);

        // Check if already dead
        let status = thread_rc.borrow().status;
        if matches!(status, CoroutineStatus::Dead) {
            std::mem::forget(thread_rc);
            return Err(LuaError::RuntimeError(
                "cannot close dead coroutine".to_string(),
            ));
        }

        // Check if running
        if matches!(status, CoroutineStatus::Running) {
            std::mem::forget(thread_rc);
            return Err(LuaError::RuntimeError(
                "cannot close running coroutine".to_string(),
            ));
        }

        // Mark as dead
        thread_rc.borrow_mut().status = CoroutineStatus::Dead;

        std::mem::forget(thread_rc);
    }

    Ok(MultiValue::multiple(vec![LuaValue::boolean(true)]))
}
