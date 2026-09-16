// Coroutine standard library for Neyuki VM.

use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

pub fn create_coroutine_lib() -> Value {
    let t = Rc::new(RefCell::new(VmTable::new()));
    let mut b = t.borrow_mut();

    b.set_str("create", Value::Native("coroutine.create", coroutine_create));
    b.set_str("resume", Value::Native("coroutine.resume", coroutine_resume));
    b.set_str("yield", Value::Native("coroutine.yield", coroutine_yield));
    b.set_str("status", Value::Native("coroutine.status", coroutine_status));
    b.set_str("wrap", Value::Native("coroutine.wrap", coroutine_wrap));
    b.set_str(
        "isyieldable",
        Value::Native("coroutine.isyieldable", coroutine_isyieldable),
    );

    Value::Table(t.clone())
}

fn coroutine_create(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let func = match args.first() {
        Some(f @ (Value::Closure(_) | Value::Native(_, _))) => f.clone(),
        _ => return Err("bad argument #1 to 'coroutine.create' (function expected)".to_string()),
    };

    let co = Rc::new(RefCell::new(VmTable::new()));
    let mut b = co.borrow_mut();
    b.set_str("__type", Value::String("thread".to_string()));
    b.set_str("status", Value::String("suspended".to_string()));
    b.set_str("func", func);

    Ok(vec![Value::Table(co.clone())])
}

fn coroutine_resume(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let co_val = match args.first() {
        Some(Value::Table(t)) => t.clone(),
        _ => return Err("bad argument #1 to 'coroutine.resume' (thread expected)".to_string()),
    };

    let (status, func) = {
        let b = co_val.borrow();
        (b.get_str("status"), b.get_str("func"))
    };

    let status_str = match status {
        Value::String(s) => s,
        _ => "dead".to_string(),
    };

    if status_str == "dead" {
        return Ok(vec![
            Value::Bool(false),
            Value::String("cannot resume dead coroutine".to_string()),
        ]);
    }
    if status_str == "running" {
        return Ok(vec![
            Value::Bool(false),
            Value::String("cannot resume running coroutine".to_string()),
        ]);
    }

    co_val
        .borrow_mut()
        .set_str("status", Value::String("running".to_string()));

    let resume_args = &args[1..];
    let call_res = vm.call_function(func, resume_args);

    match call_res {
        Ok(results) => {
            co_val
                .borrow_mut()
                .set_str("status", Value::String("dead".to_string()));
            let mut ret = vec![Value::Bool(true)];
            ret.extend(results);
            Ok(ret)
        }
        Err(err) => {
            co_val
                .borrow_mut()
                .set_str("status", Value::String("dead".to_string()));
            Ok(vec![Value::Bool(false), Value::String(err)])
        }
    }
}

fn coroutine_yield(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    // Return the yielded arguments
    Ok(args.to_vec())
}

fn coroutine_status(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let co_val = match args.first() {
        Some(Value::Table(t)) => t,
        _ => return Err("bad argument #1 to 'coroutine.status' (thread expected)".to_string()),
    };

    let status = co_val.borrow().get_str("status");
    match status {
        Value::String(_) => Ok(vec![status]),
        _ => Ok(vec![Value::String("dead".to_string())]),
    }
}

fn wrapped_coroutine_call(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    if args.is_empty() {
        return Err("coroutine wrapper called without self".to_string());
    }
    let wrap_tbl = match &args[0] {
        Value::Table(t) => t.clone(),
        _ => return Err("coroutine wrapper called on non-table".to_string()),
    };
    let co_table = wrap_tbl.borrow().get_str("_co");
    if matches!(co_table, Value::Nil) {
        return Err("invalid coroutine wrapper".to_string());
    }
    let mut resume_args = vec![co_table];
    resume_args.extend_from_slice(&args[1..]);
    let res = coroutine_resume(vm, &resume_args)?;
    if let Some(Value::Bool(true)) = res.first() {
        Ok(res[1..].to_vec())
    } else {
        let err_msg = res.get(1).map(|v| v.to_string()).unwrap_or_else(|| "error in coroutine".to_string());
        Err(err_msg)
    }
}

fn coroutine_wrap(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let create_res = coroutine_create(vm, args)?;
    let co_table = create_res.into_iter().next().unwrap();

    let wrapper_table = Rc::new(RefCell::new(VmTable::new()));
    wrapper_table.borrow_mut().set_str("_co", co_table);

    let mt = Rc::new(RefCell::new(VmTable::new()));
    mt.borrow_mut().set_str("__call", Value::Native("wrapped_coroutine_call", wrapped_coroutine_call));
    wrapper_table.borrow_mut().metatable = Some(mt);

    Ok(vec![Value::Table(wrapper_table)])
}

fn coroutine_isyieldable(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Bool(true)])
}
