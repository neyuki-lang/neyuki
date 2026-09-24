// Standard table manipulation library for Neyuki VM.

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
            Value::String(s) => s.to_string(),
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
        return Ok(vec![Value::String((String::new()).into())]);
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
    Ok(vec![Value::String((parts.join(&sep)).into())])
}

fn table_pack(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut tbl = VmTable::new();
    tbl.array = args.to_vec();
    tbl.set_str("n", Value::from_usize(args.len()));
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
    // Cloned handles share the coroutine: there is still exactly one state.
    new_tbl.co_state = tbl.co_state.clone();
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
        let res = merge_sort_by(vm, &comp_fn, &mut items);
        tbl_rc.borrow_mut().array = items;
        res?;
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

/// Bottom-up merge sort driven by a Neyuki comparator: `comp(a, b)` is true
/// when `a` must come before `b` (the same contract `table.sort` documents).
/// Stable, worst-case O(n log n) comparator calls, iterative so there is no
/// recursion depth to blow, and a comparator error aborts immediately with
/// the table restored by the caller. This replaced an insertion sort whose
/// O(n^2) callbacks made comparator sorts ~200x slower than they should be.
fn merge_sort_by(vm: &mut VM, comp: &Value, items: &mut Vec<Value>) -> Result<(), String> {
    let n = items.len();
    if n < 2 {
        return Ok(());
    }
    let mut scratch: Vec<Value> = Vec::with_capacity(n);
    let mut width = 1;
    while width < n {
        scratch.clear();
        let mut start = 0;
        while start < n {
            let mid = (start + width).min(n);
            let end = (start + 2 * width).min(n);
            let (mut left, mut right) = (start, mid);
            while left < mid && right < end {
                // `comp(right, left)` true means right sorts first.
                let right_first = vm
                    .call_function(comp.clone(), &[items[right].clone(), items[left].clone()])?
                    .first()
                    .map(|v| v.is_truthy())
                    .unwrap_or(false);
                if right_first {
                    scratch.push(items[right].clone());
                    right += 1;
                } else {
                    scratch.push(items[left].clone());
                    left += 1;
                }
            }
            while left < mid {
                scratch.push(items[left].clone());
                left += 1;
            }
            while right < end {
                scratch.push(items[right].clone());
                right += 1;
            }
            start = end;
        }
        std::mem::swap(items, &mut scratch);
        width *= 2;
    }
    Ok(())
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
    table.set_str("insert", crate::native!("table.insert", table_insert));
    table.set_str("remove", crate::native!("table.remove", table_remove));
    table.set_str("concat", crate::native!("table.concat", table_concat));
    table.set_str("pack", crate::native!("table.pack", table_pack));
    table.set_str("unpack", crate::native!("table.unpack", table_unpack));
    table.set_str("freeze", crate::native!("table.freeze", table_freeze));
    table.set_str("isfrozen", crate::native!("table.isfrozen", table_isfrozen));
    table.set_str("clear", crate::native!("table.clear", table_clear));
    table.set_str("clone", crate::native!("table.clone", table_clone));
    table.set_str("sort", crate::native!("table.sort", table_sort));
    table.set_str("move", crate::native!("table.move", table_move));
    table.set_str(
        "setmetatable",
        crate::native!("table.setmetatable", table_setmetatable),
    );
    table.set_str(
        "getmetatable",
        crate::native!("table.getmetatable", table_getmetatable),
    );
    table.frozen = true;
    Value::Table(Rc::new(RefCell::new(table)))
}
