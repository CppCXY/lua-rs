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

    let arg = l.get_arg(1);

    if let Some(table_val) = arg {
        if table_val.is_nil() {
            // os.time() with nil = current time
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            l.push_value(LuaValue::integer(timestamp as i64))?;
            return Ok(1);
        }
        // os.time(table) - convert table to timestamp
        if let Some(_tbl) = table_val.as_table() {
            let get_field = |l: &mut LuaState, name: &str| -> Result<Option<i64>, String> {
                let key = l.create_string(name).unwrap();
                let val = table_val.as_table().unwrap().raw_get(&key);
                match val {
                    Some(v) => {
                        if let Some(n) = v.as_integer() {
                            Ok(Some(n))
                        } else if let Some(n) = v.as_number() {
                            if n.fract() != 0.0 {
                                Err("not an integer".to_string())
                            } else {
                                Ok(Some(n as i64))
                            }
                        } else {
                            Err("not an integer".to_string())
                        }
                    }
                    None => Ok(None),
                }
            };

            let year = get_field(l, "year")
                .map_err(|e| l.error(format!("field 'year' is {}", e)))?
                .ok_or_else(|| l.error("field 'year' missing in date table".to_string()))?;
            let month = get_field(l, "month")
                .map_err(|e| l.error(format!("field 'month' is {}", e)))?
                .ok_or_else(|| l.error("field 'month' missing in date table".to_string()))?;
            let day = get_field(l, "day")
                .map_err(|e| l.error(format!("field 'day' is {}", e)))?
                .ok_or_else(|| l.error("field 'day' missing in date table".to_string()))?;
            let hour = match get_field(l, "hour") {
                Ok(Some(v)) => v,
                Ok(None) => 12,
                Err(e) => return Err(l.error(format!("field 'hour' is {}", e))),
            };
            let min = match get_field(l, "min") {
                Ok(Some(v)) => v,
                Ok(None) => 0,
                Err(e) => return Err(l.error(format!("field 'min' is {}", e))),
            };
            let sec = match get_field(l, "sec") {
                Ok(Some(v)) => v,
                Ok(None) => 0,
                Err(e) => return Err(l.error(format!("field 'sec' is {}", e))),
            };

            // Validate year range for 32-bit time_t compatibility
            // Lua checks if year fits in an int after subtracting 1900
            let year_offset = year - 1900;
            if year_offset < i32::MIN as i64 || year_offset > i32::MAX as i64 {
                return Err(l.error("field 'year' is out-of-bound".to_string()));
            }

            // Validate other fields fit in int
            if month < i32::MIN as i64 || month > i32::MAX as i64 {
                return Err(l.error("field 'month' is out-of-bound".to_string()));
            }
            if day < i32::MIN as i64 || day > i32::MAX as i64 {
                return Err(l.error("field 'day' is out-of-bound".to_string()));
            }
            if hour < i32::MIN as i64 || hour > i32::MAX as i64 {
                return Err(l.error("field 'hour' is out-of-bound".to_string()));
            }
            if min < i32::MIN as i64 || min > i32::MAX as i64 {
                return Err(l.error("field 'min' is out-of-bound".to_string()));
            }
            if sec < i32::MIN as i64 || sec > i32::MAX as i64 {
                return Err(l.error("field 'sec' is out-of-bound".to_string()));
            }

            // Use chrono to build a NaiveDateTime, then convert to local time
            // chrono handles month/day normalization naturally
            use chrono::NaiveDate;

            // Handle out-of-range months by adjusting year
            let mut adj_year = year;
            let mut adj_month = month;
            if adj_month < 1 || adj_month > 12 {
                // Normalize: month 0 = December of previous year, month 13 = January of next year, etc.
                adj_year += (adj_month - 1).div_euclid(12);
                adj_month = (adj_month - 1).rem_euclid(12) + 1;
            }

            // Try building the date - handle day overflow via duration addition
            // For years that fit in i32, use chrono. For larger years, compute directly.
            if adj_year >= i32::MIN as i64 && adj_year <= i32::MAX as i64 {
                use chrono::NaiveTime;
                let base_date = NaiveDate::from_ymd_opt(adj_year as i32, adj_month as u32, 1);
                let base_time = NaiveTime::from_hms_opt(0, 0, 0).unwrap();
                if let Some(base) = base_date {
                    let base_dt = base.and_time(base_time);
                    let dt = base_dt
                        + chrono::Duration::days(day - 1)
                        + chrono::Duration::hours(hour)
                        + chrono::Duration::minutes(min)
                        + chrono::Duration::seconds(sec);

                    // Convert to local timezone timestamp
                    let local_result = Local.from_local_datetime(&dt);
                    if let Some(local_dt) = local_result.single().or_else(|| local_result.latest())
                    {
                        let timestamp = local_dt.timestamp();

                        // Normalize the input table (like C's mktime)
                        normalize_time_table(l, &table_val, &local_dt)?;

                        l.push_value(LuaValue::integer(timestamp))?;
                        return Ok(1);
                    }
                }
            } else {
                // Year too large for chrono - compute timestamp directly (UTC approximation)
                // This handles extreme years like (1<<31) + 1899
                let ts = mktime_approx(adj_year, adj_month, day, hour, min, sec);
                if let Some(t) = ts {
                    // Check that the normalized result has a year that fits in
                    // C's int (year - 1900 must fit in i32)
                    let result_year = year_from_timestamp(t);
                    let result_year_offset = result_year - 1900;
                    if result_year_offset < i32::MIN as i64 || result_year_offset > i32::MAX as i64
                    {
                        return Err(l.error(
                            "time result cannot be represented in this installation".to_string(),
                        ));
                    }
                    l.push_value(LuaValue::integer(t))?;
                    return Ok(1);
                }
            }

            return Err(
                l.error("time result cannot be represented in this installation".to_string())
            );
        } else {
            return Err(l.error("table expected".to_string()));
        }
    }

    // No argument: return current time
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    l.push_value(LuaValue::integer(timestamp as i64))?;
    Ok(1)
}

/// Approximate mktime for years outside chrono's i32 range
/// Computes seconds since epoch assuming UTC (no timezone offset)
fn mktime_approx(year: i64, month: i64, day: i64, hour: i64, min: i64, sec: i64) -> Option<i64> {
    // Days from year 0 to year Y (approximate, handling leap years)
    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        // Algorithm from Howard Hinnant's date library
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = (y - era * 400) as u64;
        let m = m as u64;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as u64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe as i64 - 719468 // days since 1970-01-01
    }

    let days = days_from_civil(year, month, day);
    let ts = days
        .checked_mul(86400)?
        .checked_add(hour * 3600)?
        .checked_add(min * 60)?
        .checked_add(sec)?;
    Some(ts)
}

/// Extract the year from a Unix timestamp (UTC)
fn year_from_timestamp(ts: i64) -> i64 {
    // Inverse of days_from_civil: convert days since epoch to year
    let days = ts.div_euclid(86400);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 { y + 1 } else { y }
}

/// Normalize the time table fields after computing the timestamp (like C's mktime)
fn normalize_time_table(
    l: &mut LuaState,
    table_val: &LuaValue,
    dt: &DateTime<Local>,
) -> LuaResult<()> {
    let year_key = LuaValue::from(l.create_string("year")?);
    l.raw_set(table_val, year_key, LuaValue::integer(dt.year() as i64));
    let month_key = LuaValue::from(l.create_string("month")?);
    l.raw_set(table_val, month_key, LuaValue::integer(dt.month() as i64));
    let day_key = LuaValue::from(l.create_string("day")?);
    l.raw_set(table_val, day_key, LuaValue::integer(dt.day() as i64));
    let hour_key = LuaValue::from(l.create_string("hour")?);
    l.raw_set(table_val, hour_key, LuaValue::integer(dt.hour() as i64));
    let min_key = LuaValue::from(l.create_string("min")?);
    l.raw_set(table_val, min_key, LuaValue::integer(dt.minute() as i64));
    let sec_key = LuaValue::from(l.create_string("sec")?);
    l.raw_set(table_val, sec_key, LuaValue::integer(dt.second() as i64));
    let wday_key = LuaValue::from(l.create_string("wday")?);
    l.raw_set(
        table_val,
        wday_key,
        LuaValue::integer(dt.weekday().number_from_sunday() as i64),
    );
    let yday_key = LuaValue::from(l.create_string("yday")?);
    l.raw_set(table_val, yday_key, LuaValue::integer(dt.ordinal() as i64));

    Ok(())
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
            .ok_or_else(|| {
                l.error("time result cannot be represented in this installation".to_string())
            })?
            .with_timezone(&Local)
    } else {
        Local.timestamp_opt(timestamp, 0).single().ok_or_else(|| {
            l.error("time result cannot be represented in this installation".to_string())
        })?
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
        _ => {
            // Use strftime-style format string
            let date_str = format_date_string(dt, actual_format).map_err(|e| l.error(e))?;
            let result = l.create_string(&date_str)?;
            l.push_value(result)?;
            Ok(1)
        }
    }
}

// Helper function to format date with Lua-style format codes
fn format_date_string(dt: DateTime<Local>, format: &str) -> Result<String, String> {
    // Valid Lua date format specifiers (from C strftime)
    // Lua uses %a, %A, %b, %B, %c, %d, %H, %I, %j, %m, %M, %p, %S, %U, %w, %W, %x, %X, %y, %Y, %z, %Z, %%
    let mut result = String::new();
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '%' {
            if let Some(&next_ch) = chars.peek() {
                chars.next(); // consume the format character
                let formatted = match next_ch {
                    'a' => dt.format("%a").to_string(),
                    'A' => dt.format("%A").to_string(),
                    'b' | 'h' => dt.format("%b").to_string(),
                    'B' => dt.format("%B").to_string(),
                    'c' => dt.format("%a %b %d %H:%M:%S %Y").to_string(),
                    'd' => dt.format("%d").to_string(),
                    'D' => dt.format("%m/%d/%y").to_string(),
                    'e' => dt.format("%e").to_string(),
                    'F' => dt.format("%Y-%m-%d").to_string(),
                    'g' => dt.format("%g").to_string(),
                    'G' => dt.format("%G").to_string(),
                    'H' => dt.format("%H").to_string(),
                    'I' => dt.format("%I").to_string(),
                    'j' => dt.format("%j").to_string(),
                    'm' => dt.format("%m").to_string(),
                    'M' => dt.format("%M").to_string(),
                    'n' => "\n".to_string(),
                    'p' => dt.format("%p").to_string(),
                    'r' => dt.format("%I:%M:%S %p").to_string(),
                    'R' => dt.format("%H:%M").to_string(),
                    'S' => dt.format("%S").to_string(),
                    't' => "\t".to_string(),
                    'T' => dt.format("%H:%M:%S").to_string(),
                    'u' => {
                        let d = dt.weekday().number_from_monday();
                        d.to_string()
                    }
                    'U' => dt.format("%U").to_string(),
                    'V' => dt.format("%V").to_string(),
                    'w' => dt.weekday().num_days_from_sunday().to_string(),
                    'W' => dt.format("%W").to_string(),
                    'x' => dt.format("%m/%d/%y").to_string(),
                    'X' => dt.format("%H:%M:%S").to_string(),
                    'y' => dt.format("%y").to_string(),
                    'Y' => dt.format("%Y").to_string(),
                    'z' => dt.format("%z").to_string(),
                    'Z' => dt.format("%Z").to_string(),
                    '%' => "%".to_string(),
                    // POSIX extensions: %E and %O are modifier prefixes
                    // %Ec, %EC, %Ex, %EX, %Ey, %EY = alternative era-based representations
                    // %Od, %Oe, %OH, %OI, %Om, %OM, %OS, %Ou, %OU, %OV, %Ow, %OW, %Oy = alternative numeric
                    'E' => {
                        if let Some(&mod_ch) = chars.peek() {
                            match mod_ch {
                                'c' | 'C' | 'x' | 'X' | 'y' | 'Y' => {
                                    chars.next();
                                    let formatted = match mod_ch {
                                        'c' => dt.format("%a %b %d %H:%M:%S %Y").to_string(),
                                        'C' => dt.format("%C").to_string(),
                                        'x' => dt.format("%m/%d/%y").to_string(),
                                        'X' => dt.format("%H:%M:%S").to_string(),
                                        'y' => dt.format("%y").to_string(),
                                        'Y' => dt.format("%Y").to_string(),
                                        _ => unreachable!(),
                                    };
                                    result.push_str(&formatted);
                                    continue;
                                }
                                _ => {
                                    return Err(format!(
                                        "invalid conversion specifier '%E{}'",
                                        mod_ch
                                    ));
                                }
                            }
                        } else {
                            return Err("invalid conversion specifier '%E'".to_string());
                        }
                    }
                    'O' => {
                        if let Some(&mod_ch) = chars.peek() {
                            match mod_ch {
                                'd' | 'e' | 'H' | 'I' | 'm' | 'M' | 'S' | 'u' | 'U' | 'V' | 'w'
                                | 'W' | 'y' => {
                                    chars.next();
                                    let formatted = match mod_ch {
                                        'd' => dt.format("%d").to_string(),
                                        'e' => dt.format("%e").to_string(),
                                        'H' => dt.format("%H").to_string(),
                                        'I' => dt.format("%I").to_string(),
                                        'm' => dt.format("%m").to_string(),
                                        'M' => dt.format("%M").to_string(),
                                        'S' => dt.format("%S").to_string(),
                                        'u' => dt.weekday().number_from_monday().to_string(),
                                        'U' => dt.format("%U").to_string(),
                                        'V' => dt.format("%V").to_string(),
                                        'w' => dt.weekday().num_days_from_sunday().to_string(),
                                        'W' => dt.format("%W").to_string(),
                                        'y' => dt.format("%y").to_string(),
                                        _ => unreachable!(),
                                    };
                                    result.push_str(&formatted);
                                    continue;
                                }
                                _ => {
                                    return Err(format!(
                                        "invalid conversion specifier '%O{}'",
                                        mod_ch
                                    ));
                                }
                            }
                        } else {
                            return Err("invalid conversion specifier '%O'".to_string());
                        }
                    }
                    _ => {
                        return Err(format!("invalid conversion specifier '%{}'", next_ch));
                    }
                };
                result.push_str(&formatted);
            } else {
                // Trailing % with no specifier
                return Err("invalid conversion specifier '%'".to_string());
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
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
