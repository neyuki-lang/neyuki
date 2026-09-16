// Compile-time constant folding and expression optimization.

use num_bigint::{BigInt, Sign};
use num_integer::Integer as _;
use num_traits::{FromPrimitive, Signed, ToPrimitive, Zero};
use std::str::FromStr;

use crate::parser::{Expr, InterpPart, Stmt, TableEntry};

#[derive(Clone, Debug, PartialEq)]
enum FoldVal {
    Nil,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    Str(String),
}

impl FoldVal {
    fn is_truthy(&self) -> bool {
        match self {
            FoldVal::Nil => false,
            FoldVal::Bool(b) => *b,
            _ => true,
        }
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_expr(self) -> Expr {
        match self {
            FoldVal::Nil => Expr::Literal("nil".to_string()),
            FoldVal::Bool(b) => Expr::Literal(b.to_string()),
            FoldVal::Int(i) => Expr::Literal(i.to_string()),
            FoldVal::Float(f) => {
                let s = f.to_string();
                if !s.contains('.') && !s.contains('e') && !s.contains('E') {
                    Expr::Literal(format!("{}.0", s))
                } else {
                    Expr::Literal(s)
                }
            }
            FoldVal::Str(s) => Expr::Str(s),
        }
    }

    fn from_expr(expr: &Expr) -> Option<Self> {
        match expr {
            Expr::Literal(s) => match s.as_str() {
                "nil" => Some(FoldVal::Nil),
                "true" => Some(FoldVal::Bool(true)),
                "false" => Some(FoldVal::Bool(false)),
                _ => {
                    if let Ok(i) = s.parse::<i64>() {
                        Some(FoldVal::Int(BigInt::from(i)))
                    } else if let Ok(bi) = BigInt::from_str(s) {
                        Some(FoldVal::Int(bi))
                    } else if let Ok(f) = s.parse::<f64>() {
                        Some(FoldVal::Float(f))
                    } else {
                        None
                    }
                }
            },
            Expr::Str(s) => Some(FoldVal::Str(s.clone())),
            _ => None,
        }
    }

    fn to_f64(&self) -> Option<f64> {
        match self {
            FoldVal::Float(f) => Some(*f),
            FoldVal::Int(i) => i.to_f64(),
            _ => None,
        }
    }

    fn to_bigint(&self) -> Option<BigInt> {
        match self {
            FoldVal::Int(i) => Some(i.clone()),
            FoldVal::Float(f) => BigInt::from_f64(f.trunc()),
            _ => None,
        }
    }
}

// Fold constant unary operations
fn fold_unary_op(op: &str, val: FoldVal) -> Option<FoldVal> {
    match op {
        "-" => match val {
            FoldVal::Int(i) => Some(FoldVal::Int(-i)),
            FoldVal::Float(f) => Some(FoldVal::Float(-f)),
            _ => None,
        },
        "not" => Some(FoldVal::Bool(!val.is_truthy())),
        "#" => match val {
            FoldVal::Str(s) => Some(FoldVal::Int(BigInt::from(s.len()))),
            _ => None,
        },
        "~" => match val {
            FoldVal::Int(i) => Some(FoldVal::Int(!i)),
            FoldVal::Float(f) => {
                let bi = BigInt::from_f64(f.trunc())?;
                Some(FoldVal::Int(!bi))
            }
            _ => None,
        },
        _ => None,
    }
}

// Fold constant binary operations
fn fold_binary_op(op: &str, left: FoldVal, right: FoldVal) -> Option<FoldVal> {
    match op {
        "+" => match (left, right) {
            (FoldVal::Int(a), FoldVal::Int(b)) => Some(FoldVal::Int(a + b)),
            (a, b) => {
                let fa = a.to_f64()?;
                let fb = b.to_f64()?;
                Some(FoldVal::Float(fa + fb))
            }
        },
        "-" => match (left, right) {
            (FoldVal::Int(a), FoldVal::Int(b)) => Some(FoldVal::Int(a - b)),
            (a, b) => {
                let fa = a.to_f64()?;
                let fb = b.to_f64()?;
                Some(FoldVal::Float(fa - fb))
            }
        },
        "*" => match (left, right) {
            (FoldVal::Int(a), FoldVal::Int(b)) => Some(FoldVal::Int(a * b)),
            (a, b) => {
                let fa = a.to_f64()?;
                let fb = b.to_f64()?;
                Some(FoldVal::Float(fa * fb))
            }
        },
        "/" => {
            let fa = left.to_f64()?;
            let fb = right.to_f64()?;
            if fb == 0.0 {
                return None;
            }
            Some(FoldVal::Float(fa / fb))
        }
        "//" => match (left, right) {
            (FoldVal::Int(a), FoldVal::Int(b)) => {
                if b.is_zero() {
                    return None;
                }
                Some(FoldVal::Int(a.div_floor(&b)))
            }
            (a, b) => {
                let fa = a.to_f64()?;
                let fb = b.to_f64()?;
                if fb == 0.0 {
                    return None;
                }
                Some(FoldVal::Float((fa / fb).floor()))
            }
        },
        "%" => match (left, right) {
            (FoldVal::Int(a), FoldVal::Int(b)) => {
                if b.is_zero() {
                    return None;
                }
                Some(FoldVal::Int(a.mod_floor(&b)))
            }
            (a, b) => {
                let fa = a.to_f64()?;
                let fb = b.to_f64()?;
                if fb == 0.0 {
                    return None;
                }
                Some(FoldVal::Float(fa.rem_euclid(fb)))
            }
        },
        "^" => {
            let fa = left.to_f64()?;
            let fb = right.to_f64()?;
            Some(FoldVal::Float(fa.powf(fb)))
        }
        "&" => {
            let ia = left.to_bigint()?;
            let ib = right.to_bigint()?;
            Some(FoldVal::Int(ia & ib))
        }
        "|" => {
            let ia = left.to_bigint()?;
            let ib = right.to_bigint()?;
            Some(FoldVal::Int(ia | ib))
        }
        "~" => {
            let ia = left.to_bigint()?;
            let ib = right.to_bigint()?;
            Some(FoldVal::Int(ia ^ ib))
        }
        "<<" => {
            let ia = left.to_bigint()?;
            let ib = right.to_bigint()?;
            if ib.sign() == Sign::Minus {
                let shift = (-ib).to_usize()?;
                Some(FoldVal::Int(ia >> shift))
            } else {
                let shift = ib.to_usize()?;
                Some(FoldVal::Int(ia << shift))
            }
        }
        ">>" => {
            let ia = left.to_bigint()?;
            let ib = right.to_bigint()?;
            if ib.sign() == Sign::Minus {
                let shift = (-ib).to_usize()?;
                Some(FoldVal::Int(ia << shift))
            } else {
                let shift = ib.to_usize()?;
                Some(FoldVal::Int(ia >> shift))
            }
        }
        ".." => {
            let sa = match left {
                FoldVal::Str(s) => s,
                FoldVal::Int(i) => i.to_string(),
                FoldVal::Float(f) => f.to_string(),
                _ => return None,
            };
            let sb = match right {
                FoldVal::Str(s) => s,
                FoldVal::Int(i) => i.to_string(),
                FoldVal::Float(f) => f.to_string(),
                _ => return None,
            };
            Some(FoldVal::Str(format!("{}{}", sa, sb)))
        }
        "==" => Some(FoldVal::Bool(left == right)),
        "!=" => Some(FoldVal::Bool(left != right)),
        "<" => match (&left, &right) {
            (FoldVal::Int(a), FoldVal::Int(b)) => Some(FoldVal::Bool(a < b)),
            (FoldVal::Str(a), FoldVal::Str(b)) => Some(FoldVal::Bool(a < b)),
            _ => {
                let fa = left.to_f64()?;
                let fb = right.to_f64()?;
                Some(FoldVal::Bool(fa < fb))
            }
        },
        "<=" => match (&left, &right) {
            (FoldVal::Int(a), FoldVal::Int(b)) => Some(FoldVal::Bool(a <= b)),
            (FoldVal::Str(a), FoldVal::Str(b)) => Some(FoldVal::Bool(a <= b)),
            _ => {
                let fa = left.to_f64()?;
                let fb = right.to_f64()?;
                Some(FoldVal::Bool(fa <= fb))
            }
        },
        ">" => match (&left, &right) {
            (FoldVal::Int(a), FoldVal::Int(b)) => Some(FoldVal::Bool(a > b)),
            (FoldVal::Str(a), FoldVal::Str(b)) => Some(FoldVal::Bool(a > b)),
            _ => {
                let fa = left.to_f64()?;
                let fb = right.to_f64()?;
                Some(FoldVal::Bool(fa > fb))
            }
        },
        ">=" => match (&left, &right) {
            (FoldVal::Int(a), FoldVal::Int(b)) => Some(FoldVal::Bool(a >= b)),
            (FoldVal::Str(a), FoldVal::Str(b)) => Some(FoldVal::Bool(a >= b)),
            _ => {
                let fa = left.to_f64()?;
                let fb = right.to_f64()?;
                Some(FoldVal::Bool(fa >= fb))
            }
        },
        _ => None,
    }
}

// Fold constant builtin calls like math.abs, bit.band, string.len, etc.
fn fold_builtin_call(callee: &Expr, args: &[FoldVal]) -> Option<FoldVal> {
    match callee {
        Expr::Variable(name) => match name.as_str() {
            "type" => {
                let arg = args.first()?;
                let t_str = match arg {
                    FoldVal::Nil => "nil",
                    FoldVal::Bool(_) => "boolean",
                    FoldVal::Int(_) | FoldVal::Float(_) => "number",
                    FoldVal::Str(_) => "string",
                };
                Some(FoldVal::Str(t_str.to_string()))
            }
            "typeof" => {
                let arg = args.first()?;
                let t_str = match arg {
                    FoldVal::Nil => "nil",
                    FoldVal::Bool(_) => "boolean",
                    FoldVal::Int(_) => "bigint",
                    FoldVal::Float(_) => "float",
                    FoldVal::Str(_) => "string",
                };
                Some(FoldVal::Str(t_str.to_string()))
            }
            "tostring" => {
                let arg = args.first()?;
                let s = match arg {
                    FoldVal::Nil => "nil".to_string(),
                    FoldVal::Bool(b) => b.to_string(),
                    FoldVal::Int(i) => i.to_string(),
                    FoldVal::Float(f) => f.to_string(),
                    FoldVal::Str(s) => s.clone(),
                };
                Some(FoldVal::Str(s))
            }
            _ => None,
        },
        Expr::Member { object, field } => {
            if let Expr::Variable(mod_name) = object.as_ref() {
                match (mod_name.as_str(), field.as_str()) {
                    ("math", "abs") => {
                        let arg = args.first()?;
                        match arg {
                            FoldVal::Int(i) => Some(FoldVal::Int(i.abs())),
                            FoldVal::Float(f) => Some(FoldVal::Float(f.abs())),
                            _ => None,
                        }
                    }
                    ("math", "floor") => {
                        let f = args.first()?.to_f64()?;
                        Some(FoldVal::Float(f.floor()))
                    }
                    ("math", "ceil") => {
                        let f = args.first()?.to_f64()?;
                        Some(FoldVal::Float(f.ceil()))
                    }
                    ("math", "sqrt") => {
                        let f = args.first()?.to_f64()?;
                        if f < 0.0 {
                            return None;
                        }
                        Some(FoldVal::Float(f.sqrt()))
                    }
                    ("math", "min") => {
                        if args.is_empty() {
                            return None;
                        }
                        let mut min_val = args[0].to_f64()?;
                        for a in &args[1..] {
                            let v = a.to_f64()?;
                            if v < min_val {
                                min_val = v;
                            }
                        }
                        Some(FoldVal::Float(min_val))
                    }
                    ("math", "max") => {
                        if args.is_empty() {
                            return None;
                        }
                        let mut max_val = args[0].to_f64()?;
                        for a in &args[1..] {
                            let v = a.to_f64()?;
                            if v > max_val {
                                max_val = v;
                            }
                        }
                        Some(FoldVal::Float(max_val))
                    }
                    ("bit" | "bit32", "band") => {
                        let mut res = args.first()?.to_bigint()?;
                        for a in &args[1..] {
                            res &= a.to_bigint()?;
                        }
                        Some(FoldVal::Int(res))
                    }
                    ("bit" | "bit32", "bor") => {
                        let mut res = args.first()?.to_bigint()?;
                        for a in &args[1..] {
                            res |= a.to_bigint()?;
                        }
                        Some(FoldVal::Int(res))
                    }
                    ("bit" | "bit32", "bxor") => {
                        let mut res = args.first()?.to_bigint()?;
                        for a in &args[1..] {
                            res ^= a.to_bigint()?;
                        }
                        Some(FoldVal::Int(res))
                    }
                    ("bit" | "bit32", "bnot") => {
                        let val = args.first()?.to_bigint()?;
                        Some(FoldVal::Int(!val))
                    }
                    ("bit" | "bit32", "lshift") => {
                        let val = args.first()?.to_bigint()?;
                        let shift = args.get(1)?.to_bigint()?.to_usize()?;
                        Some(FoldVal::Int(val << shift))
                    }
                    ("bit" | "bit32", "rshift") => {
                        let val = args.first()?.to_bigint()?;
                        let shift = args.get(1)?.to_bigint()?.to_usize()?;
                        Some(FoldVal::Int(val >> shift))
                    }
                    ("string", "len") => {
                        if let Some(FoldVal::Str(s)) = args.first() {
                            Some(FoldVal::Int(BigInt::from(s.len())))
                        } else {
                            None
                        }
                    }
                    ("string", "lower") => {
                        if let Some(FoldVal::Str(s)) = args.first() {
                            Some(FoldVal::Str(s.to_lowercase()))
                        } else {
                            None
                        }
                    }
                    ("string", "upper") => {
                        if let Some(FoldVal::Str(s)) = args.first() {
                            Some(FoldVal::Str(s.to_uppercase()))
                        } else {
                            None
                        }
                    }
                    ("string", "reverse") => {
                        if let Some(FoldVal::Str(s)) = args.first() {
                            Some(FoldVal::Str(s.chars().rev().collect()))
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

// Recursively optimize an AST expression by folding constants
pub fn fold_expr(expr: Expr) -> Expr {
    match expr {
        Expr::Literal(_) | Expr::Str(_) | Expr::Variable(_) | Expr::Vararg => expr,
        Expr::Interp(parts) => {
            let folded_parts: Vec<InterpPart> = parts
                .into_iter()
                .map(|p| match p {
                    InterpPart::Literal(s) => InterpPart::Literal(s),
                    InterpPart::Expr(e) => InterpPart::Expr(fold_expr(e)),
                })
                .collect();
            let all_literals = folded_parts
                .iter()
                .all(|p| matches!(p, InterpPart::Literal(_)));
            if all_literals {
                let mut combined = String::new();
                for p in folded_parts {
                    if let InterpPart::Literal(s) = p {
                        combined.push_str(&s);
                    }
                }
                Expr::Str(combined)
            } else {
                Expr::Interp(folded_parts)
            }
        }
        Expr::Unary { op, expr: inner } => {
            let folded_inner = fold_expr(*inner);
            if let Some(c) = FoldVal::from_expr(&folded_inner)
                && let Some(res) = fold_unary_op(&op, c) {
                    return res.to_expr();
                }
            Expr::Unary {
                op,
                expr: Box::new(folded_inner),
            }
        }
        Expr::Binary { left, op, right } => {
            let folded_left = fold_expr(*left);

            // Short-circuit folding for 'and', 'or', and '??'
            if op == "and" {
                if let Some(c) = FoldVal::from_expr(&folded_left) {
                    if !c.is_truthy() {
                        return folded_left;
                    }
                    return fold_expr(*right);
                }
            } else if op == "or" {
                if let Some(c) = FoldVal::from_expr(&folded_left) {
                    if c.is_truthy() {
                        return folded_left;
                    }
                    return fold_expr(*right);
                }
            } else if op == "??"
                && let Some(c) = FoldVal::from_expr(&folded_left) {
                    if c != FoldVal::Nil {
                        return folded_left;
                    }
                    return fold_expr(*right);
                }

            let folded_right = fold_expr(*right);

            if let (Some(cl), Some(cr)) = (
                FoldVal::from_expr(&folded_left),
                FoldVal::from_expr(&folded_right),
            )
                && let Some(res) = fold_binary_op(&op, cl, cr) {
                    return res.to_expr();
                }

            Expr::Binary {
                left: Box::new(folded_left),
                op,
                right: Box::new(folded_right),
            }
        }
        Expr::Member { object, field } => Expr::Member {
            object: Box::new(fold_expr(*object)),
            field,
        },
        Expr::Index { object, index } => Expr::Index {
            object: Box::new(fold_expr(*object)),
            index: Box::new(fold_expr(*index)),
        },
        Expr::Call { callee, args } => {
            let folded_callee = fold_expr(*callee);
            let folded_args: Vec<Expr> = args.into_iter().map(fold_expr).collect();

            // Check if all arguments are constant foldables
            let mut const_args = Vec::with_capacity(folded_args.len());
            let mut all_const = true;
            for a in &folded_args {
                if let Some(c) = FoldVal::from_expr(a) {
                    const_args.push(c);
                } else {
                    all_const = false;
                    break;
                }
            }

            if all_const
                && let Some(res) = fold_builtin_call(&folded_callee, &const_args) {
                    return res.to_expr();
                }

            Expr::Call {
                callee: Box::new(folded_callee),
                args: folded_args,
            }
        }
        Expr::Function { params, body } => Expr::Function {
            params,
            body: fold_program(body),
        },
        Expr::MethodCall {
            object,
            method,
            args,
        } => Expr::MethodCall {
            object: Box::new(fold_expr(*object)),
            method,
            args: args.into_iter().map(fold_expr).collect(),
        },
        Expr::Table(entries) => Expr::Table(
            entries
                .into_iter()
                .map(|e| TableEntry {
                    key: e.key,
                    value: fold_expr(e.value),
                })
                .collect(),
        ),
    }
}

// Optimize statement by folding constant expressions within it
pub fn fold_stmt(stmt: Stmt) -> Option<Stmt> {
    match stmt {
        Stmt::Local {
            name,
            is_const,
            type_name,
            initializer,
        } => Some(Stmt::Local {
            name,
            is_const,
            type_name,
            initializer: initializer.map(fold_expr),
        }),
        Stmt::LocalMany {
            names,
            is_const,
            initializers,
        } => Some(Stmt::LocalMany {
            names,
            is_const,
            initializers: initializers.into_iter().map(fold_expr).collect(),
        }),
        Stmt::Assign {
            target,
            value,
            is_const,
        } => Some(Stmt::Assign {
            target: fold_expr(target),
            value: fold_expr(value),
            is_const,
        }),
        Stmt::Increment { target, amount } => Some(Stmt::Increment {
            target: fold_expr(target),
            amount,
        }),
        Stmt::Function {
            name,
            is_const,
            params,
            return_type,
            body,
        } => Some(Stmt::Function {
            name,
            is_const,
            params,
            return_type,
            body: fold_program(body),
        }),
        Stmt::If {
            condition,
            then_branch,
            else_if_branches,
            else_branch,
        } => {
            let folded_cond = fold_expr(condition);

            // If static truthy condition, dead-branch elimination can happen
            if let Some(c) = FoldVal::from_expr(&folded_cond)
                && c.is_truthy() {
                    let folded_then = fold_program(then_branch);
                    return Some(Stmt::If {
                        condition: folded_cond,
                        then_branch: folded_then,
                        else_if_branches: Vec::new(),
                        else_branch: None,
                    });
                }

            Some(Stmt::If {
                condition: folded_cond,
                then_branch: fold_program(then_branch),
                else_if_branches: else_if_branches
                    .into_iter()
                    .map(|(cond, branch)| (fold_expr(cond), fold_program(branch)))
                    .collect(),
                else_branch: else_branch.map(fold_program),
            })
        }
        Stmt::While { condition, body } => {
            let folded_cond = fold_expr(condition);
            if let Some(FoldVal::Bool(false)) | Some(FoldVal::Nil) = FoldVal::from_expr(&folded_cond) {
                // Eliminate while false loop entirely
                return None;
            }
            Some(Stmt::While {
                condition: folded_cond,
                body: fold_program(body),
            })
        }
        Stmt::NumericFor {
            var,
            start,
            end,
            step,
            body,
        } => Some(Stmt::NumericFor {
            var,
            start: fold_expr(start),
            end: fold_expr(end),
            step: step.map(fold_expr),
            body: fold_program(body),
        }),
        Stmt::For { vars, source, body } => Some(Stmt::For {
            vars,
            source: fold_expr(source),
            body: fold_program(body),
        }),
        Stmt::Repeat { body, condition } => Some(Stmt::Repeat {
            body: fold_program(body),
            condition: fold_expr(condition),
        }),
        Stmt::Return(exprs) => Some(Stmt::Return(exprs.into_iter().map(fold_expr).collect())),
        Stmt::Expr(expr) => Some(Stmt::Expr(fold_expr(expr))),
        Stmt::Break => Some(Stmt::Break),
        Stmt::Continue => Some(Stmt::Continue),
        Stmt::AssignMany { targets, values } => Some(Stmt::AssignMany {
            targets: targets.into_iter().map(fold_expr).collect(),
            values: values.into_iter().map(fold_expr).collect(),
        }),
    }
}

// Optimize entire program statements
pub fn fold_program(stmts: Vec<Stmt>) -> Vec<Stmt> {
    stmts.into_iter().filter_map(fold_stmt).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    #[test]
    fn test_fold_arithmetic() {
        let mut parser = Parser::new("local x = 10 + 20 * 3");
        let stmts = parser.parse_program();
        let optimized = fold_program(stmts);
        match &optimized[0] {
            Stmt::Local { initializer: Some(Expr::Literal(val)), .. } => {
                assert_eq!(val, "70");
            }
            _ => panic!("failed to fold arithmetic"),
        }
    }

    #[test]
    fn test_fold_bitwise() {
        let mut parser = Parser::new("local x = (1 << 4) | (16 >> 2)");
        let stmts = parser.parse_program();
        let optimized = fold_program(stmts);
        // (1 << 4) = 16, (16 >> 2) = 4, 16 | 4 = 20
        match &optimized[0] {
            Stmt::Local { initializer: Some(Expr::Literal(val)), .. } => {
                assert_eq!(val, "20");
            }
            _ => panic!("failed to fold bitwise"),
        }
    }

    #[test]
    fn test_fold_builtin_math_and_bit() {
        let mut parser = Parser::new("local a = math.abs(-42)\nlocal b = bit.band(255, 15)");
        let stmts = parser.parse_program();
        let optimized = fold_program(stmts);
        match &optimized[0] {
            Stmt::Local { initializer: Some(Expr::Literal(val)), .. } => {
                assert_eq!(val, "42");
            }
            _ => panic!("failed to fold math.abs"),
        }
        match &optimized[1] {
            Stmt::Local { initializer: Some(Expr::Literal(val)), .. } => {
                assert_eq!(val, "15");
            }
            _ => panic!("failed to fold bit.band"),
        }
    }
}
