//! Argument checks and result builders shared by the native libraries.

use std::cell::RefCell;
use std::rc::Rc;

use num_traits::ToPrimitive;

use crate::vm::value::{Value, VmTable};

/// A number as a float. Integers too large for an `f64` are an error rather
/// than silently becoming infinity.
pub(crate) fn number(value: &Value) -> Result<f64, String> {
    match value {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => Ok(*i as f64),
        Value::BigInt(i) => i
            .to_f64()
            .filter(|f| f.is_finite())
            .ok_or_else(|| "integer is too large for floating-point conversion".to_string()),
        _ => Err("expected a number".to_string()),
    }
}

/// A named float argument.
pub(crate) fn number_arg(args: &[Value], index: usize, name: &str) -> Result<f64, String> {
    number(args.get(index).unwrap_or(&Value::Nil)).map_err(|_| match args.get(index) {
        None | Some(Value::Nil) => format!("{} must be provided", name),
        Some(Value::BigInt(_)) => format!("{} is too large", name),
        Some(other) => format!("{} must be a number, got {}", name, other.type_name()),
    })
}

/// A named string argument.
pub(crate) fn str_arg<'a>(args: &'a [Value], index: usize, name: &str) -> Result<&'a str, String> {
    match args.get(index) {
        Some(Value::String(s)) => Ok(s),
        None | Some(Value::Nil) => Err(format!("{} must be provided", name)),
        Some(other) => Err(format!(
            "{} must be a string, got {}",
            name,
            other.type_name()
        )),
    }
}

/// An optional boolean argument, false when absent.
pub(crate) fn bool_arg(args: &[Value], index: usize, name: &str) -> Result<bool, String> {
    match args.get(index) {
        Some(Value::Bool(b)) => Ok(*b),
        None | Some(Value::Nil) => Ok(false),
        Some(other) => Err(format!(
            "{} must be a boolean, got {}",
            name,
            other.type_name()
        )),
    }
}

/// A named integer argument. Floats with no fractional part are accepted.
pub(crate) fn int_arg(args: &[Value], index: usize, name: &str) -> Result<i64, String> {
    match args.get(index) {
        Some(Value::Int(i)) => Ok(*i),
        Some(Value::BigInt(_)) => Err(format!("{} is out of range", name)),
        Some(Value::Float(f)) if f.is_finite() && f.fract() == 0.0 => {
            if *f >= i64::MIN as f64 && *f < i64::MAX as f64 {
                Ok(*f as i64)
            } else {
                Err(format!("{} is out of range", name))
            }
        }
        None | Some(Value::Nil) => Err(format!("{} must be provided", name)),
        Some(Value::Float(_)) => Err(format!("{} must be an integer", name)),
        Some(other) => Err(format!(
            "{} must be an integer, got {}",
            name,
            other.type_name()
        )),
    }
}

/// An optional integer argument with a default.
pub(crate) fn opt_int_arg(
    args: &[Value],
    index: usize,
    name: &str,
    default: i64,
) -> Result<i64, String> {
    match args.get(index) {
        None | Some(Value::Nil) => Ok(default),
        _ => int_arg(args, index, name),
    }
}

/// A table argument.
pub(crate) fn table_arg<'a>(
    args: &'a [Value],
    index: usize,
    name: &str,
) -> Result<&'a Rc<RefCell<VmTable>>, String> {
    match args.get(index) {
        Some(Value::Table(t)) => Ok(t),
        None | Some(Value::Nil) => Err(format!("{} must be provided", name)),
        Some(other) => Err(format!(
            "{} must be a table, got {}",
            name,
            other.type_name()
        )),
    }
}

/// A sequence of values as a new array table.
pub(crate) fn array(values: Vec<Value>) -> Value {
    let mut table = VmTable::new();
    table.array = values;
    Value::Table(Rc::new(RefCell::new(table)))
}

/// Wraps a frozen library table.
pub(crate) fn frozen(mut table: VmTable) -> Value {
    table.frozen = true;
    Value::Table(Rc::new(RefCell::new(table)))
}
