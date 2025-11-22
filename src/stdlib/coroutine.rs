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
fn coroutine_wrap(_vm: &mut LuaVM) -> LuaResult<MultiValue> {
    Err(LuaError::RuntimeError(
        "coroutine.wrap should be overridden by Lua implementation in lib_registry".to_string(),
    ))
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
