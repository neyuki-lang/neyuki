#![allow(dead_code)]

use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};
use std::str::FromStr;

use crate::parser::Expr;

pub fn fold_builtin_call(callee: &Expr, args: &[Expr]) -> Option<Expr> {
    fold_builtin_call_scoped(callee, args, &std::collections::HashSet::new())
}

pub fn fold_builtin_call_scoped(
    callee: &Expr,
    args: &[Expr],
    shadowed: &std::collections::HashSet<String>,
) -> Option<Expr> {
    let (mod_name, func_name) = match callee {
        Expr::Member { object, field } => match &**object {
            Expr::Variable(name) => {
                if shadowed.contains(name.as_str()) {
                    return None;
                }
                (name.as_str(), field.as_str())
            }
            _ => return None,
        },
        Expr::Variable(name) => {
            if shadowed.contains(name.as_str()) {
                return None;
            }
            // Global builtins e.g. int(), float(), type()
            ("", name.as_str())
        }
        _ => return None,
    };

    match (mod_name, func_name) {
        ("math", "abs") => fold_math_abs(args),
        ("math", "floor") => fold_math_floor(args),
        ("math", "ceil") => fold_math_ceil(args),
        ("math", "sqrt") => fold_math_sqrt(args),
        ("math", "min") => fold_math_min(args),
        ("math", "max") => fold_math_max(args),
        ("math", "pow") => fold_math_pow(args),
        ("bit", "band") => fold_bit_band(args),
        ("bit", "bor") => fold_bit_bor(args),
        ("bit", "bxor") => fold_bit_bxor(args),
        ("bit", "bnot") => fold_bit_bnot(args),
        ("bit", "lshift") => fold_bit_lshift(args),
        ("bit", "rshift") => fold_bit_rshift(args),
        ("string", "len") => fold_string_len(args),
        ("string", "lower") => fold_string_lower(args),
        ("string", "upper") => fold_string_upper(args),
        ("", "int") => fold_builtin_int(args),
        _ => None,
    }
}

fn get_int(expr: &Expr) -> Option<BigInt> {
    match expr {
        Expr::Literal(s) => BigInt::from_str(s).ok(),
        _ => None,
    }
}

fn get_float(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Literal(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

fn get_str(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Str(s) => Some(s.as_str()),
        _ => None,
    }
}

fn fold_math_abs(args: &[Expr]) -> Option<Expr> {
    let arg = args.first()?;
    if let Some(i) = get_int(arg) {
        return Some(Expr::Literal(i.abs().to_string()));
    }
    if let Some(f) = get_float(arg) {
        return Some(Expr::Literal(f.abs().to_string()));
    }
    None
}

fn fold_math_floor(args: &[Expr]) -> Option<Expr> {
    let arg = args.first()?;
    if let Some(i) = get_int(arg) {
        return Some(Expr::Literal(i.to_string()));
    }
    if let Some(f) = get_float(arg) {
        return Some(Expr::Literal(f.floor().to_string()));
    }
    None
}

fn fold_math_ceil(args: &[Expr]) -> Option<Expr> {
    let arg = args.first()?;
    if let Some(i) = get_int(arg) {
        return Some(Expr::Literal(i.to_string()));
    }
    if let Some(f) = get_float(arg) {
        return Some(Expr::Literal(f.ceil().to_string()));
    }
    None
}

fn fold_math_sqrt(args: &[Expr]) -> Option<Expr> {
    let arg = args.first()?;
    let f = get_float(arg)?;
    if f >= 0.0 {
        Some(Expr::Literal(f.sqrt().to_string()))
    } else {
        None
    }
}

fn fold_math_min(args: &[Expr]) -> Option<Expr> {
    if args.is_empty() {
        return None;
    }
    let mut min_val = get_float(args.first()?)?;
    for arg in &args[1..] {
        let f = get_float(arg)?;
        if f < min_val {
            min_val = f;
        }
    }
    Some(Expr::Literal(min_val.to_string()))
}

fn fold_math_max(args: &[Expr]) -> Option<Expr> {
    if args.is_empty() {
        return None;
    }
    let mut max_val = get_float(args.first()?)?;
    for arg in &args[1..] {
        let f = get_float(arg)?;
        if f > max_val {
            max_val = f;
        }
    }
    Some(Expr::Literal(max_val.to_string()))
}

fn fold_math_pow(args: &[Expr]) -> Option<Expr> {
    let a = get_float(args.first()?)?;
    let b = get_float(args.get(1)?)?;
    let res = a.powf(b);
    if res.is_finite() {
        Some(Expr::Literal(res.to_string()))
    } else {
        None
    }
}

fn fold_bit_band(args: &[Expr]) -> Option<Expr> {
    let a = get_int(args.first()?)?;
    let b = get_int(args.get(1)?)?;
    Some(Expr::Literal((a & b).to_string()))
}

fn fold_bit_bor(args: &[Expr]) -> Option<Expr> {
    let a = get_int(args.first()?)?;
    let b = get_int(args.get(1)?)?;
    Some(Expr::Literal((a | b).to_string()))
}

fn fold_bit_bxor(args: &[Expr]) -> Option<Expr> {
    let a = get_int(args.first()?)?;
    let b = get_int(args.get(1)?)?;
    Some(Expr::Literal((a ^ b).to_string()))
}

fn fold_bit_bnot(args: &[Expr]) -> Option<Expr> {
    let a = get_int(args.first()?)?;
    let u = a.to_u32().unwrap_or(0);
    Some(Expr::Literal((!u as i64).to_string()))
}

fn fold_bit_lshift(args: &[Expr]) -> Option<Expr> {
    let a = get_int(args.first()?)?;
    let b = get_int(args.get(1)?)?;
    let shift = b.to_usize()?;
    if shift <= 64 {
        Some(Expr::Literal((a << shift).to_string()))
    } else {
        Some(Expr::Literal("0".to_string()))
    }
}

fn fold_bit_rshift(args: &[Expr]) -> Option<Expr> {
    let a = get_int(args.first()?)?;
    let b = get_int(args.get(1)?)?;
    let shift = b.to_usize()?;
    if shift <= 64 {
        Some(Expr::Literal((a >> shift).to_string()))
    } else {
        Some(Expr::Literal("0".to_string()))
    }
}

fn fold_string_len(args: &[Expr]) -> Option<Expr> {
    let s = get_str(args.first()?)?;
    Some(Expr::Literal(s.len().to_string()))
}

fn fold_string_lower(args: &[Expr]) -> Option<Expr> {
    let s = get_str(args.first()?)?;
    Some(Expr::Str(s.to_lowercase()))
}

fn fold_string_upper(args: &[Expr]) -> Option<Expr> {
    let s = get_str(args.first()?)?;
    Some(Expr::Str(s.to_uppercase()))
}

fn fold_builtin_int(args: &[Expr]) -> Option<Expr> {
    let arg = args.first()?;
    if let Some(i) = get_int(arg) {
        return Some(Expr::Literal(i.to_string()));
    }
    if let Some(f) = get_float(arg) {
        return Some(Expr::Literal((f as i64).to_string()));
    }
    if let Some(i) = get_str(arg).and_then(|s| BigInt::from_str(s).ok()) {
        return Some(Expr::Literal(i.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_math_folding() {
        let abs_call = Expr::Member {
            object: Box::new(Expr::Variable("math".to_string())),
            field: "abs".to_string(),
        };
        let res = fold_builtin_call(&abs_call, &[Expr::Literal("-42".to_string())]);
        assert_eq!(res, Some(Expr::Literal("42".to_string())));

        let sqrt_call = Expr::Member {
            object: Box::new(Expr::Variable("math".to_string())),
            field: "sqrt".to_string(),
        };
        let res = fold_builtin_call(&sqrt_call, &[Expr::Literal("16".to_string())]);
        assert_eq!(res, Some(Expr::Literal("4".to_string())));
    }

    #[test]
    fn test_bit_and_string_folding() {
        let band_call = Expr::Member {
            object: Box::new(Expr::Variable("bit".to_string())),
            field: "band".to_string(),
        };
        let res = fold_builtin_call(
            &band_call,
            &[
                Expr::Literal("15".to_string()),
                Expr::Literal("7".to_string()),
            ],
        );
        assert_eq!(res, Some(Expr::Literal("7".to_string())));

        let len_call = Expr::Member {
            object: Box::new(Expr::Variable("string".to_string())),
            field: "len".to_string(),
        };
        let res = fold_builtin_call(&len_call, &[Expr::Str("hello world".to_string())]);
        assert_eq!(res, Some(Expr::Literal("11".to_string())));
    }
}
