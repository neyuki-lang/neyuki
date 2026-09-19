// Metatable and raw table built-in functions: setmetatable, getmetatable, rawget, rawset, rawequal, rawlen.

use std::rc::Rc;

use crate::vm::machine::VM;
use crate::vm::value::Value;

pub fn builtin_setmetatable(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_val = args
        .first()
        .ok_or_else(|| "setmetatable expects table as first argument".to_string())?;
    let mt_val = args
        .get(1)
        .ok_or_else(|| "setmetatable expects 2 arguments".to_string())?;
    match tbl_val {
        Value::Table(t) => {
            if let Some(existing_mt) = &t.borrow().metatable
                && let Some(metaname) = existing_mt.borrow().fields.get("__metatable")
                && !matches!(metaname, Value::Nil)
            {
                return Err("cannot change a protected metatable".to_string());
            }
            match mt_val {
                Value::Nil => {
                    t.borrow_mut().metatable = None;
                    Ok(vec![tbl_val.clone()])
                }
                Value::Table(mt) => {
                    t.borrow_mut().metatable = Some(mt.clone());
                    Ok(vec![tbl_val.clone()])
                }
                _ => Err("metatable must be a table or nil".to_string()),
            }
        }
        _ => Err("setmetatable expects a table as first argument".to_string()),
    }
}

pub fn builtin_getmetatable(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_val = args
        .first()
        .ok_or_else(|| "getmetatable expects 1 argument".to_string())?;
    match tbl_val {
        Value::Table(t) => {
            if let Some(mt) = &t.borrow().metatable {
                if let Some(metaname) = mt.borrow().fields.get("__metatable")
                    && !matches!(metaname, Value::Nil)
                {
                    return Ok(vec![metaname.clone()]);
                }
                Ok(vec![Value::Table(mt.clone())])
            } else {
                Ok(vec![Value::Nil])
            }
        }
        _ => Ok(vec![Value::Nil]),
    }
}

pub fn builtin_rawget(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_val = args
        .first()
        .ok_or_else(|| "rawget expects table as first argument".to_string())?;
    let key = args
        .get(1)
        .ok_or_else(|| "rawget expects 2 arguments".to_string())?;
    match tbl_val {
        Value::Table(t) => {
            let tbl = t.borrow();
            match key {
                Value::String(k) => Ok(vec![tbl.fields.get(&**k).cloned().unwrap_or(Value::Nil)]),
                Value::Int(idx) if *idx > 0 => {
                    let i = *idx as usize;
                    Ok(vec![tbl.array.get(i - 1).cloned().unwrap_or(Value::Nil)])
                }
                Value::BigInt(_) => Err("table index too large".to_string()),
                _ => Ok(vec![Value::Nil]),
            }
        }
        _ => Err("rawget expects a table".to_string()),
    }
}

pub fn builtin_rawset(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_val = args
        .first()
        .ok_or_else(|| "rawset expects table as first argument".to_string())?;
    let val = args.get(2).cloned().unwrap_or(Value::Nil);
    vm.gc.write_barrier(tbl_val, &val);
    let tbl_val = args
        .first()
        .ok_or_else(|| "rawset expects table as first argument".to_string())?;
    let key = args
        .get(1)
        .ok_or_else(|| "rawset expects 3 arguments".to_string())?;
    let val = args.get(2).cloned().unwrap_or(Value::Nil);
    match tbl_val {
        Value::Table(t) => {
            let mut tbl = t.borrow_mut();
            if tbl.frozen {
                return Err("attempted to mutate a frozen table".to_string());
            }
            match key {
                Value::String(k) => {
                    tbl.fields.insert(k.clone(), val);
                }
                Value::BigInt(_) => return Err("table index too large".to_string()),
                Value::Int(idx) if *idx > 0 => {
                    let i = *idx as usize;
                    if i - 1 < tbl.array.len() {
                        tbl.array[i - 1] = val;
                    } else if i - 1 == tbl.array.len() {
                        tbl.array.push(val);
                    } else {
                        tbl.array.resize(i - 1, Value::Nil);
                        tbl.array.push(val);
                    }
                }
                _ => return Err("invalid table key".to_string()),
            }
            Ok(vec![tbl_val.clone()])
        }
        _ => Err("rawset expects a table".to_string()),
    }
}

pub fn builtin_rawequal(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let a = args.first().unwrap_or(&Value::Nil);
    let b = args.get(1).unwrap_or(&Value::Nil);
    match (a, b) {
        (Value::Nil, Value::Nil) => Ok(vec![Value::Bool(true)]),
        (Value::Bool(x), Value::Bool(y)) => Ok(vec![Value::Bool(x == y)]),
        (Value::Int(x), Value::Int(y)) => Ok(vec![Value::Bool(x == y)]),
        (Value::Float(x), Value::Float(y)) => Ok(vec![Value::Bool(x == y)]),
        (Value::String(x), Value::String(y)) => Ok(vec![Value::Bool(x == y)]),
        (Value::Table(x), Value::Table(y)) => Ok(vec![Value::Bool(Rc::ptr_eq(x, y))]),
        (Value::Buffer(x), Value::Buffer(y)) => Ok(vec![Value::Bool(Rc::ptr_eq(x, y))]),
        (Value::Closure(x), Value::Closure(y)) => Ok(vec![Value::Bool(Rc::ptr_eq(x, y))]),
        (Value::Native(na), Value::Native(nb)) => Ok(vec![Value::Bool(na.name == nb.name)]),
        _ => Ok(vec![Value::Bool(false)]),
    }
}

pub fn builtin_rawlen(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let val = args
        .first()
        .ok_or_else(|| "rawlen expects 1 argument".to_string())?;
    match val {
        Value::Table(t) => Ok(vec![Value::from_usize(t.borrow().array.len())]),
        Value::String(s) => Ok(vec![Value::from_usize(s.len())]),
        Value::Buffer(b) => Ok(vec![Value::from_usize(b.borrow().len())]),
        _ => Err("rawlen expects table, string, or buffer".to_string()),
    }
}
