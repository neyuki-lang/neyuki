//! The `os` library.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::format::{Item, StrftimeItems};
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeDelta, TimeZone, Timelike, Utc};

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

fn os_clock(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(vec![Value::Float(now.as_secs_f64())])
}

/// `time()` is the current time in whole seconds since the Unix epoch;
/// `time(t)` converts a local date table (`year`, `month`, `day`, and
/// optionally `hour` (default 12), `min` and `sec`) the same way, with
/// out-of-range fields carried over as in Lua (month 13 is next January).
fn os_time(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let Some(Value::Table(fields)) = args.first() else {
        if let Some(other) = args.first().filter(|v| !matches!(v, Value::Nil)) {
            return Err(format!("t must be a table, got {}", other.type_name()));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        return Ok(vec![Value::from_bigint(BigInt::from(now.as_secs()))]);
    };
    let fields = fields.borrow();
    let field = |name: &str, default: Option<i64>| -> Result<i64, String> {
        match fields.get_str(name) {
            Value::Nil => default.ok_or_else(|| format!("field '{}' is missing", name)),
            Value::Int(i) => Ok(i),
            Value::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => Ok(f as i64),
            other => Err(format!(
                "field '{}' must be an integer, got {}",
                name,
                other.type_name()
            )),
        }
    };
    let (year, month, day) = (
        field("year", None)?,
        field("month", None)?,
        field("day", None)?,
    );
    let (hour, min, sec) = (
        field("hour", Some(12))?,
        field("min", Some(0))?,
        field("sec", Some(0))?,
    );
    let months = year
        .checked_mul(12)
        .and_then(|m| m.checked_add(month - 1))
        .ok_or("date is out of range")?;
    let first = i32::try_from(months.div_euclid(12))
        .ok()
        .and_then(|y| NaiveDate::from_ymd_opt(y, months.rem_euclid(12) as u32 + 1, 1))
        .ok_or("date is out of range")?;
    let offset = TimeDelta::try_days(day - 1)
        .zip(TimeDelta::try_hours(hour))
        .zip(TimeDelta::try_minutes(min))
        .zip(TimeDelta::try_seconds(sec))
        .and_then(|(((d, h), m), s)| d.checked_add(&h)?.checked_add(&m)?.checked_add(&s));
    let naive = offset
        .and_then(|offset| first.and_time(NaiveTime::MIN).checked_add_signed(offset))
        .ok_or("date is out of range")?;
    let local = Local
        .from_local_datetime(&naive)
        .earliest()
        .ok_or("that local time does not exist")?;
    Ok(vec![Value::Int(local.timestamp())])
}

/// A time as the table `date("*t")` returns.
fn date_table<Tz: TimeZone>(time: &DateTime<Tz>) -> Value {
    let mut table = VmTable::new();
    table.set_str("year", Value::Int(time.year() as i64));
    table.set_str("month", Value::Int(time.month() as i64));
    table.set_str("day", Value::Int(time.day() as i64));
    table.set_str("hour", Value::Int(time.hour() as i64));
    table.set_str("min", Value::Int(time.minute() as i64));
    table.set_str("sec", Value::Int(time.second() as i64));
    // Lua counts weekdays from Sunday = 1.
    table.set_str(
        "wday",
        Value::Int(time.weekday().num_days_from_sunday() as i64 + 1),
    );
    table.set_str("yday", Value::Int(time.ordinal() as i64));
    Value::Table(Rc::new(RefCell::new(table)))
}

/// Formats a time with strftime codes, rejecting unknown ones instead of
/// letting the formatter fail.
fn format_time<Tz: TimeZone>(time: &DateTime<Tz>, format: &str) -> Result<String, String>
where
    Tz::Offset: std::fmt::Display,
{
    let items: Vec<Item> = StrftimeItems::new(format).collect();
    if items.iter().any(|item| matches!(item, Item::Error)) {
        return Err(format!("invalid date format `{}`", format));
    }
    let mut out = String::new();
    use std::fmt::Write;
    write!(out, "{}", time.format_with_items(items.into_iter()))
        .map_err(|_| format!("invalid date format `{}`", format))?;
    Ok(out)
}

/// `date(format?, time?)`: formats `time` (default now, in seconds since the
/// epoch) in local time with strftime codes (default "%c"). A leading "!"
/// uses UTC instead, and "*t" returns a table of the fields.
fn os_date(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let format = match args.first() {
        None | Some(Value::Nil) => "%c",
        Some(Value::String(s)) => &**s,
        Some(other) => {
            return Err(format!(
                "format must be a string, got {}",
                other.type_name()
            ));
        }
    };
    let seconds = match args.get(1) {
        None | Some(Value::Nil) => None,
        Some(value) => Some(
            crate::vm::libs::args::number(value)
                .map_err(|_| format!("time must be a number, got {}", value.type_name()))?,
        ),
    };
    let utc_time = match seconds {
        None => Utc::now(),
        Some(seconds) => {
            let whole = seconds.floor();
            DateTime::from_timestamp(whole as i64, ((seconds - whole) * 1e9) as u32)
                .filter(|_| seconds.is_finite())
                .ok_or("time is out of range")?
        }
    };
    let (utc, format) = match format.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, format),
    };
    let value = match (utc, format) {
        (true, "*t") => date_table(&utc_time),
        (false, "*t") => date_table(&utc_time.with_timezone(&Local)),
        (true, _) => Value::string(format_time(&utc_time, format)?),
        (false, _) => Value::string(format_time(&utc_time.with_timezone(&Local), format)?),
    };
    Ok(vec![value])
}

fn os_difftime(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let t2 = match args
        .first()
        .ok_or_else(|| "os.difftime expects 2 arguments".to_string())?
    {
        Value::Int(i) => i.to_f64().unwrap_or(0.0),
        Value::Float(f) => *f,
        _ => return Err("os.difftime expects numbers".to_string()),
    };
    let t1 = match args
        .get(1)
        .ok_or_else(|| "os.difftime expects 2 arguments".to_string())?
    {
        Value::Int(i) => i.to_f64().unwrap_or(0.0),
        Value::Float(f) => *f,
        _ => return Err("os.difftime expects numbers".to_string()),
    };
    Ok(vec![Value::Float(t2 - t1)])
}

static SCRIPT_ARGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Records the command-line arguments that follow the script path, for
/// `os.args`. Only the first call has an effect.
pub fn set_script_args(args: Vec<String>) {
    let _ = SCRIPT_ARGS.set(args);
}

/// The script's command-line arguments as a new list of strings.
fn os_args(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    let args = SCRIPT_ARGS.get().map(Vec::as_slice).unwrap_or_default();
    Ok(vec![crate::vm::libs::args::array(
        args.iter().map(|a| Value::string(a.clone())).collect(),
    )])
}

/// Blocks the script for `seconds` (fractions allowed).
fn os_sleep(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let seconds = crate::vm::libs::args::number_arg(args, 0, "seconds")?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err("seconds must be a non-negative number".to_string());
    }
    std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
    Ok(vec![])
}

/// Ends the process. `true` or nothing exits with 0, `false` with 1 and an
/// integer with that code. Standard output is flushed first.
fn os_exit(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let code = match args.first() {
        None | Some(Value::Nil) | Some(Value::Bool(true)) => 0,
        Some(Value::Bool(false)) => 1,
        Some(Value::Int(i)) => i32::try_from(*i).map_err(|_| "exit code is out of range")?,
        Some(other) => {
            return Err(format!(
                "code must be a boolean or an integer, got {}",
                other.type_name()
            ));
        }
    };
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(code)
}

/// The value of an environment variable, or nil when it is unset or not
/// valid Unicode.
fn os_getenv(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let varname = match args
        .first()
        .ok_or_else(|| "os.getenv expects variable name".to_string())?
    {
        Value::String(s) => s,
        _ => return Err("os.getenv expects string".to_string()),
    };
    match std::env::var(&**varname) {
        Ok(v) => Ok(vec![Value::string(v)]),
        Err(_) => Ok(vec![Value::Nil]),
    }
}

pub fn create_os_lib() -> Value {
    let mut table = VmTable::new();
    table.set_str("clock", crate::native!("os.clock", os_clock));
    table.set_str("time", crate::native!("os.time", os_time));
    table.set_str("date", crate::native!("os.date", os_date));
    table.set_str("difftime", crate::native!("os.difftime", os_difftime));
    table.set_str("getenv", crate::native!("os.getenv", os_getenv));
    table.set_str("sleep", crate::native!("os.sleep", os_sleep));
    table.set_str("exit", crate::native!("os.exit", os_exit));
    table.set_str("args", crate::native!("os.args", os_args));
    table.frozen = true;
    Value::Table(Rc::new(RefCell::new(table)))
}
