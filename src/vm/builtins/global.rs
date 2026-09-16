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
        let msg = args.get(1).map(|v| v.to_string()).unwrap_or_else(|| "assertion failed".to_string());
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
        let handler = t.borrow().metatable.as_ref().and_then(|mt| {
            mt.borrow().fields.get("__tostring").cloned()
        });
        if let Some(h) = handler
            && !matches!(h, Value::Nil) {
                let res = vm.call_function(h, std::slice::from_ref(val))?;
                return Ok(vec![res.into_iter().next().unwrap_or(Value::Nil)]);
            }
    }
    Ok(vec![Value::String(val.to_string())])
}

pub fn builtin_tonumber(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().unwrap_or(&Value::Nil);
    match val {
        Value::Int(i) => Ok(vec![Value::Int(i.clone())]),
        Value::Float(f) => Ok(vec![Value::Float(*f)]),
        Value::String(s) => {
            if let Ok(i) = s.parse::<i64>() {
                Ok(vec![Value::Int(BigInt::from(i))])
            } else if let Ok(bi) = BigInt::from_str(s) {
                Ok(vec![Value::Int(bi)])
            } else if let Ok(f) = s.parse::<f64>() {
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
        Value::Float(f) => Ok(vec![Value::Int(BigInt::from_f64(f.trunc()).unwrap_or_default())]),
        Value::String(s) => {
            let bi = BigInt::parse_bytes(s.as_bytes(), 10).ok_or_else(|| "invalid integer string".to_string())?;
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
            let f = s.parse::<f64>().map_err(|_| "invalid float string".to_string())?;
            Ok(vec![Value::Float(f)])
        }
        _ => Err("float expects number or string".to_string()),
    }
}

pub fn builtin_error(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let msg = args.first().map(|v| v.to_string()).unwrap_or_else(|| "error".to_string());
    Err(msg)
}

pub fn builtin_pcall(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let func = args.first().ok_or_else(|| "pcall expects at least 1 argument".to_string())?;
    match vm.call_function(func.clone(), &args[1..]) {
        Ok(mut res) => {
            res.insert(0, Value::Bool(true));
            Ok(res)
        }
        Err(err) => Ok(vec![Value::Bool(false), Value::String(err)]),
    }
}

pub fn builtin_xpcall(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let func = args.first().ok_or_else(|| "xpcall expects at least 2 arguments".to_string())?;
    let err_handler = args.get(1).ok_or_else(|| "xpcall expects at least 2 arguments".to_string())?;
    let pcall_res = builtin_pcall(vm, std::slice::from_ref(func))?;
    if pcall_res[0] == Value::Bool(true) {
        Ok(pcall_res)
    } else {
        let err_msg = pcall_res.get(1).cloned().unwrap_or(Value::Nil);
        match err_handler {
            Value::Native(_, f) => {
                let h_res = f(vm, &[err_msg])?;
                let ret = h_res.into_iter().next().unwrap_or(Value::Nil);
                Ok(vec![Value::Bool(false), ret])
            }
            _ => Ok(vec![Value::Bool(false), err_msg]),
        }
    }
}

pub fn builtin_require(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let pkg = match args.first().ok_or_else(|| "require expects a module path".to_string())? {
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
            if let Ok(src) = std::fs::read_to_string(&path) {
                let stmts = crate::compiler::compile_source(&src)?;
                let proto = crate::compiler::compile_to_proto(&stmts);
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
