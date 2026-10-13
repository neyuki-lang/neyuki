//! The `os` library.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

fn os_clock(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(vec![Value::Float(now.as_secs_f64())])
}

fn os_time(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Ok(vec![Value::from_bigint(BigInt::from(now.as_secs()))])
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

// Capability-based security sandbox for os.getenv:
// Only explicitly permitted non-sensitive environment variables can be accessed.
pub fn is_env_var_allowed(name: &str) -> bool {
    const ALLOWED_ENV_VARS: &[&str] = &[
        "PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TMPDIR",
        "TMP", "TEMP", "PWD",
    ];
    ALLOWED_ENV_VARS.contains(&name)
}

fn os_getenv(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let varname = match args
        .first()
        .ok_or_else(|| "os.getenv expects variable name".to_string())?
    {
        Value::String(s) => s,
        _ => return Err("os.getenv expects string".to_string()),
    };
    if !is_env_var_allowed(varname) {
        return Ok(vec![Value::Nil]);
    }
    match std::env::var(&**varname) {
        Ok(v) => Ok(vec![Value::string(v)]),
        Err(_) => Ok(vec![Value::Nil]),
    }
}

pub fn create_os_lib() -> Value {
    let mut table = VmTable::new();
    table.set_str("clock", crate::native!("os.clock", os_clock));
    table.set_str("time", crate::native!("os.time", os_time));
    table.set_str("difftime", crate::native!("os.difftime", os_difftime));
    table.set_str("getenv", crate::native!("os.getenv", os_getenv));
    table.set_str("sleep", crate::native!("os.sleep", os_sleep));
    table.set_str("exit", crate::native!("os.exit", os_exit));
    table.frozen = true;
    Value::Table(Rc::new(RefCell::new(table)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_os_getenv_security_sandbox() {
        assert!(is_env_var_allowed("PATH"));
        assert!(is_env_var_allowed("HOME"));
        assert!(is_env_var_allowed("USER"));
        assert!(is_env_var_allowed("LANG"));

        // Sensitive credentials and arbitrary environment variables must be denied
        assert!(!is_env_var_allowed("AWS_SECRET_ACCESS_KEY"));
        assert!(!is_env_var_allowed("GITHUB_TOKEN"));
        assert!(!is_env_var_allowed("DATABASE_URL"));
        assert!(!is_env_var_allowed("MY_APP_PASSWORD"));
        assert!(!is_env_var_allowed("SSH_PRIVATE_KEY"));
        assert!(!is_env_var_allowed("AUTH_BEARER_TOKEN"));
        assert!(!is_env_var_allowed("FOO_BAR"));
    }
}
