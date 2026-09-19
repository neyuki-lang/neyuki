// Coroutine standard library for Neyuki VM.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::frame::CallFrame;
use crate::vm::machine::{CoroutineState, VM};
use crate::vm::value::{Value, VmTable};

pub fn create_coroutine_lib() -> Value {
    let t = Rc::new(RefCell::new(VmTable::new()));
    let mut b = t.borrow_mut();

    b.set_str(
        "create",
        Value::Native("coroutine.create", coroutine_create),
    );
    b.set_str(
        "resume",
        Value::Native("coroutine.resume", coroutine_resume),
    );
    b.set_str("yield", Value::Native("coroutine.yield", coroutine_yield));
    b.set_str(
        "status",
        Value::Native("coroutine.status", coroutine_status),
    );
    b.set_str(
        "running",
        Value::Native("coroutine.running", coroutine_running),
    );
    b.set_str("wrap", Value::Native("coroutine.wrap", coroutine_wrap));
    b.set_str(
        "isyieldable",
        Value::Native("coroutine.isyieldable", coroutine_isyieldable),
    );

    Value::Table(t.clone())
}

pub(crate) fn coroutine_create(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let func = match args.first() {
        Some(f @ (Value::Closure(_) | Value::Native(_, _))) => f.clone(),
        _ => return Err("bad argument #1 to 'coroutine.create' (function expected)".to_string()),
    };

    let id = vm.next_co_id;
    vm.next_co_id += 1;

    let state = CoroutineState {
        stack: Vec::new(),
        frames: Vec::new(),
        status: "suspended".to_string(),
        func: func.clone(),
        yield_callee: 0,
        yield_retc: 0,
        yield_values: Vec::new(),
        open_upvalues: std::collections::HashMap::new(),
    };
    vm.coroutines.insert(id, Rc::new(RefCell::new(state)));

    let co = Rc::new(RefCell::new(VmTable::new()));
    let mut b = co.borrow_mut();
    b.set_str("__type", Value::String("thread".to_string()));
    b.set_str("_id", Value::Int(BigInt::from(id)));
    b.set_str("status", Value::String("suspended".to_string()));
    b.set_str("func", func);
    drop(b);

    Ok(vec![Value::Table(co)])
}

pub(crate) fn coroutine_resume(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let co_val = match args.first() {
        Some(Value::Table(t)) => t.clone(),
        _ => return Err("bad argument #1 to 'coroutine.resume' (thread expected)".to_string()),
    };

    let co_id = match co_val.borrow().get_str("_id") {
        Value::Int(i) => i.to_usize().unwrap_or(0),
        _ => 0,
    };

    if co_id == 0 || !vm.coroutines.contains_key(&co_id) {
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
        let resume_args = &args[1..];
        co_val
            .borrow_mut()
            .set_str("status", Value::String("dead".to_string()));
        return match vm.call_function(func, resume_args) {
            Ok(results) => {
                let mut ret = vec![Value::Bool(true)];
                ret.extend(results);
                Ok(ret)
            }
            Err(err) => Ok(vec![Value::Bool(false), Value::String(err)]),
        };
    }

    let co_rc = vm.coroutines.get(&co_id).unwrap().clone();

    let current_status = co_rc.borrow().status.clone();
    if current_status == "dead" {
        return Ok(vec![
            Value::Bool(false),
            Value::String("cannot resume dead coroutine".to_string()),
        ]);
    }
    if current_status == "running" {
        return Ok(vec![
            Value::Bool(false),
            Value::String("cannot resume running coroutine".to_string()),
        ]);
    }

    let caller_stack = std::mem::take(&mut vm.stack);
    let caller_frames = std::mem::take(&mut vm.frames);
    let caller_open_upvalues = std::mem::take(&mut vm.open_upvalues);
    let caller_co = vm.current_co;

    let is_initial = co_rc.borrow().frames.is_empty();
    {
        let mut co_state = co_rc.borrow_mut();
        vm.stack = std::mem::take(&mut co_state.stack);
        vm.frames = std::mem::take(&mut co_state.frames);
        vm.open_upvalues = std::mem::take(&mut co_state.open_upvalues);
        vm.current_co = Some(co_id);
        co_state.status = "running".to_string();
        co_val
            .borrow_mut()
            .set_str("status", Value::String("running".to_string()));
    }

    let resume_args = &args[1..];

    if is_initial {
        let func = co_rc.borrow().func.clone();
        match func {
            Value::Closure(c) => {
                let base = 0;
                let needed = base + c.proto.max_registers as usize + resume_args.len() + 1;
                vm.stack.resize(needed, Value::Nil);
                for (i, arg) in resume_args.iter().enumerate() {
                    vm.stack[base + i] = arg.clone();
                }
                let num_params = c.proto.num_params as usize;
                let varargs = if resume_args.len() > num_params {
                    resume_args[num_params..].to_vec()
                } else {
                    Vec::new()
                };
                vm.frames.push(CallFrame::with_varargs(c, base, varargs));
            }
            Value::Native(_, f) => {
                let res = f(vm, resume_args);
                co_rc.borrow_mut().status = "dead".to_string();
                co_val
                    .borrow_mut()
                    .set_str("status", Value::String("dead".to_string()));
                vm.coroutines.remove(&co_id);
                vm.stack = caller_stack;
                vm.frames = caller_frames;
                vm.open_upvalues = caller_open_upvalues;
                vm.current_co = caller_co;
                return match res {
                    Ok(vals) => {
                        let mut out = vec![Value::Bool(true)];
                        out.extend(vals);
                        Ok(out)
                    }
                    Err(e) => Ok(vec![Value::Bool(false), Value::String(e)]),
                };
            }
            _ => {
                vm.stack = caller_stack;
                vm.frames = caller_frames;
                vm.open_upvalues = caller_open_upvalues;
                vm.current_co = caller_co;
                return Err("coroutine function is not callable".to_string());
            }
        }
    } else {
        let (callee, retc) = {
            let cs = co_rc.borrow();
            (cs.yield_callee, cs.yield_retc)
        };
        let count = if retc == 0 { 1 } else { retc as usize };
        let base = vm.frames.last().map(|f| f.base).unwrap_or(0);
        for i in 0..count {
            let val = resume_args.get(i).cloned().unwrap_or(Value::Nil);
            let slot = base + callee as usize + i;
            if slot >= vm.stack.len() {
                vm.stack.resize(slot + 1, Value::Nil);
            }
            vm.stack[slot] = val;
        }
    }

    let run_res = vm.run_to_depth(0);

    match run_res {
        Err(ref _err) if vm.is_yielding => {
            vm.is_yielding = false;
            let mut cs = co_rc.borrow_mut();
            cs.stack = std::mem::take(&mut vm.stack);
            cs.frames = std::mem::take(&mut vm.frames);
            cs.open_upvalues = std::mem::take(&mut vm.open_upvalues);
            cs.status = "suspended".to_string();
            co_val
                .borrow_mut()
                .set_str("status", Value::String("suspended".to_string()));
            let yielded = std::mem::take(&mut cs.yield_values);
            drop(cs);

            vm.stack = caller_stack;
            vm.frames = caller_frames;
            vm.open_upvalues = caller_open_upvalues;
            vm.current_co = caller_co;

            let mut out = vec![Value::Bool(true)];
            out.extend(yielded);
            Ok(out)
        }
        Ok(ret_val) => {
            let mut cs = co_rc.borrow_mut();
            cs.status = "dead".to_string();
            co_val
                .borrow_mut()
                .set_str("status", Value::String("dead".to_string()));
            drop(cs);
            vm.coroutines.remove(&co_id);

            vm.stack = caller_stack;
            vm.frames = caller_frames;
            vm.open_upvalues = caller_open_upvalues;
            vm.current_co = caller_co;

            Ok(vec![Value::Bool(true), ret_val])
        }
        Err(err) => {
            let mut cs = co_rc.borrow_mut();
            cs.status = "dead".to_string();
            co_val
                .borrow_mut()
                .set_str("status", Value::String("dead".to_string()));
            drop(cs);
            vm.coroutines.remove(&co_id);

            vm.stack = caller_stack;
            vm.frames = caller_frames;
            vm.open_upvalues = caller_open_upvalues;
            vm.current_co = caller_co;

            Ok(vec![Value::Bool(false), Value::String(err)])
        }
    }
}

fn coroutine_yield(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let Some(co_id) = vm.current_co else {
        return Err("attempt to yield from outside a coroutine".to_string());
    };
    if let Some(co_rc) = vm.coroutines.get(&co_id) {
        co_rc.borrow_mut().yield_values = args.to_vec();
    }
    vm.is_yielding = true;
    Err("__NEYUKI_COROUTINE_YIELD__".to_string())
}

fn coroutine_status(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let co_val = match args.first() {
        Some(Value::Table(t)) => t,
        _ => return Err("bad argument #1 to 'coroutine.status' (thread expected)".to_string()),
    };

    let co_id = match co_val.borrow().get_str("_id") {
        Value::Int(i) => i.to_usize().unwrap_or(0),
        _ => 0,
    };

    if let Some(co_rc) = vm.coroutines.get(&co_id) {
        let st = co_rc.borrow().status.clone();
        Ok(vec![Value::String(st)])
    } else {
        let status = co_val.borrow().get_str("status");
        match status {
            Value::String(_) => Ok(vec![status]),
            _ => Ok(vec![Value::String("dead".to_string())]),
        }
    }
}

fn coroutine_running(vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    if let Some(co_id) = vm.current_co {
        let tbl = Rc::new(RefCell::new(VmTable::new()));
        let mut b = tbl.borrow_mut();
        b.set_str("__type", Value::String("thread".to_string()));
        b.set_str("_id", Value::Int(BigInt::from(co_id)));
        b.set_str("status", Value::String("running".to_string()));
        drop(b);
        Ok(vec![Value::Table(tbl), Value::Bool(false)])
    } else {
        Ok(vec![Value::Nil, Value::Bool(true)])
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
        let err_msg = res
            .get(1)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "error in coroutine".to_string());
        Err(err_msg)
    }
}

fn coroutine_wrap(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let create_res = coroutine_create(vm, args)?;
    let co_table = create_res.into_iter().next().unwrap();

    let wrapper_table = Rc::new(RefCell::new(VmTable::new()));
    wrapper_table.borrow_mut().set_str("_co", co_table);

    let mt = Rc::new(RefCell::new(VmTable::new()));
    mt.borrow_mut().set_str(
        "__call",
        Value::Native("wrapped_coroutine_call", wrapped_coroutine_call),
    );
    wrapper_table.borrow_mut().metatable = Some(mt);

    Ok(vec![Value::Table(wrapper_table)])
}

fn coroutine_isyieldable(vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Bool(vm.current_co.is_some())])
}
