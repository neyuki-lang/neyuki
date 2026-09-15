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
    let sep = if let Some(sep_val) = args.get(2) {
        to_string_arg(sep_val)?
    } else {
        String::new()
    };
    let parts = vec![s; n as usize];
    Ok(vec![Value::String(parts.join(&sep))])
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

fn string_split(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let s = to_string_arg(args.first().ok_or_else(|| "string.split expects at least 1 argument".to_string())?)?;
    let sep = if let Some(sep_val) = args.get(1) {
        to_string_arg(sep_val)?
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
    Ok(vec![Value::Table(Rc::new(RefCell::new(table)))])
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
    table.frozen = true;
    Value::Table(Rc::new(RefCell::new(table)))
}
