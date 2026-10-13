//! Iteration and argument helpers: `next`, `pairs`, `ipairs` and `select`.

use crate::vm::machine::VM;
use crate::vm::value::{NativeDef, Value};

/// `next` by address, so a generic `for` over `pairs(t)` can recognise it
/// and iterate the table directly.
pub static NEXT: &NativeDef = &NativeDef {
    name: "next",
    func: builtin_next,
};

/// `next(t, key?)`: the entry after `key` (the first one when `key` is nil),
/// or nil at the end. The order is the one `for k, v in t` uses.
pub fn builtin_next(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let Some(Value::Table(t)) = args.first() else {
        return Err("next expects a table".to_string());
    };
    let key = args.get(1).cloned().unwrap_or(Value::Nil);
    match crate::vm::table::next_entry(&t.borrow(), &key)? {
        Some((k, v)) => Ok(vec![k, v]),
        None => Ok(vec![Value::Nil]),
    }
}

/// `pairs(t)`: `next, t, nil`, or whatever the `__pairs` metamethod returns.
pub fn builtin_pairs(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let Some(table @ Value::Table(t)) = args.first() else {
        return Err("pairs expects a table".to_string());
    };
    let handler = t.borrow().metamethod("__pairs");
    if let Some(handler) = handler {
        let mut results = vm.call_function(handler, std::slice::from_ref(table))?;
        results.resize(3, Value::Nil);
        return Ok(results);
    }
    Ok(vec![Value::Native(NEXT), table.clone(), Value::Nil])
}

fn ipairs_step(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let table = args.first().cloned().unwrap_or(Value::Nil);
    let index = match args.get(1) {
        Some(Value::Int(i)) => i + 1,
        _ => 1,
    };
    let value = vm.table_get(&table, &Value::Int(index))?;
    if matches!(value, Value::Nil) {
        Ok(vec![Value::Nil])
    } else {
        Ok(vec![Value::Int(index), value])
    }
}

/// `ipairs(t)`: `t[1]`, `t[2]`, ... up to the first nil.
pub fn builtin_ipairs(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let table = args.first().cloned().unwrap_or(Value::Nil);
    if !matches!(table, Value::Table(_) | Value::String(_) | Value::Buffer(_)) {
        return Err(format!("ipairs expects a table, got {}", table.type_name()));
    }
    Ok(vec![
        crate::native!("ipairs iterator", ipairs_step),
        table,
        Value::Int(0),
    ])
}

/// `select(n, ...)` returns the arguments from the `n`th on (counting back
/// from the end when negative); `select("#", ...)` counts them.
pub fn builtin_select(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first() {
        Some(Value::String(s)) if &**s == "#" => Ok(vec![Value::from_usize(rest.len())]),
        Some(Value::Int(n)) if *n > 0 => Ok(rest.get(*n as usize - 1..).unwrap_or(&[]).to_vec()),
        Some(Value::Int(n)) if *n < 0 && n.unsigned_abs() as usize <= rest.len() => {
            Ok(rest[rest.len() - n.unsigned_abs() as usize..].to_vec())
        }
        _ => Err("select expects '#' or an index within range".to_string()),
    }
}

/// `warn(...)` writes its arguments to stderr as one warning line.
pub fn builtin_warn(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut line = String::from("warning: ");
    for arg in args {
        line.push_str(&crate::vm::builtins::global::tostring(vm, arg)?);
    }
    eprintln!("{}", line);
    Ok(Vec::new())
}

fn globals_index(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    match args.get(1) {
        Some(Value::String(name)) => {
            Ok(vec![vm.globals.get(&**name).cloned().unwrap_or(Value::Nil)])
        }
        _ => Ok(vec![Value::Nil]),
    }
}

fn globals_newindex(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    match args.get(1) {
        Some(Value::String(name)) => {
            let value = args.get(2).cloned().unwrap_or(Value::Nil);
            vm.set_global(name, value);
            Ok(Vec::new())
        }
        _ => Err("global names must be strings".to_string()),
    }
}

/// `_G`, a view of the global variables: reading and writing its fields
/// reads and writes globals.
pub fn globals_table() -> Value {
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut meta = crate::vm::value::VmTable::new();
    meta.set_str("__index", crate::native!("_G.__index", globals_index));
    meta.set_str(
        "__newindex",
        crate::native!("_G.__newindex", globals_newindex),
    );
    let mut table = crate::vm::value::VmTable::new();
    table.metatable = Some(Rc::new(RefCell::new(meta)));
    Value::Table(Rc::new(RefCell::new(table)))
}
