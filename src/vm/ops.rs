// Arithmetic, bitwise, and comparison evaluation operations for Neyuki VM.

use num_bigint::{BigInt, Sign};
use num_integer::Integer as _;
use num_traits::{FromPrimitive, ToPrimitive, Zero};

use crate::vm::value::Value;

pub fn to_bigint(v: Value) -> Result<BigInt, String> {
    match v {
        Value::Int(i) => Ok(i),
        Value::Float(f) => BigInt::from_f64(f.trunc()).ok_or_else(|| "cannot convert float to integer".to_string()),
        _ => Err("expected integer".to_string()),
    }
}

pub fn to_f64(v: Value) -> Result<f64, String> {
    match v {
        Value::Float(f) => Ok(f),
        Value::Int(i) => i.to_f64().ok_or_else(|| "integer overflow in float conversion".to_string()),
        _ => Err("expected number".to_string()),
    }
}

pub fn eval_add(a: Value, b: Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
        return Ok(Value::Int(ia + ib));
    }
    let fa = to_f64(a)?;
    let fb = to_f64(b)?;
    Ok(Value::Float(fa + fb))
}

pub fn eval_sub(a: Value, b: Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
        return Ok(Value::Int(ia - ib));
    }
    let fa = to_f64(a)?;
    let fb = to_f64(b)?;
    Ok(Value::Float(fa - fb))
}

pub fn eval_mul(a: Value, b: Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
        return Ok(Value::Int(ia * ib));
    }
    let fa = to_f64(a)?;
    let fb = to_f64(b)?;
    Ok(Value::Float(fa * fb))
}

pub fn eval_div(a: Value, b: Value) -> Result<Value, String> {
    let fa = to_f64(a)?;
    let fb = to_f64(b)?;
    if fb == 0.0 {
        return Err("division by zero".to_string());
    }
    Ok(Value::Float(fa / fb))
}

pub fn eval_idiv(a: Value, b: Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
        if ib.is_zero() {
            return Err("division by zero".to_string());
        }
        return Ok(Value::Int(ia.div_floor(ib)));
    }
    let fa = to_f64(a)?;
    let fb = to_f64(b)?;
    if fb == 0.0 {
        return Err("division by zero".to_string());
    }
    Ok(Value::Float((fa / fb).floor()))
}

pub fn eval_mod(a: Value, b: Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (&a, &b) {
        if ib.is_zero() {
            return Err("modulo by zero".to_string());
        }
        return Ok(Value::Int(ia.mod_floor(ib)));
    }
    let fa = to_f64(a)?;
    let fb = to_f64(b)?;
    if fb == 0.0 {
        return Err("modulo by zero".to_string());
    }
    Ok(Value::Float(fa % fb))
}

pub fn eval_pow(a: Value, b: Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (&a, &b)
        && ib.sign() != Sign::Minus
            && let Some(exp) = ib.to_u32() {
                return Ok(Value::Int(ia.pow(exp)));
            }
    let fa = to_f64(a)?;
    let fb = to_f64(b)?;
    Ok(Value::Float(fa.powf(fb)))
}

pub fn eval_bitand(a: Value, b: Value) -> Result<Value, String> {
    let ia = to_bigint(a)?;
    let ib = to_bigint(b)?;
    Ok(Value::Int(ia & ib))
}

pub fn eval_bitor(a: Value, b: Value) -> Result<Value, String> {
    let ia = to_bigint(a)?;
    let ib = to_bigint(b)?;
    Ok(Value::Int(ia | ib))
}

pub fn eval_bitxor(a: Value, b: Value) -> Result<Value, String> {
    let ia = to_bigint(a)?;
    let ib = to_bigint(b)?;
    Ok(Value::Int(ia ^ ib))
}

pub fn eval_shl(a: Value, b: Value) -> Result<Value, String> {
    let ia = to_bigint(a)?;
    let ib = to_bigint(b)?;
    if ib.sign() == Sign::Minus {
        let shift = (-ib).to_usize().ok_or_else(|| "shift is too large".to_string())?;
        Ok(Value::Int(ia >> shift))
    } else {
        let shift = ib.to_usize().ok_or_else(|| "shift is too large".to_string())?;
        Ok(Value::Int(ia << shift))
    }
}

pub fn eval_shr(a: Value, b: Value) -> Result<Value, String> {
    let ia = to_bigint(a)?;
    let ib = to_bigint(b)?;
    if ib.sign() == Sign::Minus {
        let shift = (-ib).to_usize().ok_or_else(|| "shift is too large".to_string())?;
        Ok(Value::Int(ia << shift))
    } else {
        let shift = ib.to_usize().ok_or_else(|| "shift is too large".to_string())?;
        Ok(Value::Int(ia >> shift))
    }
}

pub fn eval_lshl(a: Value, b: Value) -> Result<Value, String> {
    let ia = to_bigint(a)?;
    let ib = to_bigint(b)?;
    let word = (ia & BigInt::from(u64::MAX)).to_u64().unwrap_or(0);
    let bits = ib.to_usize().unwrap_or(usize::MAX);
    let res = if bits >= 64 { 0 } else { word << bits };
    Ok(Value::Int(BigInt::from(res)))
}

pub fn eval_lshr(a: Value, b: Value) -> Result<Value, String> {
    let ia = to_bigint(a)?;
    let ib = to_bigint(b)?;
    let word = (ia & BigInt::from(u64::MAX)).to_u64().unwrap_or(0);
    let bits = ib.to_usize().unwrap_or(usize::MAX);
    let res = if bits >= 64 { 0 } else { word >> bits };
    Ok(Value::Int(BigInt::from(res)))
}

pub fn eval_lt(a: &Value, b: &Value) -> Result<bool, String> {
    match (a, b) {
        (Value::Int(ia), Value::Int(ib)) => Ok(ia < ib),
        (Value::String(sa), Value::String(sb)) => Ok(sa < sb),
        _ => Ok(to_f64(a.clone())? < to_f64(b.clone())?),
    }
}

pub fn eval_le(a: &Value, b: &Value) -> Result<bool, String> {
    match (a, b) {
        (Value::Int(ia), Value::Int(ib)) => Ok(ia <= ib),
        (Value::String(sa), Value::String(sb)) => Ok(sa <= sb),
        _ => Ok(to_f64(a.clone())? <= to_f64(b.clone())?),
    }
}

pub fn eval_gt(a: &Value, b: &Value) -> Result<bool, String> {
    match (a, b) {
        (Value::Int(ia), Value::Int(ib)) => Ok(ia > ib),
        (Value::String(sa), Value::String(sb)) => Ok(sa > sb),
        _ => Ok(to_f64(a.clone())? > to_f64(b.clone())?),
    }
}

pub fn eval_ge(a: &Value, b: &Value) -> Result<bool, String> {
    match (a, b) {
        (Value::Int(ia), Value::Int(ib)) => Ok(ia >= ib),
        (Value::String(sa), Value::String(sb)) => Ok(sa >= sb),
        _ => Ok(to_f64(a.clone())? >= to_f64(b.clone())?),
    }
}
