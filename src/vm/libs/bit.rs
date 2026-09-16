// Standard 32-bit bitwise manipulation library for Neyuki VM.

use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

fn to_u32(val: &Value) -> Result<u32, String> {
    match val {
        Value::Int(i) => Ok(i.to_u32().unwrap_or(0)),
        Value::Float(f) => Ok(*f as i64 as u32),
        _ => Err("bit library expects number".to_string()),
    }
}

fn to_i32(val: &Value) -> Result<i32, String> {
    match val {
        Value::Int(i) => Ok(i.to_i32().unwrap_or(0)),
        Value::Float(f) => Ok(*f as i64 as i32),
        _ => Err("bit library expects number".to_string()),
    }
}

fn bit_band(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    if args.is_empty() {
        return Ok(vec![Value::Int(BigInt::from(0xFFFFFFFFu32))]);
    }
    let mut res = to_u32(&args[0])?;
    for a in &args[1..] {
        res &= to_u32(a)?;
    }
    Ok(vec![Value::Int(BigInt::from(res))])
}

fn bit_bor(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut res = 0u32;
    for a in args {
        res |= to_u32(a)?;
    }
    Ok(vec![Value::Int(BigInt::from(res))])
}

fn bit_bxor(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut res = 0u32;
    for a in args {
        res ^= to_u32(a)?;
    }
    Ok(vec![Value::Int(BigInt::from(res))])
}

fn bit_bnot(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().ok_or_else(|| "bit.bnot expects 1 argument".to_string())?;
    let u = to_u32(val)?;
    Ok(vec![Value::Int(BigInt::from(!u))])
}

fn bit_lshift(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().ok_or_else(|| "bit.lshift expects 2 arguments".to_string())?;
    let disp = args.get(1).ok_or_else(|| "bit.lshift expects 2 arguments".to_string())?;
    let u = to_u32(val)?;
    let s = to_u32(disp)? % 32;
    Ok(vec![Value::Int(BigInt::from(u << s))])
}

fn bit_rshift(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().ok_or_else(|| "bit.rshift expects 2 arguments".to_string())?;
    let disp = args.get(1).ok_or_else(|| "bit.rshift expects 2 arguments".to_string())?;
    let u = to_u32(val)?;
    let s = to_u32(disp)? % 32;
    Ok(vec![Value::Int(BigInt::from(u >> s))])
}

fn bit_arshift(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().ok_or_else(|| "bit.arshift expects 2 arguments".to_string())?;
    let disp = args.get(1).ok_or_else(|| "bit.arshift expects 2 arguments".to_string())?;
    let i = to_i32(val)?;
    let s = to_u32(disp)? % 32;
    Ok(vec![Value::Int(BigInt::from(i >> s))])
}

fn bit_rol(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().ok_or_else(|| "bit.rol expects 2 arguments".to_string())?;
    let disp = args.get(1).ok_or_else(|| "bit.rol expects 2 arguments".to_string())?;
    let u = to_u32(val)?;
    let s = to_u32(disp)? % 32;
    Ok(vec![Value::Int(BigInt::from(u.rotate_left(s)))])
}

fn bit_ror(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().ok_or_else(|| "bit.ror expects 2 arguments".to_string())?;
    let disp = args.get(1).ok_or_else(|| "bit.ror expects 2 arguments".to_string())?;
    let u = to_u32(val)?;
    let s = to_u32(disp)? % 32;
    Ok(vec![Value::Int(BigInt::from(u.rotate_right(s)))])
}

fn bit_btest(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let res = bit_band(_vm, args)?;
    if let Some(Value::Int(i)) = res.first() {
        Ok(vec![Value::Bool(!i.is_zero())])
    } else {
        Ok(vec![Value::Bool(false)])
    }
}

fn bit_extract(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().ok_or_else(|| "bit.extract expects at least 2 arguments".to_string())?;
    let field = args.get(1).ok_or_else(|| "bit.extract expects at least 2 arguments".to_string())?;
    let width = args.get(2);
    let u = to_u32(val)?;
    let f = to_u32(field)?;
    let w = if let Some(wv) = width { to_u32(wv)? } else { 1 };
    if f + w > 32 || w == 0 {
        return Err("invalid bit field or width".to_string());
    }
    let mask = (1u64 << w) - 1;
    let res = ((u >> f) as u64 & mask) as u32;
    Ok(vec![Value::Int(BigInt::from(res))])
}

fn bit_replace(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().ok_or_else(|| "bit.replace expects at least 3 arguments".to_string())?;
    let rep = args.get(1).ok_or_else(|| "bit.replace expects at least 3 arguments".to_string())?;
    let field = args.get(2).ok_or_else(|| "bit.replace expects at least 3 arguments".to_string())?;
    let width = args.get(3);
    let u = to_u32(val)?;
    let v = to_u32(rep)?;
    let f = to_u32(field)?;
    let w = if let Some(wv) = width { to_u32(wv)? } else { 1 };
    if f + w > 32 || w == 0 {
        return Err("invalid bit field or width".to_string());
    }
    let mask = (((1u64 << w) - 1) << f) as u32;
    let res = (u & !mask) | ((v << f) & mask);
    Ok(vec![Value::Int(BigInt::from(res))])
}

fn bit_tohex(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().ok_or_else(|| "bit.tohex expects at least 1 argument".to_string())?;
    let u = to_u32(val)?;
    let n = if let Some(nv) = args.get(1) { to_i32(nv)? } else { 8 };
    let hex_full = format!("{:08x}", u);
    let len = n.unsigned_abs() as usize;
    let res = if len <= 8 {
        hex_full[8 - len..].to_string()
    } else {
        format!("{:0width$x}", u, width = len)
    };
    Ok(vec![Value::String(if n < 0 { res.to_uppercase() } else { res })])
}

fn bit_tobit(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args.first().ok_or_else(|| "bit.tobit expects 1 argument".to_string())?;
    let i = to_i32(val)?;
    Ok(vec![Value::Int(BigInt::from(i))])
}

pub fn create_bit_lib() -> Value {
    let mut table = VmTable::new();
    table.set_str("band", Value::Native("bit.band", bit_band));
    table.set_str("bor", Value::Native("bit.bor", bit_bor));
    table.set_str("bxor", Value::Native("bit.bxor", bit_bxor));
    table.set_str("bnot", Value::Native("bit.bnot", bit_bnot));
    table.set_str("lshift", Value::Native("bit.lshift", bit_lshift));
    table.set_str("rshift", Value::Native("bit.rshift", bit_rshift));
    table.set_str("arshift", Value::Native("bit.arshift", bit_arshift));
    table.set_str("rol", Value::Native("bit.rol", bit_rol));
    table.set_str("ror", Value::Native("bit.ror", bit_ror));
    table.set_str("btest", Value::Native("bit.btest", bit_btest));
    table.set_str("extract", Value::Native("bit.extract", bit_extract));
    table.set_str("replace", Value::Native("bit.replace", bit_replace));
    table.set_str("tohex", Value::Native("bit.tohex", bit_tohex));
    table.set_str("tobit", Value::Native("bit.tobit", bit_tobit));
    table.frozen = true;
    Value::Table(Rc::new(RefCell::new(table)))
}
