// OS library (stub implementation)
// Implements: clock, date, difftime, execute, exit, getenv, remove, rename,
// setlocale, time, tmpname

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaResult, LuaState};

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

fn os_clock(l: &mut LuaState) -> LuaResult<usize> {
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

    l.push_value(LuaValue::float(elapsed))?;
    Ok(1)
}

fn os_time(l: &mut LuaState) -> LuaResult<usize> {
    use std::time::SystemTime;

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    l.push_value(LuaValue::integer(timestamp as i64))?;
    Ok(1)
}

fn os_date(l: &mut LuaState) -> LuaResult<usize> {
    // Stub: return current timestamp as string
    use std::time::SystemTime;

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let date_str = format!("timestamp: {}", timestamp);
    let result = l.vm_mut().create_string(&date_str);
    l.push_value(result)?;
    Ok(1)
}

fn os_exit(_l: &mut LuaState) -> LuaResult<usize> {
    std::process::exit(0);
}

fn os_difftime(l: &mut LuaState) -> LuaResult<usize> {
    let t2 = l
        .get_arg(1)
        .and_then(|v| v.as_integer())
        .ok_or_else(|| l.error("difftime: argument 1 must be a number".to_string()))?;
    let t1 = l
        .get_arg(2)
        .and_then(|v| v.as_integer())
        .ok_or_else(|| l.error("difftime: argument 2 must be a number".to_string()))?;

    let diff = t2 - t1;
    l.push_value(LuaValue::integer(diff))?;
    Ok(1)
}

fn os_execute(l: &mut LuaState) -> LuaResult<usize> {
    use std::process::Command;

    let cmd_opt = l.get_arg(1).and_then(|v| {
        if v.is_nil() {
            None
        } else {
            v.as_string_id()
                .and_then(|id| l.vm_mut().object_pool.get_string(id).map(|s| s.to_string()))
        }
    });

    // If no command given, check if shell is available
    let Some(cmd) = cmd_opt else {
        // Return true to indicate shell is available
        l.push_value(LuaValue::boolean(true))?;
        return Ok(1);
    };

    // Platform-specific command execution
    #[cfg(target_os = "windows")]
    let output = Command::new("cmd").args(["/C", &cmd]).output();

    #[cfg(not(target_os = "windows"))]
    let output = Command::new("sh").arg("-c").arg(&cmd).output();

    match output {
        Ok(result) => {
            let exit_code = result.status.code().unwrap_or(-1);
            let exit_str = l.vm_mut().create_string("exit");
            l.push_value(LuaValue::boolean(result.status.success()))?;
            l.push_value(exit_str)?;
            l.push_value(LuaValue::integer(exit_code as i64))?;
            Ok(3)
        }
        Err(_) => {
            let exit_str = l.vm_mut().create_string("exit");
            l.push_value(LuaValue::nil())?;
            l.push_value(exit_str)?;
            l.push_value(LuaValue::integer(-1))?;
            Ok(3)
        }
    }
}

fn os_getenv(l: &mut LuaState) -> LuaResult<usize> {
    let varname = l
        .get_arg(1)
        .and_then(|v| v.as_string_id())
        .and_then(|id| l.vm_mut().object_pool.get_string(id).map(|s| s.to_string()))
        .ok_or_else(|| l.error("getenv: argument 1 must be a string".to_string()))?;

    match std::env::var(&varname) {
        Ok(value) => {
            let result = l.vm_mut().create_string(&value);
            l.push_value(result)?;
            Ok(1)
        }
        Err(_) => {
            l.push_value(LuaValue::nil())?;
            Ok(1)
        }
    }
}

fn os_remove(l: &mut LuaState) -> LuaResult<usize> {
    let filename = l
        .get_arg(1)
        .and_then(|v| v.as_string_id())
        .and_then(|id| l.vm_mut().object_pool.get_string(id).map(|s| s.to_string()))
        .ok_or_else(|| l.error("remove: argument 1 must be a string".to_string()))?;

    match std::fs::remove_file(&filename) {
        Ok(_) => {
            l.push_value(LuaValue::boolean(true))?;
            Ok(1)
        }
        Err(e) => {
            let err_msg = l.vm_mut().create_string(&format!("{}", e));
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            Ok(2)
        }
    }
}

fn os_rename(l: &mut LuaState) -> LuaResult<usize> {
    let oldname = l
        .get_arg(1)
        .and_then(|v| v.as_string_id())
        .and_then(|id| l.vm_mut().object_pool.get_string(id).map(|s| s.to_string()))
        .ok_or_else(|| l.error("rename: argument 1 must be a string".to_string()))?;
    let newname = l
        .get_arg(2)
        .and_then(|v| v.as_string_id())
        .and_then(|id| l.vm_mut().object_pool.get_string(id).map(|s| s.to_string()))
        .ok_or_else(|| l.error("rename: argument 2 must be a string".to_string()))?;

    match std::fs::rename(&oldname, &newname) {
        Ok(_) => {
            l.push_value(LuaValue::boolean(true))?;
            Ok(1)
        }
        Err(e) => {
            let err_msg = l.vm_mut().create_string(&format!("{}", e));
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            Ok(2)
        }
    }
}

fn os_setlocale(l: &mut LuaState) -> LuaResult<usize> {
    // Stub implementation - just return the requested locale or "C"
    let locale = l
        .get_arg(1)
        .and_then(|v| v.as_string_id())
        .and_then(|id| l.vm_mut().object_pool.get_string(id).map(|s| s.to_string()))
        .unwrap_or_else(|| "C".to_string());

    let result = l.vm_mut().create_string(&locale);
    l.push_value(result)?;
    Ok(1)
}

fn os_tmpname(l: &mut LuaState) -> LuaResult<usize> {
    use std::time::SystemTime;

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let tmpname = format!("/tmp/lua_tmp_{}", timestamp);
    let result = l.vm_mut().create_string(&tmpname);
    l.push_value(result)?;
    Ok(1)
}
