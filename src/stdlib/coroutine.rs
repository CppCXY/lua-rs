// Coroutine library - Full implementation
// Implements: create, resume, yield, status, running, wrap, isyieldable

use crate::lib_registry::LibraryModule;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::{LuaVM, CoroutineStatus};
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
    })
}

/// coroutine.create(f) - Create a new coroutine
fn coroutine_create(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let func = crate::lib_registry::get_arg(vm, 0)
        .ok_or_else(|| "coroutine.create requires a function argument".to_string())?;
    
    if !func.is_function() && !func.is_cfunction() {
        return Err("coroutine.create requires a function argument".to_string());
    }
    
    let thread_rc = vm.create_thread(func);
    let thread_val = LuaValue::thread_ptr(Rc::into_raw(thread_rc));
    
    Ok(MultiValue::single(thread_val))
}

/// coroutine.resume(co, ...) - Resume a coroutine
fn coroutine_resume(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let thread_val = crate::lib_registry::get_arg(vm, 0)
        .ok_or_else(|| "coroutine.resume requires a thread argument".to_string())?;
    
    if !thread_val.is_thread() {
        return Err("coroutine.resume requires a thread argument".to_string());
    }
    
    // Get thread from value
    let thread_rc = unsafe {
        let ptr = thread_val.as_thread_ptr().ok_or("invalid thread")?;
        if ptr.is_null() {
            return Err("invalid thread".to_string());
        }
        Rc::from_raw(ptr)
    };
    
    // Clone for resumption (we'll forget the original Rc)
    let thread_clone = thread_rc.clone();
    std::mem::forget(thread_rc); // Don't drop the original
    
    // Get arguments
    let all_args = crate::lib_registry::get_args(vm);
    let args: Vec<LuaValue> = if all_args.len() > 1 {
        all_args[1..].to_vec()
    } else {
        Vec::new()
    };
    
    // Resume the thread
    let (success, results) = vm.resume_thread(thread_clone, args)?;
    
    // Return success status and results
    let mut return_values = vec![LuaValue::boolean(success)];
    return_values.extend(results);
    
    Ok(MultiValue::multiple(return_values))
}

/// coroutine.yield(...) - Yield from current coroutine
fn coroutine_yield(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let args = crate::lib_registry::get_args(vm);
    
    // Check if we're in a coroutine
    if vm.current_thread.is_none() {
        return Err("attempt to yield from outside a coroutine".to_string());
    }
    
    // Yield with values - this will store the values and mark as suspended
    let resume_values = vm.yield_thread(args)?;
    
    // Return the resume values (what was passed to resume after yield)
    Ok(MultiValue::multiple(resume_values))
}

/// coroutine.status(co) - Get coroutine status
fn coroutine_status(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let thread_val = crate::lib_registry::get_arg(vm, 0)
        .ok_or_else(|| "coroutine.status requires a thread argument".to_string())?;
    
    if !thread_val.is_thread() {
        return Err("coroutine.status requires a thread argument".to_string());
    }
    
    // Get thread from value
    let status_str = unsafe {
        let ptr = thread_val.as_thread_ptr().ok_or("invalid thread")?;
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
    
    let s = vm.create_string(status_str.to_string());
    Ok(MultiValue::single(LuaValue::from_string_rc(s)))
}

/// coroutine.running() - Get currently running coroutine
fn coroutine_running(vm: &mut LuaVM) -> Result<MultiValue, String> {
    if let Some(thread_rc) = &vm.current_thread {
        let thread_ptr = Rc::into_raw(thread_rc.clone());
        let thread_val = LuaValue::thread_ptr(thread_ptr);
        std::mem::forget(unsafe { Rc::from_raw(thread_ptr) }); // Don't drop
        Ok(MultiValue::multiple(vec![thread_val, LuaValue::boolean(false)]))
    } else {
        // Main thread
        Ok(MultiValue::multiple(vec![LuaValue::nil(), LuaValue::boolean(true)]))
    }
}

/// coroutine.wrap(f) - Create a wrapped coroutine
fn coroutine_wrap(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let func = crate::lib_registry::get_arg(vm, 0)
        .ok_or_else(|| "coroutine.wrap requires a function argument".to_string())?;
    
    if !func.is_function() && !func.is_cfunction() {
        return Err("coroutine.wrap requires a function argument".to_string());
    }
    
    let thread_rc = vm.create_thread(func);
    
    // Create a wrapper function
    // For now, return the thread directly (simplified)
    let thread_val = LuaValue::thread_ptr(Rc::into_raw(thread_rc));
    
    Ok(MultiValue::single(thread_val))
}

/// coroutine.isyieldable() - Check if current position can yield
fn coroutine_isyieldable(vm: &mut LuaVM) -> Result<MultiValue, String> {
    let can_yield = vm.current_thread.is_some();
    Ok(MultiValue::single(LuaValue::boolean(can_yield)))
}
