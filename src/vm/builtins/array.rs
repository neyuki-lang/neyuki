// Array and Object built-in manipulation functions.

use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

pub fn builtin_array_create(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let count = match args.first() {
        Some(Value::Int(i)) => i.to_usize().unwrap_or(0),
        Some(Value::Float(f)) => *f as usize,
        _ => 0,
    };
    const MAX_ARRAY_SIZE: usize = 1_000_000;
    if count > MAX_ARRAY_SIZE {
        return Err(format!(
            "array size exceeds maximum limit ({})",
            MAX_ARRAY_SIZE
        ));
    }
    let init_val = args.get(1).cloned().unwrap_or(Value::Nil);

    let rc = Rc::new(RefCell::new(VmTable::new()));
    vm.gc.register_table(&rc);
    {
        let mut tbl = rc.borrow_mut();
        tbl.array.resize(count, init_val);
    }
    Ok(vec![Value::Table(rc)])
}

pub fn builtin_table_clear(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_val = args
        .first()
        .ok_or_else(|| "table.clear expects a table".to_string())?;
    match tbl_val {
        Value::Table(t) => {
            let mut tbl = t.borrow_mut();
            tbl.array.clear();
            tbl.fields.clear();
            Ok(vec![Value::Nil])
        }
        _ => Err("table.clear expects a table".to_string()),
    }
}
