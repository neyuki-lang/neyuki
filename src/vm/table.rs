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

/// Where a non-array key sorts: numbers first (ascending), then strings,
/// booleans, and everything else by identity.
fn key_rank(key: &Value) -> u8 {
    match key {
        Value::Int(_) | Value::BigInt(_) | Value::Float(_) => 0,
        Value::String(_) => 1,
        Value::Bool(_) => 2,
        _ => 3,
    }
}

fn identity(key: &Value) -> usize {
    match key {
        Value::Table(t) => Rc::as_ptr(t) as *const u8 as usize,
        Value::Closure(c) => Rc::as_ptr(c) as *const u8 as usize,
        Value::Native(d) => *d as *const NativeDef as usize,
        Value::Buffer(b) => Rc::as_ptr(b) as *const u8 as usize,
        _ => 0,
    }
}

fn compare_keys(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    key_rank(a).cmp(&key_rank(b)).then_with(|| match (a, b) {
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ if key_rank(a) == 0 => {
            if crate::vm::ops::eval_lt(a, b).unwrap_or(false) {
                Ordering::Less
            } else if crate::vm::ops::eval_lt(b, a).unwrap_or(false) {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }
        _ => identity(a).cmp(&identity(b)),
    })
}

/// The keys outside the array part, in iteration order: numbers ascending,
/// strings sorted, then booleans and the rest. The array part always comes
/// first, so a table iterates the same way every run.
pub fn sorted_keys(table: &VmTable) -> Vec<Value> {
    let mut keys: Vec<Value> = table
        .fields
        .iter()
        .filter(|(_, v)| !matches!(v, Value::Nil))
        .map(|(k, _)| Value::String(k.clone()))
        .collect();
    if let Some(general) = &table.general {
        keys.extend(general.values().map(|(k, _)| k.clone()));
    }
    keys.sort_by(compare_keys);
    keys
}

/// A raw lookup that ignores metatables.
pub fn raw_get(table: &VmTable, key: &Value) -> Value {
    table.raw_get(key)
}

/// The entry after `key` in iteration order, as `next` returns it.
pub fn next_entry(table: &VmTable, key: &Value) -> Result<Option<(Value, Value)>, String> {
    let array_start = match key {
        Value::Nil => Some(0),
        Value::Int(i) if *i >= 1 && (*i as u64) <= table.array.len() as u64 => Some(*i as usize),
        _ => None,
    };
    if let Some(start) = array_start {
        for (i, v) in table.array.iter().enumerate().skip(start) {
            if !matches!(v, Value::Nil) {
                return Ok(Some((Value::from_usize(i + 1), v.clone())));
            }
        }
        if table.fields.is_empty() && table.general.is_none() {
            return Ok(None);
        }
        // Without a previous hash key the smallest one is next, which a
        // scan finds without sorting.
        let mut best: Option<Value> = None;
        for k in table
            .fields
            .iter()
            .filter(|(_, v)| !matches!(v, Value::Nil))
            .map(|(k, _)| Value::String(k.clone()))
            .chain(
                table
                    .general
                    .iter()
                    .flat_map(|g| g.values().map(|(k, _)| k.clone())),
            )
        {
            if best
                .as_ref()
                .is_none_or(|b| compare_keys(&k, b) == std::cmp::Ordering::Less)
            {
                best = Some(k);
            }
        }
        return Ok(best.map(|k| {
            let v = table.raw_get(&k);
            (k, v)
        }));
    }
    let keys = sorted_keys(table);
    let Some(pos) = keys
        .iter()
        .position(|k| compare_keys(k, key) == std::cmp::Ordering::Equal && k == key)
    else {
        return Err("invalid key to 'next'".to_string());
    };
    Ok(keys.into_iter().nth(pos + 1).map(|k| {
        let v = table.raw_get(&k);
        (k, v)
    }))
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
