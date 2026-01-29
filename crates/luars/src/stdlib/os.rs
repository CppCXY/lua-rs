// OS library (stub implementation)
// Implements: clock, date, difftime, execute, exit, getenv, remove, rename,
// setlocale, time, tmpname

use crate::lib_registry::LibraryModule;
use crate::lua_value::LuaValue;
use crate::lua_vm::{LuaResult, LuaState};
use chrono::{DateTime, Datelike, Local, TimeZone, Timelike, Utc};

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
    // Use VM's start_time for consistent measurements
    let elapsed = l.vm_mut().start_time.elapsed().as_secs_f64();
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

// os.date([format [, time]])
// format: optional string specifying format (default "*t")
// time: optional timestamp (default current time)
fn os_date(l: &mut LuaState) -> LuaResult<usize> {
    let format_arg = l.get_arg(1);
    let time_arg = l.get_arg(2);

    // Get timestamp (default to current time)
    let timestamp = if let Some(t) = time_arg {
        if let Some(n) = t.as_number() {
            n as i64
        } else {
            return Err(l.error("bad argument #2 to 'date' (number expected)".to_string()));
        }
    } else {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    };

    // Parse format string (default is "%c" which gives a readable date/time)
    let format_str = if let Some(f) = format_arg {
        if let Some(s) = f.as_str() {
            s.to_string()
        } else {
            return Err(l.error("bad argument #1 to 'date' (string expected)".to_string()));
        }
    } else {
        "%c".to_string() // Default to standard date/time format
    };

    // Check if UTC (starts with '!') or local time
    let (use_utc, actual_format) = if format_str.starts_with('!') {
        (true, &format_str[1..])
    } else {
        (false, format_str.as_str())
    };

    // Get DateTime object
    let dt: DateTime<Local> = if use_utc {
        Utc.timestamp_opt(timestamp, 0)
            .single()
            .ok_or_else(|| l.error("invalid timestamp".to_string()))?
            .with_timezone(&Local)
    } else {
        Local
            .timestamp_opt(timestamp, 0)
            .single()
            .ok_or_else(|| l.error("invalid timestamp".to_string()))?
    };

    // Handle special formats
    match actual_format {
        "*t" => {
            // Return table with date components
            let table = l.create_table(0, 9)?;

            let year_key = l.create_string("year")?;
            l.raw_set(&table, year_key, LuaValue::integer(dt.year() as i64));

            let month_key = l.create_string("month")?;
            l.raw_set(&table, month_key, LuaValue::integer(dt.month() as i64));

            let day_key = l.create_string("day")?;
            l.raw_set(&table, day_key, LuaValue::integer(dt.day() as i64));

            let hour_key = l.create_string("hour")?;
            l.raw_set(&table, hour_key, LuaValue::integer(dt.hour() as i64));

            let min_key = l.create_string("min")?;
            l.raw_set(&table, min_key, LuaValue::integer(dt.minute() as i64));

            let sec_key = l.create_string("sec")?;
            l.raw_set(&table, sec_key, LuaValue::integer(dt.second() as i64));

            // wday: weekday (Sunday is 1)
            let wday_key = l.create_string("wday")?;
            let wday = dt.weekday().number_from_sunday();
            l.raw_set(&table, wday_key, LuaValue::integer(wday as i64));

            // yday: day of year (1-366)
            let yday_key = l.create_string("yday")?;
            let yday = dt.ordinal();
            l.raw_set(&table, yday_key, LuaValue::integer(yday as i64));

            // isdst: daylight saving time flag (TODO: implement properly)
            let isdst_key = l.create_string("isdst")?;
            l.raw_set(&table, isdst_key, LuaValue::boolean(false));

            l.push_value(table)?;
            Ok(1)
        }
        "" => {
            // Empty format: return default date string
            let date_str = dt.format("%a %b %d %H:%M:%S %Y").to_string();
            let result = l.create_string(&date_str)?;
            l.push_value(result)?;
            Ok(1)
        }
        _ => {
            // Use strftime-style format string
            // Map Lua format codes to chrono format codes
            let date_str = format_date_string(dt, actual_format);
            let result = l.create_string(&date_str)?;
            l.push_value(result)?;
            Ok(1)
        }
    }
}

// Helper function to format date with Lua-style format codes
fn format_date_string(dt: DateTime<Local>, format: &str) -> String {
    // Simple implementation: convert common Lua date codes to chrono codes
    // Lua uses %a, %A, %b, %B, %c, %d, %H, %I, %j, %m, %M, %p, %S, %U, %w, %W, %x, %X, %y, %Y, %z, %Z, %%
    let mut result = String::new();
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '%' {
            if let Some(&next_ch) = chars.peek() {
                chars.next(); // consume the format character
                let formatted = match next_ch {
                    'a' => dt.format("%a").to_string(), // abbreviated weekday
                    'A' => dt.format("%A").to_string(), // full weekday
                    'b' => dt.format("%b").to_string(), // abbreviated month
                    'B' => dt.format("%B").to_string(), // full month
                    'c' => dt.format("%a %b %d %H:%M:%S %Y").to_string(), // standard date/time
                    'd' => dt.format("%d").to_string(), // day of month (01-31)
                    'H' => dt.format("%H").to_string(), // hour (00-23)
                    'I' => dt.format("%I").to_string(), // hour (01-12)
                    'j' => dt.format("%j").to_string(), // day of year (001-366)
                    'm' => dt.format("%m").to_string(), // month (01-12)
                    'M' => dt.format("%M").to_string(), // minute (00-59)
                    'p' => dt.format("%p").to_string(), // AM/PM
                    'S' => dt.format("%S").to_string(), // second (00-59)
                    'w' => dt.weekday().num_days_from_sunday().to_string(), // weekday (0-6, Sunday is 0)
                    'x' => dt.format("%m/%d/%y").to_string(),               // date representation
                    'X' => dt.format("%H:%M:%S").to_string(),               // time representation
                    'y' => dt.format("%y").to_string(),                     // year (00-99)
                    'Y' => dt.format("%Y").to_string(),                     // year (full)
                    'z' => dt.format("%z").to_string(),                     // timezone offset
                    'Z' => dt.format("%Z").to_string(),                     // timezone name
                    '%' => "%".to_string(),                                 // literal %
                    _ => format!("%{}", next_ch),                           // unknown, keep as-is
                };
                result.push_str(&formatted);
            } else {
                result.push('%');
            }
        } else {
            result.push(ch);
        }
    }

    result
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
            v.as_str().map(|s| s.to_string())
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
            let exit_str = l.create_string("exit")?;
            l.push_value(LuaValue::boolean(result.status.success()))?;
            l.push_value(exit_str)?;
            l.push_value(LuaValue::integer(exit_code as i64))?;
            Ok(3)
        }
        Err(_) => {
            let exit_str = l.create_string("exit")?;
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
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| l.error("getenv: argument 1 must be a string".to_string()))?;

    match std::env::var(&varname) {
        Ok(value) => {
            let result = l.create_string(&value)?;
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
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| l.error("remove: argument 1 must be a string".to_string()))?;

    match std::fs::remove_file(&filename) {
        Ok(_) => {
            l.push_value(LuaValue::boolean(true))?;
            Ok(1)
        }
        Err(e) => {
            let err_msg = l.create_string(&format!("{}", e))?;
            l.push_value(LuaValue::nil())?;
            l.push_value(err_msg)?;
            Ok(2)
        }
    }
}

fn os_rename(l: &mut LuaState) -> LuaResult<usize> {
    let oldname = l
        .get_arg(1)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| l.error("rename: argument 1 must be a string".to_string()))?;
    let newname = l
        .get_arg(2)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| l.error("rename: argument 2 must be a string".to_string()))?;

    match std::fs::rename(&oldname, &newname) {
        Ok(_) => {
            l.push_value(LuaValue::boolean(true))?;
            Ok(1)
        }
        Err(e) => {
            let err_msg = l.create_string(&format!("{}", e))?;
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
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "C".to_string());

    let result = l.create_string(&locale)?;
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
    let result = l.create_string(&tmpname)?;
    l.push_value(result)?;
    Ok(1)
}
