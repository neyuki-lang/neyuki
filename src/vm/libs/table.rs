// Standard table manipulation library for Neyuki VM.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

fn get_table(val: &Value) -> Result<&Rc<RefCell<VmTable>>, String> {
    match val {
        Value::Table(t) => Ok(t),
        _ => Err("table operation expects a table".to_string()),
    }
}

fn to_isize(val: &Value, name: &str) -> Result<isize, String> {
    match val {
        Value::Int(i) => i
            .to_isize()
            .ok_or_else(|| format!("{} is out of bounds", name)),
        Value::Float(f) => Ok(*f as isize),
        _ => Err(format!("{} expects an integer", name)),
    }
}

fn table_insert(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "table.insert expects at least 2 arguments".to_string())?,
    )?;
    let mut tbl = tbl_rc.borrow_mut();
    if tbl.frozen {
        return Err("cannot modify frozen table".to_string());
    }

    if args.len() == 2 {
        tbl.array.push(args[1].clone());
    } else if args.len() >= 3 {
        let pos = to_isize(&args[1], "pos")?;
        let len = tbl.array.len() as isize;
        if pos < 1 || pos > len + 1 {
            return Err("table.insert position out of bounds".to_string());
        }
        tbl.array.insert((pos - 1) as usize, args[2].clone());
    } else {
        return Err("table.insert expects at least 2 arguments".to_string());
    }
    Ok(vec![])
}

fn table_remove(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "table.remove expects table".to_string())?,
    )?;
    let mut tbl = tbl_rc.borrow_mut();
    if tbl.frozen {
        return Err("cannot modify frozen table".to_string());
    }

    let len = tbl.array.len() as isize;
    if len == 0 {
        return Ok(vec![Value::Nil]);
    }

    let pos = if let Some(pv) = args.get(1) {
        to_isize(pv, "pos")?
    } else {
        len
    };

    if pos < 1 || pos > len {
        return Err("table.remove position out of bounds".to_string());
    }

    let removed = tbl.array.remove((pos - 1) as usize);
    Ok(vec![removed])
}

fn table_concat(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "table.concat expects table".to_string())?,
    )?;
    let sep = if let Some(sv) = args.get(1) {
        match sv {
            Value::String(s) => s.clone(),
            _ => sv.to_string(),
        }
    } else {
        String::new()
    };

    let tbl = tbl_rc.borrow();
    let len = tbl.array.len();
    let i = if let Some(iv) = args.get(2) {
        to_isize(iv, "start")?.max(1) as usize
    } else {
        1
    };
    let j = if let Some(jv) = args.get(3) {
        to_isize(jv, "end")?.min(len as isize).max(0) as usize
    } else {
        len
    };

    if i > j || i > len {
        return Ok(vec![Value::String(String::new())]);
    }

    if j.saturating_sub(i - 1) > 1_000_000 {
        return Err("table.concat range exceeds maximum limit (1,000,000)".to_string());
    }

    let mut parts = Vec::new();
    for idx in (i - 1)..j {
        if let Some(v) = tbl.array.get(idx) {
            parts.push(v.to_string());
        }
    }
    Ok(vec![Value::String(parts.join(&sep))])
}

fn table_pack(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut tbl = VmTable::new();
    tbl.array = args.to_vec();
    tbl.set_str("n", Value::Int(BigInt::from(args.len())));
    let rc = Rc::new(RefCell::new(tbl));
    vm.gc.register_table(&rc);
    Ok(vec![Value::Table(rc)])
}

fn table_unpack(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "table.unpack expects table".to_string())?,
    )?;
    let tbl = tbl_rc.borrow();
    let len = tbl.array.len();
    let i = if let Some(iv) = args.get(1) {
        to_isize(iv, "start")?.max(1) as usize
    } else {
        1
    };
    let j = if let Some(jv) = args.get(2) {
        to_isize(jv, "end")?.min(len as isize).max(0) as usize
    } else {
        len
    };

    if i > j || i > len {
        return Ok(vec![]);
    }

    if j.saturating_sub(i - 1) > 8000 {
        return Err("too many results to unpack (maximum 8000)".to_string());
    }

    let mut results = Vec::new();
    for idx in (i - 1)..j {
        if let Some(v) = tbl.array.get(idx) {
            results.push(v.clone());
        } else {
            results.push(Value::Nil);
        }
    }
    Ok(results)
}

fn table_freeze(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "table.freeze expects table".to_string())?,
    )?;
    tbl_rc.borrow_mut().frozen = true;
    Ok(vec![Value::Table(tbl_rc.clone())])
}

fn table_isfrozen(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "table.isfrozen expects table".to_string())?,
    )?;
    let frozen = tbl_rc.borrow().frozen;
    Ok(vec![Value::Bool(frozen)])
}

fn table_clear(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "table.clear expects table".to_string())?,
    )?;
    let mut tbl = tbl_rc.borrow_mut();
    if tbl.frozen {
        return Err("cannot clear frozen table".to_string());
    }
    tbl.array.clear();
    tbl.fields.clear();
    Ok(vec![])
}

fn table_clone(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "table.clone expects table".to_string())?,
    )?;
    let tbl = tbl_rc.borrow();
    let mut new_tbl = VmTable::new();
    new_tbl.array = tbl.array.clone();
    new_tbl.fields = tbl.fields.clone();
    new_tbl.metatable = tbl.metatable.clone();
    let rc = Rc::new(RefCell::new(new_tbl));
    vm.gc.register_table(&rc);
    Ok(vec![Value::Table(rc)])
}

fn table_sort(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "table.sort expects table".to_string())?,
    )?
    .clone();
    if tbl_rc.borrow().frozen {
        return Err("cannot sort frozen table".to_string());
    }

    let comp = args.get(1).cloned();
    if let Some(comp_fn) = comp
        && !matches!(comp_fn, Value::Nil)
    {
        let mut items = std::mem::take(&mut tbl_rc.borrow_mut().array);
        let mut sort_err = None;
        for i in 1..items.len() {
            let mut j = i;
            while j > 0 {
                let a = &items[j - 1];
                let b = &items[j];
                match vm.call_function(comp_fn.clone(), &[b.clone(), a.clone()]) {
                    Ok(res) => {
                        let b_less_than_a = res.first().map(|v| v.is_truthy()).unwrap_or(false);
                        if b_less_than_a {
                            items.swap(j - 1, j);
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                    Err(e) => {
                        sort_err = Some(e);
                        break;
                    }
                }
            }
            if sort_err.is_some() {
                break;
            }
        }
        tbl_rc.borrow_mut().array = items;
        if let Some(err) = sort_err {
            return Err(err);
        }
    } else {
        let mut tbl = tbl_rc.borrow_mut();
        tbl.array.sort_by(|a, b| match (a, b) {
            (Value::Int(ia), Value::Int(ib)) => ia.cmp(ib),
            (Value::String(sa), Value::String(sb)) => sa.cmp(sb),
            (Value::Float(fa), Value::Float(fb)) => {
                fa.partial_cmp(fb).unwrap_or(std::cmp::Ordering::Equal)
            }
            _ => a.to_string().cmp(&b.to_string()),
        });
    }
    Ok(vec![])
}

fn table_move(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let a1_rc = get_table(
        args.first()
            .ok_or_else(|| "table.move expects at least 4 arguments".to_string())?,
    )?
    .clone();
    let f = to_isize(
        args.get(1)
            .ok_or_else(|| "table.move expects f".to_string())?,
        "f",
    )?;
    let e = to_isize(
        args.get(2)
            .ok_or_else(|| "table.move expects e".to_string())?,
        "e",
    )?;
    let t = to_isize(
        args.get(3)
            .ok_or_else(|| "table.move expects t".to_string())?,
        "t",
    )?;
    let a2_rc = if let Some(a2_val) = args.get(4) {
        if matches!(a2_val, Value::Nil) {
            a1_rc.clone()
        } else {
            get_table(a2_val)?.clone()
        }
    } else {
        a1_rc.clone()
    };
    if e >= f {
        let count = (e - f + 1) as usize;
        let mut slice = Vec::with_capacity(count);
        {
            let a1 = a1_rc.borrow();
            for idx in f..=e {
                let v = if idx >= 1 && (idx as usize) <= a1.array.len() {
                    a1.array[(idx - 1) as usize].clone()
                } else {
                    Value::Nil
                };
                slice.push(v);
            }
        }
        {
            let mut a2 = a2_rc.borrow_mut();
            if a2.frozen {
                return Err("cannot modify frozen table".to_string());
            }
            let needed = (t + (count as isize) - 1).max(0) as usize;
            if needed > a2.array.len() {
                a2.array.resize(needed, Value::Nil);
            }
            for (offset, val) in slice.into_iter().enumerate() {
                let dest = (t + (offset as isize) - 1) as usize;
                if dest < a2.array.len() {
                    a2.array[dest] = val;
                }
            }
        }
    }
    Ok(vec![Value::Table(a2_rc)])
}

pub fn table_setmetatable(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "setmetatable expects table as 1st argument".to_string())?,
    )?;
    let mt = match args.get(1) {
        Some(Value::Table(m)) => Some(m.clone()),
        Some(Value::Nil) | None => None,
        _ => return Err("setmetatable expects table or nil as 2nd argument".to_string()),
    };
    tbl_rc.borrow_mut().metatable = mt;
    Ok(vec![args[0].clone()])
}

pub fn table_getmetatable(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let tbl_rc = get_table(
        args.first()
            .ok_or_else(|| "getmetatable expects table".to_string())?,
    )?;
    let mt = tbl_rc.borrow().metatable.clone();
    match mt {
        Some(m) => Ok(vec![Value::Table(m)]),
        None => Ok(vec![Value::Nil]),
    }
}

pub fn create_table_lib() -> Value {
    let mut table = VmTable::new();
    table.set_str("insert", Value::Native("table.insert", table_insert));
    table.set_str("remove", Value::Native("table.remove", table_remove));
    table.set_str("concat", Value::Native("table.concat", table_concat));
    table.set_str("pack", Value::Native("table.pack", table_pack));
    table.set_str("unpack", Value::Native("table.unpack", table_unpack));
    table.set_str("freeze", Value::Native("table.freeze", table_freeze));
    table.set_str("isfrozen", Value::Native("table.isfrozen", table_isfrozen));
    table.set_str("clear", Value::Native("table.clear", table_clear));
    table.set_str("clone", Value::Native("table.clone", table_clone));
    table.set_str("sort", Value::Native("table.sort", table_sort));
    table.set_str("move", Value::Native("table.move", table_move));
    table.set_str(
        "setmetatable",
        Value::Native("table.setmetatable", table_setmetatable),
    );
    table.set_str(
        "getmetatable",
        Value::Native("table.getmetatable", table_getmetatable),
    );
    table.frozen = true;
    Value::Table(Rc::new(RefCell::new(table)))
}
