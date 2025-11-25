// OS library (stub implementation)
// Implements: clock, date, difftime, execute, exit, getenv, remove, rename,
// setlocale, time, tmpname

use crate::lib_registry::LibraryModule;
use crate::lib_registry::get_arg;
use crate::lua_value::{LuaValue, MultiValue};
use crate::lua_vm::LuaResult;
use crate::lua_vm::LuaVM;

pub fn create_os_lib() -> LibraryModule {
    crate::lib_module!("os", {
        "clock" => os_clock,
        "time" => os_time,
        "date" => os_date,
        "difftime" => os_difftime,
        "execute" => os_execute,
        "exit" => os_exit,
        "getenv" => os_getenv,
        "remove" => os_remove,
        "rename" => os_rename,
        "setlocale" => os_setlocale,
        "tmpname" => os_tmpname,
    })
}

fn os_clock(_vm: &mut LuaVM) -> LuaResult<MultiValue> {
    use std::time::Instant;

    // Use a thread-local static to track start time
    thread_local! {
        static START_TIME: std::cell::RefCell<Option<Instant>> = std::cell::RefCell::new(None);
    }

    let elapsed = START_TIME.with(|start| {
        let mut start_ref = start.borrow_mut();
        if start_ref.is_none() {
            *start_ref = Some(Instant::now());
        }
        start_ref.unwrap().elapsed().as_secs_f64()
    });

    Ok(MultiValue::single(LuaValue::float(elapsed)))
}

fn os_time(_vm: &mut LuaVM) -> LuaResult<MultiValue> {
    use std::time::SystemTime;

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    Ok(MultiValue::single(LuaValue::integer(timestamp as i64)))
}

fn os_date(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Stub: return current timestamp as string
    use std::time::SystemTime;

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let date_str = format!("timestamp: {}", timestamp);
    let result = vm.create_string(&date_str);
    Ok(MultiValue::single(result))
}

fn os_exit(_vm: &mut LuaVM) -> LuaResult<MultiValue> {
    std::process::exit(0);
}

fn os_difftime(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let t2 = get_arg(vm, 0)
        .and_then(|v| v.as_integer())
        .ok_or(vm.error(
            "difftime: argument 1 must be a number".to_string(),
        ))?;
    let t1 = get_arg(vm, 1)
        .and_then(|v| v.as_integer())
        .ok_or(vm.error(
            "difftime: argument 2 must be a number".to_string(),
        ))?;

    let diff = t2 - t1;
    Ok(MultiValue::single(LuaValue::integer(diff)))
}

fn os_execute(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    use std::process::Command;

    let cmd = get_arg(vm, 0)
        .and_then(|v| v.as_lua_string().map(|s| s.as_str().to_string()))
        .ok_or(vm.error(
            "execute: argument 1 must be a string".to_string(),
        ))?;

    let output = Command::new("sh").arg("-c").arg(cmd.as_str()).output();

    match output {
        Ok(result) => {
            let exit_code = result.status.code().unwrap_or(-1);
            Ok(MultiValue::multiple(vec![
                LuaValue::boolean(result.status.success()),
                vm.create_string("exit"),
                LuaValue::integer(exit_code as i64),
            ]))
        }
        Err(_) => Ok(MultiValue::single(LuaValue::nil())),
    }
}

fn os_getenv(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let varname = get_arg(vm, 0)
        .and_then(|v| v.as_lua_string().map(|s| s.as_str().to_string()))
        .ok_or(vm.error(
            "getenv: argument 1 must be a string".to_string(),
        ))?;

    match std::env::var(varname.as_str()) {
        Ok(value) => {
            let result = vm.create_string(&value);
            Ok(MultiValue::single(result))
        }
        Err(_) => Ok(MultiValue::single(LuaValue::nil())),
    }
}

fn os_remove(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let filename = get_arg(vm, 0)
        .and_then(|v| v.as_lua_string().map(|s| s.as_str().to_string()))
        .ok_or(vm.error(
            "remove: argument 1 must be a string".to_string(),
        ))?;

    match std::fs::remove_file(filename.as_str()) {
        Ok(_) => Ok(MultiValue::single(LuaValue::boolean(true))),
        Err(e) => {
            let err_msg = vm.create_string(&format!("{}", e));
            Ok(MultiValue::multiple(vec![LuaValue::nil(), err_msg]))
        }
    }
}

fn os_rename(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    let oldname = get_arg(vm, 0)
        .and_then(|v| v.as_lua_string().map(|s| s.as_str().to_string()))
        .ok_or(vm.error(
            "rename: argument 1 must be a string".to_string(),
        ))?;
    let newname = get_arg(vm, 1)
        .and_then(|v| v.as_lua_string().map(|s| s.as_str().to_string()))
        .ok_or(vm.error(
            "rename: argument 2 must be a string".to_string(),
        ))?;

    match std::fs::rename(oldname.as_str(), newname.as_str()) {
        Ok(_) => Ok(MultiValue::single(LuaValue::boolean(true))),
        Err(e) => {
            let err_msg = vm.create_string(&format!("{}", e));
            Ok(MultiValue::multiple(vec![LuaValue::nil(), err_msg]))
        }
    }
}

fn os_setlocale(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    // Stub implementation - just return the requested locale or "C"
    let locale = get_arg(vm, 0)
        .and_then(|v| v.as_lua_string().map(|s| s.as_str().to_string()))
        .map(|s| s.as_str().to_string())
        .unwrap_or_else(|| "C".to_string());

    let result = vm.create_string(&locale);
    Ok(MultiValue::single(result))
}

fn os_tmpname(vm: &mut LuaVM) -> LuaResult<MultiValue> {
    use std::time::SystemTime;

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let tmpname = format!("/tmp/lua_tmp_{}", timestamp);
    let result = vm.create_string(&tmpname);
    Ok(MultiValue::single(result))
}
