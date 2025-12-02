// Async standard library for Lua
// Provides async functions that can be directly called from Lua coroutines
// Uses tokio as the underlying runtime

use crate::lib_registry::LibraryModule;
use crate::lua_value::MultiValue;
use crate::lua_vm::{LuaError, LuaResult};
use crate::{LuaVM, LuaValue};
use std::time::Duration;
use tokio::time::sleep;

/// Create the async library module with async functions
pub fn create_async_lib() -> LibraryModule {
    let module = crate::lib_module!("async", {
        "sleep" => async_sleep_wrapper,
    });
    module
}

/// Register async functions to the executor
pub fn register_async_functions(vm: &mut LuaVM) {
    // Register the actual async implementation
    vm.async_executor.register_async_function(
        "sleep".to_string(),
        |args: Vec<LuaValue>| async move {
            let ms = args.get(0)
                .and_then(|v| v.as_number())
                .unwrap_or(0.0) as u64;

            sleep(Duration::from_millis(ms)).await;
            Ok(vec![])
        }
    );

    // TODO: Add more async functions as needed
}

/// Wrapper function that can be called from Lua
/// Automatically handles async task spawning and coroutine yielding
fn async_sleep_wrapper(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // 检查是否在协程中
    let coroutine = vm.current_thread_value.clone().ok_or_else(|| {
        vm.error("async.sleep can only be called from within a coroutine".to_string())
    })?;

    // 收集参数
    let frame = &vm.frames[vm.frame_count - 1];
    let base = frame.base_ptr;
    let top = frame.top;
    let mut args = Vec::new();
    for i in 1..top {
        args.push(vm.register_stack[base + i]);
    }

    // 启动异步任务
    let task_id = vm.async_executor.spawn_task("sleep", args, coroutine)
        .map_err(|e| vm.error(e))?;

    // Yield协程
    vm.yield_values = vec![LuaValue::integer(task_id as i64)];
    Err(LuaError::Yield)
}
