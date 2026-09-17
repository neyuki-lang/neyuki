// Global built-in functions: print, assert, type, typeof, tostring, tonumber, int, float, error, pcall, xpcall, require.

use num_bigint::BigInt;
use num_traits::{FromPrimitive, ToPrimitive};
use std::str::FromStr;

use crate::vm::machine::VM;
use crate::vm::value::Value;

pub fn builtin_print(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let out = args
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join("\t");
    println!("{}", out);
    Ok(vec![Value::Nil])
}

pub fn builtin_assert(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let cond = args.first().cloned().unwrap_or(Value::Nil);
    if !cond.is_truthy() {
        let msg = args
            .get(1)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "assertion failed".to_string());
        return Err(msg);
    }
    Ok(vec![cond])
}

pub fn builtin_type(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    Ok(vec![Value::String(val.type_name().to_string())])
}

pub fn builtin_typeof(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    Ok(vec![Value::String(val.typeof_name().to_string())])
}

pub fn builtin_tostring(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    if let Value::Table(t) = val {
        let handler = t
            .borrow()
            .metatable
            .as_ref()
            .and_then(|mt| mt.borrow().fields.get("__tostring").cloned());
        if let Some(h) = handler
            && !matches!(h, Value::Nil)
        {
            let res = vm.call_function(h, std::slice::from_ref(val))?;
            return Ok(vec![res.into_iter().next().unwrap_or(Value::Nil)]);
        }
    }
    Ok(vec![Value::String(val.to_string())])
}

pub fn builtin_tonumber(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    let base_opt = args.get(1);

    if let Some(base_val) = base_opt
        && !matches!(base_val, Value::Nil)
    {
        let base = match base_val {
            Value::Int(i) => i.to_i64().unwrap_or(0),
            Value::Float(f) => *f as i64,
            _ => return Err("bad argument #2 to 'tonumber' (base out of range)".to_string()),
        };
        if !(2..=36).contains(&base) {
            return Err("bad argument #2 to 'tonumber' (base out of range)".to_string());
        }
        let s = match val {
            Value::String(s) => s.as_str(),
            _ => return Ok(vec![Value::Nil]),
        };
        if s.len() > 65_536 {
            return Ok(vec![Value::Nil]);
        }
        let s_trimmed = s.trim();
        let (sign, s_digits) = if let Some(stripped) = s_trimmed.strip_prefix('-') {
            (-1, stripped.trim_start())
        } else if let Some(stripped) = s_trimmed.strip_prefix('+') {
            (1, stripped.trim_start())
        } else {
            (1, s_trimmed)
        };
        let s_digits = if base == 16 {
            if let Some(stripped) = s_digits
                .strip_prefix("0x")
                .or_else(|| s_digits.strip_prefix("0X"))
            {
                stripped
            } else {
                s_digits
            }
        } else {
            s_digits
        };
        if s_digits.is_empty() {
            return Ok(vec![Value::Nil]);
        }
        return match BigInt::parse_bytes(s_digits.as_bytes(), base as u32) {
            Some(bi) => {
                let bi = if sign < 0 { -bi } else { bi };
                Ok(vec![Value::Int(bi)])
            }
            None => Ok(vec![Value::Nil]),
        };
    }

    match val {
        Value::Int(i) => Ok(vec![Value::Int(i.clone())]),
        Value::Float(f) => Ok(vec![Value::Float(*f)]),
        Value::String(s) => {
            if s.len() > 65_536 {
                return Ok(vec![Value::Nil]);
            }
            let s_trimmed = s.trim();
            let (sign, s_rest) = if let Some(stripped) = s_trimmed.strip_prefix('-') {
                (-1, stripped.trim_start())
            } else if let Some(stripped) = s_trimmed.strip_prefix('+') {
                (1, stripped.trim_start())
            } else {
                (1, s_trimmed)
            };
            if let Some(stripped_hex) = s_rest
                .strip_prefix("0x")
                .or_else(|| s_rest.strip_prefix("0X"))
                && !stripped_hex.is_empty()
                && let Some(bi) = BigInt::parse_bytes(stripped_hex.as_bytes(), 16)
            {
                let bi = if sign < 0 { -bi } else { bi };
                return Ok(vec![Value::Int(bi)]);
            }
            if let Ok(i) = s_trimmed.parse::<i64>() {
                Ok(vec![Value::Int(BigInt::from(i))])
            } else if let Ok(bi) = BigInt::from_str(s_trimmed) {
                Ok(vec![Value::Int(bi)])
            } else if let Ok(f) = s_trimmed.parse::<f64>() {
                Ok(vec![Value::Float(f)])
            } else {
                Ok(vec![Value::Nil])
            }
        }
        _ => Ok(vec![Value::Nil]),
    }
}

pub fn builtin_int(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    match val {
        Value::Int(i) => Ok(vec![Value::Int(i.clone())]),
        Value::Float(f) => Ok(vec![Value::Int(
            BigInt::from_f64(f.trunc()).unwrap_or_default(),
        )]),
        Value::String(s) => {
            if s.len() > 65_536 {
                return Err("integer string exceeds maximum length limit (65536 bytes)".to_string());
            }
            let bi = BigInt::parse_bytes(s.as_bytes(), 10)
                .ok_or_else(|| "invalid integer string".to_string())?;
            Ok(vec![Value::Int(bi)])
        }
        _ => Err("int expects number or string".to_string()),
    }
}

pub fn builtin_float(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    match val {
        Value::Float(f) => Ok(vec![Value::Float(*f)]),
        Value::Int(i) => Ok(vec![Value::Float(i.to_f64().unwrap_or(0.0))]),
        Value::String(s) => {
            let f = s
                .parse::<f64>()
                .map_err(|_| "invalid float string".to_string())?;
            Ok(vec![Value::Float(f)])
        }
        _ => Err("float expects number or string".to_string()),
    }
}

pub fn builtin_error(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let msg = args
        .first()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "error".to_string());
    Err(msg)
}

pub fn builtin_pcall(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let func = args
        .first()
        .ok_or_else(|| "pcall expects at least 1 argument".to_string())?;
    match vm.call_function(func.clone(), &args[1..]) {
        Ok(mut res) => {
            res.insert(0, Value::Bool(true));
            Ok(res)
        }
        Err(err) => Ok(vec![Value::Bool(false), Value::String(err)]),
    }
}

pub fn builtin_xpcall(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let func = args
        .first()
        .ok_or_else(|| "xpcall expects at least 2 arguments".to_string())?;
    let err_handler = args
        .get(1)
        .ok_or_else(|| "xpcall expects at least 2 arguments".to_string())?;
    let call_args = if args.len() > 2 { &args[2..] } else { &[] };
    match vm.call_function(func.clone(), call_args) {
        Ok(mut res) => {
            res.insert(0, Value::Bool(true));
            Ok(res)
        }
        Err(err) => {
            let err_val = Value::String(err);
            match vm.call_function(err_handler.clone(), std::slice::from_ref(&err_val)) {
                Ok(h_res) => {
                    let ret = h_res.into_iter().next().unwrap_or(err_val);
                    Ok(vec![Value::Bool(false), ret])
                }
                Err(h_err) => Ok(vec![
                    Value::Bool(false),
                    Value::String(format!("error in error handling: {}", h_err)),
                ]),
            }
        }
    }
}

pub fn builtin_require(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let pkg = match args
        .first()
        .ok_or_else(|| "require expects a module path".to_string())?
    {
        Value::String(s) => s.as_str(),
        _ => return Err("require expects string argument".to_string()),
    };

    let mod_name = match pkg {
        "@neyuki/math" | "math" => "math",
        "@neyuki/table" | "table" => "table",
        "@neyuki/string" | "string" => "string",
        "@neyuki/bit" | "bit" | "@neyuki/bit32" | "bit32" => "bit",
        "@neyuki/buffer" | "buffer" => "buffer",
        "@neyuki/os" | "os" => "os",
        "@neyuki/coroutine" | "coroutine" => "coroutine",
        "@neyuki/debug" | "debug" => "debug",
        "@neyuki/json" | "json" => "json",
        "@neyuki/utf8" | "utf8" => "utf8",
        "@neyuki/crypto" | "crypto" => "crypto",
        other => {
            let path = if other.ends_with(".nyk") || other.ends_with(".nykb") {
                other.to_string()
            } else {
                format!("{}.nyk", other)
            };
            let path_obj = std::path::Path::new(&path);
            if other.contains('\0')
                || other.contains("..")
                || other.starts_with('/')
                || other.starts_with('\\')
                || path_obj.is_absolute()
                || path_obj.components().any(|c| {
                    matches!(
                        c,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            {
                return Err(format!(
                    "security error: path traversal forbidden in require: '{}'",
                    pkg
                ));
            }
            if let Ok(bytes) = std::fs::read(&path) {
                let proto = if bytes.starts_with(crate::bytecode::MAGIC) || path.ends_with(".nykb")
                {
                    let p = crate::bytecode::deserialize(&bytes)?;
                    crate::bytecode::verify_proto(&p)
                        .map_err(|e| format!("bytecode verification failed: {}", e))?;
                    p
                } else {
                    let src = std::str::from_utf8(&bytes)
                        .map_err(|_| format!("cannot read module '{}': invalid UTF-8", pkg))?;
                    let stmts = crate::compiler::compile_source(src)?;
                    let diags = crate::sema::analyze(&stmts, src);
                    if let Some(err) = diags
                        .iter()
                        .find(|d| d.severity == crate::diagnostics::severity::Severity::Error)
                    {
                        return Err(format!(
                            "semantic error in module '{}': {}",
                            pkg, err.message
                        ));
                    }
                    let p = crate::compiler::try_compile_to_proto(&stmts)?;
                    crate::bytecode::verify_proto(&p)
                        .map_err(|e| format!("bytecode verification failed: {}", e))?;
                    p
                };
                let val = vm.execute(proto)?;
                return Ok(vec![val]);
            }
            return Err(format!("cannot find module '{}'", pkg));
        }
    };

    if let Some(val) = vm.globals.get(mod_name).cloned() {
        Ok(vec![val])
    } else {
        Err(format!("module '{}' not found", mod_name))
    }
}
