// Advanced table key representation and hashing for general Lua/Neyuki tables.
// Supports arbitrary non-nil, non-NaN values as keys: booleans, floats, integers,
// strings, tables, closures, natives, and buffers.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::vm::buffer::VmBuffer;
use crate::vm::value::{NativeDef, StrRef, Value, VmClosure, VmTable};

#[derive(Clone, Debug)]
pub enum TableKey {
    Bool(bool),
    Int(i64),
    Float(u64), // normalized f64 bits
    String(StrRef),
    Table(*const RefCell<VmTable>),
    Closure(*const VmClosure),
    Native(*const NativeDef),
    Buffer(*const RefCell<VmBuffer>),
}

impl PartialEq for TableKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (TableKey::Bool(a), TableKey::Bool(b)) => a == b,
            (TableKey::Int(a), TableKey::Int(b)) => a == b,
            (TableKey::Float(a), TableKey::Float(b)) => a == b,
            (TableKey::String(a), TableKey::String(b)) => a == b,
            (TableKey::Table(a), TableKey::Table(b)) => std::ptr::eq(*a, *b),
            (TableKey::Closure(a), TableKey::Closure(b)) => std::ptr::eq(*a, *b),
            (TableKey::Native(a), TableKey::Native(b)) => std::ptr::eq(*a, *b),
            (TableKey::Buffer(a), TableKey::Buffer(b)) => std::ptr::eq(*a, *b),
            _ => false,
        }
    }
}

impl Eq for TableKey {}

impl Hash for TableKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            TableKey::Bool(b) => b.hash(state),
            TableKey::Int(i) => i.hash(state),
            TableKey::Float(bits) => bits.hash(state),
            TableKey::String(s) => s.hash(state),
            TableKey::Table(p) => p.hash(state),
            TableKey::Closure(p) => p.hash(state),
            TableKey::Native(p) => p.hash(state),
            TableKey::Buffer(p) => p.hash(state),
        }
    }
}

impl TableKey {
    pub fn from_value(val: &Value) -> Result<Self, &'static str> {
        match val {
            Value::Nil => Err("table index is nil"),
            Value::Bool(b) => Ok(TableKey::Bool(*b)),
            Value::Int(i) => Ok(TableKey::Int(*i)),
            Value::Float(f) => {
                if f.is_nan() {
                    return Err("table index is NaN");
                }
                // If float is an exact integer, represent as Int to unify lookup
                if f.fract() == 0.0 && *f >= (i64::MIN as f64) && *f <= (i64::MAX as f64) {
                    Ok(TableKey::Int(*f as i64))
                } else {
                    let bits = if *f == 0.0 { 0u64 } else { f.to_bits() };
                    Ok(TableKey::Float(bits))
                }
            }
            Value::BigInt(b) => {
                if let Some(i) = num_traits::ToPrimitive::to_i64(&**b) {
                    Ok(TableKey::Int(i))
                } else {
                    Err("table index too large")
                }
            }
            Value::String(s) => Ok(TableKey::String(s.clone())),
            Value::Table(t) => Ok(TableKey::Table(Rc::as_ptr(t))),
            Value::Closure(c) => Ok(TableKey::Closure(Rc::as_ptr(c))),
            Value::Native(def) => Ok(TableKey::Native(*def as *const _)),
            Value::Buffer(b) => Ok(TableKey::Buffer(Rc::as_ptr(b))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_key_conversions() {
        assert_eq!(
            TableKey::from_value(&Value::Bool(true)),
            Ok(TableKey::Bool(true))
        );
        assert_eq!(
            TableKey::from_value(&Value::Int(-42)),
            Ok(TableKey::Int(-42))
        );
        assert_eq!(
            TableKey::from_value(&Value::Float(10.0)),
            Ok(TableKey::Int(10))
        );
        assert!(TableKey::from_value(&Value::Nil).is_err());
        assert!(TableKey::from_value(&Value::Float(f64::NAN)).is_err());
    }
}
