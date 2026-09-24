// Standard mathematical library for Neyuki VM.

use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};
use rand::Rng;
use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

fn to_f64(val: &Value) -> Result<f64, String> {
    match val {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => i
            .to_f64()
            .ok_or_else(|| "number conversion error".to_string()),
        _ => Err("math library expects number".to_string()),
    }
}

fn math_abs(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args
        .first()
        .ok_or_else(|| "math.abs expects 1 argument".to_string())?;
    match val {
        Value::Int(i) => Ok(vec![match i.checked_abs() {
            Some(a) => Value::Int(a),
            None => Value::from_bigint(num_bigint::BigInt::from(*i).abs()),
        }]),
        Value::BigInt(i) => Ok(vec![Value::from_bigint(num_traits::Signed::abs(&**i))]),
        Value::Float(f) => Ok(vec![Value::Float(f.abs())]),
        _ => Err("math.abs expects number".to_string()),
    }
}

fn math_floor(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.floor expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.floor())])
}

fn math_ceil(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.ceil expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.ceil())])
}

fn math_round(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.round expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.round())])
}

fn math_sqrt(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.sqrt expects 1 argument".to_string())?,
    )?;
    if f < 0.0 {
        return Err("math.sqrt expects non-negative number".to_string());
    }
    Ok(vec![Value::Float(f.sqrt())])
}

/// Direct VM-native `__sqrt` primitive. Contract (values, rejections,
/// messages) is identical to the bridged `runtime::builtin_sqrt` the bundled
/// stdlib was built on, but it reads VM values straight off the argument
/// slice instead of paying the bridge's per-call argument/result conversions
/// (about ~90ns saved per call on sqrt-heavy loops: `__sqrt` used to trail
/// `math.sqrt` by ~25%).
pub(crate) fn primitive_sqrt(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let value = match args.first() {
        Some(Value::Float(f)) => *f,
        Some(Value::Int(i)) => *i as f64,
        Some(Value::BigInt(i)) => (**i)
            .to_f64()
            .ok_or_else(|| "integer is too large for floating-point conversion".to_string())?,
        _ => return Err("expected a number".to_string()),
    };
    if !value.is_finite() {
        return Err("sqrt expects a finite number".to_string());
    }
    if value < 0.0 {
        return Err("sqrt expects a non-negative number".to_string());
    }
    Ok(vec![Value::Float(value.sqrt())])
}

fn math_sin(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.sin expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.sin())])
}

fn math_cos(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.cos expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.cos())])
}

fn math_tan(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.tan expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.tan())])
}

fn math_asin(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.asin expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.asin())])
}

fn math_acos(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.acos expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.acos())])
}

fn math_atan(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.atan expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.atan())])
}

fn math_atan2(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let y = to_f64(
        args.first()
            .ok_or_else(|| "math.atan2 expects 2 arguments".to_string())?,
    )?;
    let x = to_f64(
        args.get(1)
            .ok_or_else(|| "math.atan2 expects 2 arguments".to_string())?,
    )?;
    Ok(vec![Value::Float(y.atan2(x))])
}

fn math_deg(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.deg expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.to_degrees())])
}

fn math_rad(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.rad expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.to_radians())])
}

fn math_exp(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.exp expects 1 argument".to_string())?,
    )?;
    Ok(vec![Value::Float(f.exp())])
}

fn math_log(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let f = to_f64(
        args.first()
            .ok_or_else(|| "math.log expects at least 1 argument".to_string())?,
    )?;
    if let Some(base_val) = args.get(1) {
        let base = to_f64(base_val)?;
        Ok(vec![Value::Float(f.log(base))])
    } else {
        Ok(vec![Value::Float(f.ln())])
    }
}

fn math_min(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    if args.is_empty() {
        return Err("math.min expects at least 1 argument".to_string());
    }
    let mut min_val = to_f64(&args[0])?;
    for a in &args[1..] {
        let v = to_f64(a)?;
        if v < min_val {
            min_val = v;
        }
    }
    Ok(vec![Value::Float(min_val)])
}

fn math_max(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    if args.is_empty() {
        return Err("math.max expects at least 1 argument".to_string());
    }
    let mut max_val = to_f64(&args[0])?;
    for a in &args[1..] {
        let v = to_f64(a)?;
        if v > max_val {
            max_val = v;
        }
    }
    Ok(vec![Value::Float(max_val)])
}

fn math_clamp(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = to_f64(
        args.first()
            .ok_or_else(|| "math.clamp expects 3 arguments".to_string())?,
    )?;
    let min = to_f64(
        args.get(1)
            .ok_or_else(|| "math.clamp expects 3 arguments".to_string())?,
    )?;
    let max = to_f64(
        args.get(2)
            .ok_or_else(|| "math.clamp expects 3 arguments".to_string())?,
    )?;
    if min > max {
        return Err("math.clamp min cannot be greater than max".to_string());
    }
    Ok(vec![Value::Float(val.clamp(min, max))])
}

fn math_sign(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = to_f64(
        args.first()
            .ok_or_else(|| "math.sign expects 1 argument".to_string())?,
    )?;
    let res = if val > 0.0 {
        1.0
    } else if val < 0.0 {
        -1.0
    } else {
        0.0
    };
    Ok(vec![Value::Float(res)])
}

fn math_random(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut rng = rand::thread_rng();
    if args.is_empty() {
        Ok(vec![Value::Float(rng.gen_range(0.0..1.0))])
    } else if args.len() == 1 {
        let m = to_f64(&args[0])? as i64;
        if m < 1 {
            return Err("math.random m must be >= 1".to_string());
        }
        let val = rng.gen_range(1..=m);
        Ok(vec![Value::from_bigint(BigInt::from(val))])
    } else {
        let m = to_f64(&args[0])? as i64;
        let n = to_f64(&args[1])? as i64;
        if m > n {
            return Err("math.random min cannot be greater than max".to_string());
        }
        let val = rng.gen_range(m..=n);
        Ok(vec![Value::from_bigint(BigInt::from(val))])
    }
}

pub fn create_math_lib() -> Value {
    let mut table = VmTable::new();
    table.set_str("abs", crate::native!("math.abs", math_abs));
    table.set_str("floor", crate::native!("math.floor", math_floor));
    table.set_str("ceil", crate::native!("math.ceil", math_ceil));
    table.set_str("round", crate::native!("math.round", math_round));
    table.set_str("sqrt", crate::native!("math.sqrt", math_sqrt));
    table.set_str("sin", crate::native!("math.sin", math_sin));
    table.set_str("cos", crate::native!("math.cos", math_cos));
    table.set_str("tan", crate::native!("math.tan", math_tan));
    table.set_str("asin", crate::native!("math.asin", math_asin));
    table.set_str("acos", crate::native!("math.acos", math_acos));
    table.set_str("atan", crate::native!("math.atan", math_atan));
    table.set_str("atan2", crate::native!("math.atan2", math_atan2));
    table.set_str("deg", crate::native!("math.deg", math_deg));
    table.set_str("rad", crate::native!("math.rad", math_rad));
    table.set_str("exp", crate::native!("math.exp", math_exp));
    table.set_str("log", crate::native!("math.log", math_log));
    table.set_str("min", crate::native!("math.min", math_min));
    table.set_str("max", crate::native!("math.max", math_max));
    table.set_str("clamp", crate::native!("math.clamp", math_clamp));
    table.set_str("sign", crate::native!("math.sign", math_sign));
    table.set_str("random", crate::native!("math.random", math_random));
    table.set_str("pi", Value::Float(std::f64::consts::PI));
    table.set_str("huge", Value::Float(f64::INFINITY));
    table.frozen = true;
    Value::Table(Rc::new(RefCell::new(table)))
}
