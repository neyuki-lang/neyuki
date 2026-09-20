// Arithmetic, bitwise, and comparison evaluation operations for Neyuki VM.
//
// Every operator has an `i64` fast path that the interpreter loop inlines,
// and falls back to `BigInt` when an operand is already big or the `i64`
// result overflows. Results always go through `Value::from_bigint`, which
// demotes back to `Int` whenever the answer fits.

use num_bigint::{BigInt, Sign};
use num_integer::Integer as _;
use num_traits::{FromPrimitive, Signed, ToPrimitive, Zero};

use crate::vm::value::Value;

pub fn to_bigint(v: &Value) -> Result<BigInt, String> {
    match v {
        Value::Int(i) => Ok(BigInt::from(*i)),
        Value::BigInt(i) => Ok((**i).clone()),
        Value::Float(f) => {
            BigInt::from_f64(f.trunc()).ok_or_else(|| "cannot convert float to integer".to_string())
        }
        _ => Err("expected integer".to_string()),
    }
}

#[inline]
pub fn to_f64(v: &Value) -> Result<f64, String> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => Ok(*i as f64),
        Value::BigInt(i) => i
            .to_f64()
            .ok_or_else(|| "integer overflow in float conversion".to_string()),
        _ => Err("expected number".to_string()),
    }
}

/// Both operands as `BigInt`s if both are integers of any size.
#[inline]
fn big_pair(a: &Value, b: &Value) -> Option<(BigInt, BigInt)> {
    let ia = match a {
        Value::Int(i) => BigInt::from(*i),
        Value::BigInt(i) => (**i).clone(),
        _ => return None,
    };
    let ib = match b {
        Value::Int(i) => BigInt::from(*i),
        Value::BigInt(i) => (**i).clone(),
        _ => return None,
    };
    Some((ia, ib))
}

#[inline]
pub fn eval_add(a: &Value, b: &Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (a, b) {
        return Ok(match ia.checked_add(*ib) {
            Some(r) => Value::Int(r),
            None => Value::from_bigint(BigInt::from(*ia) + BigInt::from(*ib)),
        });
    }
    if let Some((ia, ib)) = big_pair(a, b) {
        return Ok(Value::from_bigint(ia + ib));
    }
    Ok(Value::Float(to_f64(a)? + to_f64(b)?))
}

#[inline]
pub fn eval_sub(a: &Value, b: &Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (a, b) {
        return Ok(match ia.checked_sub(*ib) {
            Some(r) => Value::Int(r),
            None => Value::from_bigint(BigInt::from(*ia) - BigInt::from(*ib)),
        });
    }
    if let Some((ia, ib)) = big_pair(a, b) {
        return Ok(Value::from_bigint(ia - ib));
    }
    Ok(Value::Float(to_f64(a)? - to_f64(b)?))
}

#[inline]
pub fn eval_mul(a: &Value, b: &Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (a, b) {
        return Ok(match ia.checked_mul(*ib) {
            Some(r) => Value::Int(r),
            None => Value::from_bigint(BigInt::from(*ia) * BigInt::from(*ib)),
        });
    }
    if let Some((ia, ib)) = big_pair(a, b) {
        return Ok(Value::from_bigint(ia * ib));
    }
    Ok(Value::Float(to_f64(a)? * to_f64(b)?))
}

#[inline]
pub fn eval_div(a: &Value, b: &Value) -> Result<Value, String> {
    // `/` is always floating point and follows IEEE 754, so dividing by zero
    // gives an infinity or NaN. Only `//` and `%` treat it as an error.
    Ok(Value::Float(to_f64(a)? / to_f64(b)?))
}

pub fn eval_idiv(a: &Value, b: &Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (a, b) {
        if *ib == 0 {
            return Err("division by zero".to_string());
        }
        // The only overflowing case is i64::MIN // -1.
        return Ok(match ia.checked_div_euclid(*ib) {
            Some(_) => Value::Int(ia.div_floor(ib)),
            None => Value::from_bigint(BigInt::from(*ia).div_floor(&BigInt::from(*ib))),
        });
    }
    if let Some((ia, ib)) = big_pair(a, b) {
        if ib.is_zero() {
            return Err("division by zero".to_string());
        }
        return Ok(Value::from_bigint(ia.div_floor(&ib)));
    }
    let fa = to_f64(a)?;
    let fb = to_f64(b)?;
    if fb == 0.0 {
        return Err("division by zero".to_string());
    }
    Ok(Value::Float((fa / fb).floor()))
}

pub fn eval_mod(a: &Value, b: &Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (a, b) {
        if *ib == 0 {
            return Err("modulo by zero".to_string());
        }
        // i64::MIN % -1 overflows in Rust but is 0 mathematically.
        return Ok(if *ib == -1 {
            Value::Int(0)
        } else {
            Value::Int(ia.mod_floor(ib))
        });
    }
    if let Some((ia, ib)) = big_pair(a, b) {
        if ib.is_zero() {
            return Err("modulo by zero".to_string());
        }
        return Ok(Value::from_bigint(ia.mod_floor(&ib)));
    }
    let fa = to_f64(a)?;
    let fb = to_f64(b)?;
    if fb == 0.0 {
        return Err("modulo by zero".to_string());
    }
    Ok(Value::Float(fa - (fa / fb).floor() * fb))
}

pub fn eval_pow(a: &Value, b: &Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (a, b)
        && *ib >= 0
        && let Ok(exp) = u32::try_from(*ib)
    {
        return Ok(match ia.checked_pow(exp) {
            Some(r) => Value::Int(r),
            None => Value::from_bigint(BigInt::from(*ia).pow(exp)),
        });
    }
    if let Some((ia, ib)) = big_pair(a, b)
        && ib.sign() != Sign::Minus
        && let Some(exp) = ib.to_u32()
    {
        return Ok(Value::from_bigint(ia.pow(exp)));
    }
    Ok(Value::Float(to_f64(a)?.powf(to_f64(b)?)))
}

pub fn eval_bitand(a: &Value, b: &Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (a, b) {
        return Ok(Value::Int(ia & ib));
    }
    Ok(Value::from_bigint(to_bigint(a)? & to_bigint(b)?))
}

pub fn eval_bitor(a: &Value, b: &Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (a, b) {
        return Ok(Value::Int(ia | ib));
    }
    Ok(Value::from_bigint(to_bigint(a)? | to_bigint(b)?))
}

pub fn eval_bitxor(a: &Value, b: &Value) -> Result<Value, String> {
    if let (Value::Int(ia), Value::Int(ib)) = (a, b) {
        return Ok(Value::Int(ia ^ ib));
    }
    Ok(Value::from_bigint(to_bigint(a)? ^ to_bigint(b)?))
}

/// Arithmetic shift left that widens instead of dropping bits.
fn shift_left(ia: &Value, shift: usize) -> Result<Value, String> {
    if let Value::Int(i) = ia {
        if *i == 0 {
            return Ok(Value::Int(0));
        }
        // Shifting must not change the sign or lose a set bit: the number of
        // leading sign bits says how far it can go without widening.
        let headroom = if *i >= 0 {
            i.leading_zeros() - 1
        } else {
            i.leading_ones() - 1
        } as usize;
        if shift <= headroom {
            return Ok(Value::Int(i << shift));
        }
    }
    Ok(Value::from_bigint(to_bigint(ia)? << shift))
}

/// Arithmetic shift right (floors towards negative infinity, like BigInt).
fn shift_right(ia: &Value, shift: usize) -> Result<Value, String> {
    if let Value::Int(i) = ia {
        return Ok(Value::Int(if shift >= 64 {
            if *i < 0 { -1 } else { 0 }
        } else {
            i >> shift
        }));
    }
    Ok(Value::from_bigint(to_bigint(ia)? >> shift))
}

fn shift_amount(b: &Value) -> Result<(bool, usize), String> {
    match b {
        Value::Int(i) => Ok((*i < 0, i.unsigned_abs() as usize)),
        Value::BigInt(_) => Err("shift is too large".to_string()),
        Value::Float(f) => {
            let t = f.trunc();
            Ok((t < 0.0, t.abs() as usize))
        }
        _ => Err("expected integer".to_string()),
    }
}

pub fn eval_shl(a: &Value, b: &Value) -> Result<Value, String> {
    if !a.is_number() {
        return Err("expected integer".to_string());
    }
    let (negative, shift) = shift_amount(b)?;
    if negative {
        shift_right(a, shift)
    } else {
        shift_left(a, shift)
    }
}

pub fn eval_shr(a: &Value, b: &Value) -> Result<Value, String> {
    if !a.is_number() {
        return Err("expected integer".to_string());
    }
    let (negative, shift) = shift_amount(b)?;
    if negative {
        shift_left(a, shift)
    } else {
        shift_right(a, shift)
    }
}

/// The low 64 bits of an integer, in two's complement.
fn low_word(v: &Value) -> Result<u64, String> {
    match v {
        Value::Int(i) => Ok(*i as u64),
        _ => {
            let big = to_bigint(v)?;
            Ok((big & BigInt::from(u64::MAX)).to_u64().unwrap_or(0))
        }
    }
}

pub fn eval_lshl(a: &Value, b: &Value) -> Result<Value, String> {
    let word = low_word(a)?;
    let bits = to_bigint(b)?.to_usize().unwrap_or(usize::MAX);
    let res = if bits >= 64 { 0 } else { word << bits };
    Ok(Value::from_u64(res))
}

pub fn eval_lshr(a: &Value, b: &Value) -> Result<Value, String> {
    let word = low_word(a)?;
    let bits = to_bigint(b)?.to_usize().unwrap_or(usize::MAX);
    let res = if bits >= 64 { 0 } else { word >> bits };
    Ok(Value::from_u64(res))
}

pub fn eval_bitnot(a: &Value) -> Result<Value, String> {
    if let Value::Int(i) = a {
        return Ok(Value::Int(!i));
    }
    Ok(Value::from_bigint(!to_bigint(a)?))
}

pub fn eval_unm(a: &Value) -> Result<Value, String> {
    match a {
        Value::Int(i) => Ok(match i.checked_neg() {
            Some(r) => Value::Int(r),
            None => Value::from_bigint(-BigInt::from(*i)),
        }),
        Value::BigInt(b) => Ok(Value::from_bigint(-(**b).clone())),
        Value::Float(f) => Ok(Value::Float(-f)),
        _ => Err("unary minus expects a number".to_string()),
    }
}

/// Orders two numbers, or `None` when a NaN makes them unordered. An
/// out-of-range `BigInt` compares against an `Int` by its sign alone, so no
/// digits are touched on that path either.
#[inline]
fn num_cmp(a: &Value, b: &Value) -> Result<Option<std::cmp::Ordering>, String> {
    use std::cmp::Ordering;
    Ok(Some(match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Int(_), Value::BigInt(y)) => {
            if y.is_negative() {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (Value::BigInt(x), Value::Int(_)) => {
            if x.is_negative() {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (Value::BigInt(x), Value::BigInt(y)) => x.cmp(y),
        _ => return Ok(to_f64(a)?.partial_cmp(&to_f64(b)?)),
    }))
}

macro_rules! compare_op {
    ($name:ident, $op:tt, $($pat:pat_param)|+) => {
        #[inline]
        pub fn $name(a: &Value, b: &Value) -> Result<bool, String> {
            if let (Value::Int(ia), Value::Int(ib)) = (a, b) {
                return Ok(ia $op ib);
            }
            if let (Value::String(sa), Value::String(sb)) = (a, b) {
                return Ok(sa $op sb);
            }
            Ok(matches!(num_cmp(a, b)?, Some($($pat)|+)))
        }
    };
}

compare_op!(eval_lt, <, std::cmp::Ordering::Less);
compare_op!(eval_le, <=, std::cmp::Ordering::Less | std::cmp::Ordering::Equal);
compare_op!(eval_gt, >, std::cmp::Ordering::Greater);
compare_op!(eval_ge, >=, std::cmp::Ordering::Greater | std::cmp::Ordering::Equal);
