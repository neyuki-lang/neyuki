// Standard string manipulation library for Neyuki VM.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

fn to_string_arg(val: &Value) -> Result<String, String> {
    match val {
        Value::String(s) => Ok(s.clone()),
        Value::Int(i) => Ok(i.to_string()),
        Value::Float(f) => Ok(f.to_string()),
        _ => Err("string library expects string argument".to_string()),
    }
}

fn to_isize(val: &Value, name: &str) -> Result<isize, String> {
    match val {
        Value::Int(i) => i.to_isize().ok_or_else(|| format!("{} is out of bounds", name)),
        Value::Float(f) => Ok(*f as isize),
        _ => Err(format!("{} expects an integer", name)),
    }
}

fn string_len(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = to_string_arg(args.first().ok_or_else(|| "string.len expects 1 argument".to_string())?)?;
    Ok(vec![Value::Int(BigInt::from(s.len()))])
}

fn string_lower(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = to_string_arg(args.first().ok_or_else(|| "string.lower expects 1 argument".to_string())?)?;
    Ok(vec![Value::String(s.to_lowercase())])
}

fn string_upper(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = to_string_arg(args.first().ok_or_else(|| "string.upper expects 1 argument".to_string())?)?;
    Ok(vec![Value::String(s.to_uppercase())])
}

fn string_reverse(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = to_string_arg(args.first().ok_or_else(|| "string.reverse expects 1 argument".to_string())?)?;
    Ok(vec![Value::String(s.chars().rev().collect())])
}

fn string_rep(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = to_string_arg(args.first().ok_or_else(|| "string.rep expects at least 2 arguments".to_string())?)?;
    let n = to_isize(args.get(1).ok_or_else(|| "string.rep expects at least 2 arguments".to_string())?, "n")?;
    if n <= 0 {
        return Ok(vec![Value::String(String::new())]);
    }
    const MAX_REP_COUNT: isize = 10_000_000;
    if n > MAX_REP_COUNT {
        return Err(format!("count exceeds maximum limit ({}) in 'string.rep'", MAX_REP_COUNT));
    }
    let sep = if let Some(sep_val) = args.get(2) {
        to_string_arg(sep_val)?
    } else {
        String::new()
    };
    if s.is_empty() && sep.is_empty() {
        return Ok(vec![Value::String(String::new())]);
    }
    let unit = s.len() + sep.len();
    let total = unit.saturating_mul(n as usize);
    if total > i32::MAX as usize || total > 100 * 1024 * 1024 {
        return Err(format!("string.rep result too large: {} bytes", total));
    }
    let mut out = String::with_capacity(total);
    for i in 0..n {
        if i > 0 && !sep.is_empty() {
            out.push_str(&sep);
        }
        out.push_str(&s);
    }
    Ok(vec![Value::String(out)])
}

fn string_sub(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = to_string_arg(args.first().ok_or_else(|| "string.sub expects at least 2 arguments".to_string())?)?;
    let i = to_isize(args.get(1).ok_or_else(|| "string.sub expects at least 2 arguments".to_string())?, "start")?;
    let j = if let Some(jv) = args.get(2) {
        to_isize(jv, "end")?
    } else {
        -1
    };

    let len = s.len() as isize;
    let start = if i > 0 {
        (i - 1).min(len) as usize
    } else if i < 0 {
        (len + i).max(0) as usize
    } else {
        0
    };

    let end = if j > 0 {
        j.min(len) as usize
    } else if j < 0 {
        (len + j + 1).max(0) as usize
    } else {
        0
    };

    if start >= end || start >= s.len() {
        Ok(vec![Value::String(String::new())])
    } else {
        Ok(vec![Value::String(s[start..end].to_string())])
    }
}

fn string_byte(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = to_string_arg(args.first().ok_or_else(|| "string.byte expects at least 1 argument".to_string())?)?;
    let i = if let Some(iv) = args.get(1) {
        to_isize(iv, "start")?
    } else {
        1
    };
    let j = if let Some(jv) = args.get(2) {
        to_isize(jv, "end")?
    } else {
        i
    };

    let bytes = s.as_bytes();
    let len = bytes.len() as isize;
    let start = if i > 0 {
        (i - 1).min(len) as usize
    } else if i < 0 {
        (len + i).max(0) as usize
    } else {
        0
    };

    let end = if j > 0 {
        j.min(len) as usize
    } else if j < 0 {
        (len + j + 1).max(0) as usize
    } else {
        0
    };

    let mut res = Vec::new();
    if start < end && start < bytes.len() {
        for b in &bytes[start..end] {
            res.push(Value::Int(BigInt::from(*b)));
        }
    }
    Ok(res)
}

fn string_char(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut bytes = Vec::with_capacity(args.len());
    for a in args {
        let b = to_isize(a, "byte code")?;
        if !(0..=255).contains(&b) {
            return Err("string.char value out of range (0..255)".to_string());
        }
        bytes.push(b as u8);
    }
    let s = String::from_utf8(bytes).map_err(|e| format!("invalid utf-8: {}", e))?;
    Ok(vec![Value::String(s)])
}

fn string_split(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = to_string_arg(args.first().ok_or_else(|| "string.split expects at least 1 argument".to_string())?)?;
    let sep = if let Some(v) = args.get(1) {
        to_string_arg(v)?
    } else {
        ",".to_string()
    };

    let parts: Vec<Value> = if sep.is_empty() {
        s.chars().map(|c| Value::String(c.to_string())).collect()
    } else {
        s.split(&sep).map(|p| Value::String(p.to_string())).collect()
    };

    let mut table = VmTable::new();
    table.array = parts;
    let rc = Rc::new(RefCell::new(table));
    vm.gc.register_table(&rc);
    Ok(vec![Value::Table(rc)])
}

fn string_find(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = to_string_arg(args.first().ok_or_else(|| "string.find expects 2 arguments".to_string())?)?;
    let pattern = to_string_arg(args.get(1).ok_or_else(|| "string.find expects 2 arguments".to_string())?)?;

    if let Some(pos) = s.find(&pattern) {
        let start = pos + 1;
        let end = pos + pattern.len();
        Ok(vec![
            Value::Int(BigInt::from(start)),
            Value::Int(BigInt::from(end)),
        ])
    } else {
        Ok(vec![Value::Nil])
    }
}

fn string_format(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let fmt = to_string_arg(args.first().ok_or_else(|| "string.format expects format string".to_string())?)?;
    let mut out = String::new();
    let mut arg_idx = 1;
    let bytes = fmt.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            i += 1;
            if i >= bytes.len() {
                return Err("incomplete format specifier".to_string());
            }
            if bytes[i] == b'%' {
                out.push('%');
                i += 1;
                continue;
            }
            let val = args.get(arg_idx).ok_or_else(|| "not enough arguments to string.format".to_string())?;
            arg_idx += 1;
            match bytes[i] {
                b's' => out.push_str(&val.to_string()),
                b'd' | b'i' => {
                    let num = match val {
                        Value::Int(n) => n.to_string(),
                        Value::Float(f) => (*f as i64).to_string(),
                        Value::String(s) => s.clone(),
                        _ => return Err("format specifier expects number".to_string()),
                    };
                    out.push_str(&num);
                }
                b'f' => {
                    let num = match val {
                        Value::Float(f) => format!("{:.6}", f),
                        Value::Int(n) => format!("{:.6}", n.to_f64().unwrap_or(0.0)),
                        _ => return Err("format specifier expects number".to_string()),
                    };
                    out.push_str(&num);
                }
                b'x' => {
                    let num = match val {
                        Value::Int(n) => format!("{:x}", n),
                        Value::Float(f) => format!("{:x}", *f as i64),
                        _ => return Err("format specifier expects integer".to_string()),
                    };
                    out.push_str(&num);
                }
                b'X' => {
                    let num = match val {
                        Value::Int(n) => format!("{:X}", n),
                        Value::Float(f) => format!("{:X}", *f as i64),
                        _ => return Err("format specifier expects integer".to_string()),
                    };
                    out.push_str(&num);
                }
                b'c' => {
                    let ch = match val {
                        Value::Int(n) => n.to_u32().and_then(char::from_u32).unwrap_or('?'),
                        Value::Float(f) => char::from_u32(*f as u32).unwrap_or('?'),
                        _ => return Err("format specifier expects integer codepoint".to_string()),
                    };
                    out.push(ch);
                }
                b'q' => {
                    out.push('"');
                    out.push_str(&val.to_string().replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"));
                    out.push('"');
                }
                other => {
                    return Err(format!("unsupported format specifier '%{}'", other as char));
                }
            }
            i += 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok(vec![Value::String(out)])
}

pub fn create_string_lib() -> Value {
    let mut table = VmTable::new();
    table.set_str("len", Value::Native("string.len", string_len));
    table.set_str("lower", Value::Native("string.lower", string_lower));
    table.set_str("upper", Value::Native("string.upper", string_upper));
    table.set_str("reverse", Value::Native("string.reverse", string_reverse));
    table.set_str("rep", Value::Native("string.rep", string_rep));
    table.set_str("sub", Value::Native("string.sub", string_sub));
    table.set_str("byte", Value::Native("string.byte", string_byte));
    table.set_str("char", Value::Native("string.char", string_char));
    table.set_str("split", Value::Native("string.split", string_split));
    table.set_str("find", Value::Native("string.find", string_find));
    table.set_str("format", Value::Native("string.format", string_format));
    table.frozen = true;
    Value::Table(Rc::new(RefCell::new(table)))
}
