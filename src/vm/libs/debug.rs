// Debug standard library for Neyuki VM.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::machine::VM;
use crate::vm::value::{NativeDef, Value, VmTable};

pub fn create_debug_lib() -> Value {
    let t = Rc::new(RefCell::new(VmTable::new()));
    let mut b = t.borrow_mut();

    b.set_str(
        "traceback",
        crate::native!("debug.traceback", debug_traceback),
    );
    b.set_str("getinfo", crate::native!("debug.getinfo", debug_getinfo));
    b.set_str("getlocal", crate::native!("debug.getlocal", debug_getlocal));
    b.set_str("setlocal", crate::native!("debug.setlocal", debug_setlocal));
    b.set_str(
        "getupvalue",
        crate::native!("debug.getupvalue", debug_getupvalue),
    );
    b.set_str(
        "setupvalue",
        crate::native!("debug.setupvalue", debug_setupvalue),
    );

    Value::Table(t.clone())
}

fn debug_traceback(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut out = String::new();
    if let Some(msg) = args.first()
        && !matches!(msg, Value::Nil)
    {
        out.push_str(&msg.to_string());
        out.push('\n');
    }

    out.push_str("stack traceback:\n");
    let level_offset = match args.get(1) {
        Some(Value::Int(i)) => i.to_usize().unwrap_or(1),
        Some(Value::Float(f)) => *f as usize,
        _ => 1,
    };

    let total_frames = vm.frames.len();
    for (i, frame) in vm.frames.iter().rev().enumerate() {
        if i < level_offset.saturating_sub(1) {
            continue;
        }

        let proto = &frame.closure.proto;
        let name = proto.name.as_deref().unwrap_or("<anonymous>");
        let line = if frame.ip < proto.lines.len() {
            proto.lines[frame.ip]
        } else {
            proto.lines.last().copied().unwrap_or(1)
        };

        out.push_str(&format!(
            "  [frame {}] function '{}' at line {}\n",
            total_frames.saturating_sub(i),
            name,
            line
        ));
    }

    Ok(vec![Value::string(out)])
}

fn debug_getinfo(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let arg0 = match args.first() {
        Some(a) => a,
        None => return Ok(vec![Value::Nil]),
    };

    match arg0 {
        Value::Closure(c) => {
            let tbl = Rc::new(RefCell::new(VmTable::new()));
            let mut b = tbl.borrow_mut();
            let name = c
                .proto
                .name
                .clone()
                .unwrap_or_else(|| "<anonymous>".to_string());
            b.set_str("name", Value::String((name.clone()).into()));
            b.set_str("what", Value::str("Lua"));
            b.set_str("source", Value::string(name));
            b.set_str(
                "numparams",
                Value::from_bigint(BigInt::from(c.proto.num_params)),
            );
            b.set_str("isvararg", Value::Bool(c.proto.is_vararg));
            b.set_str("currentline", Value::from_bigint(BigInt::from(-1)));
            b.set_str("func", Value::Closure(c.clone()));
            drop(b);
            Ok(vec![Value::Table(tbl)])
        }
        Value::Native(NativeDef { name, .. }) => {
            let tbl = Rc::new(RefCell::new(VmTable::new()));
            let mut b = tbl.borrow_mut();
            b.set_str("name", Value::string(name.to_string()));
            b.set_str("what", Value::str("C"));
            b.set_str("source", Value::str("=[C]"));
            b.set_str("numparams", Value::from_bigint(BigInt::from(0)));
            b.set_str("isvararg", Value::Bool(true));
            b.set_str("currentline", Value::from_bigint(BigInt::from(-1)));
            b.set_str("func", arg0.clone());
            drop(b);
            Ok(vec![Value::Table(tbl)])
        }
        Value::Int(i) => {
            let level = i.to_usize().unwrap_or(0);
            get_frame_info(vm, level)
        }
        Value::Float(f) => {
            let level = *f as usize;
            get_frame_info(vm, level)
        }
        _ => Ok(vec![Value::Nil]),
    }
}

fn get_frame_info(vm: &VM, level: usize) -> Result<Vec<Value>, String> {
    if level == 0 {
        let tbl = Rc::new(RefCell::new(VmTable::new()));
        let mut b = tbl.borrow_mut();
        b.set_str("name", Value::str("debug.getinfo"));
        b.set_str("what", Value::str("C"));
        b.set_str("source", Value::str("=[C]"));
        b.set_str("numparams", Value::from_bigint(BigInt::from(0)));
        b.set_str("isvararg", Value::Bool(true));
        b.set_str("currentline", Value::from_bigint(BigInt::from(-1)));
        drop(b);
        return Ok(vec![Value::Table(tbl)]);
    }

    if level > vm.frames.len() {
        return Ok(vec![Value::Nil]);
    }

    let frame = &vm.frames[vm.frames.len() - level];
    let proto = &frame.closure.proto;

    let tbl = Rc::new(RefCell::new(VmTable::new()));
    let mut b = tbl.borrow_mut();

    let name = proto
        .name
        .clone()
        .unwrap_or_else(|| "<anonymous>".to_string());
    b.set_str("name", Value::String((name.clone()).into()));
    b.set_str("what", Value::str("Lua"));
    b.set_str("source", Value::string(name));
    b.set_str(
        "numparams",
        Value::from_bigint(BigInt::from(proto.num_params)),
    );
    b.set_str("isvararg", Value::Bool(proto.is_vararg));

    let line = if frame.ip < proto.lines.len() {
        proto.lines[frame.ip]
    } else {
        proto.lines.last().copied().unwrap_or(1)
    };
    b.set_str("currentline", Value::from_bigint(BigInt::from(line)));
    b.set_str("func", Value::Closure(frame.closure.clone()));
    drop(b);

    Ok(vec![Value::Table(tbl)])
}

fn debug_getlocal(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let level = match args.first() {
        Some(Value::Int(i)) => i.to_usize().unwrap_or(1),
        Some(Value::Float(f)) => *f as usize,
        _ => 1,
    };

    let idx = match args.get(1) {
        Some(Value::Int(i)) => i.to_usize().unwrap_or(1),
        Some(Value::Float(f)) => *f as usize,
        _ => 1,
    };

    if level == 0 || level > vm.frames.len() {
        return Ok(vec![Value::Nil]);
    }

    let frame = &vm.frames[vm.frames.len() - level];
    let proto = &frame.closure.proto;

    let current_ip = frame.ip as u32;
    let mut matching_locals: Vec<&crate::bytecode::proto::LocalVarInfo> = Vec::new();

    for info in &proto.local_names {
        if current_ip >= info.from_pc && current_ip <= info.to_pc {
            matching_locals.push(info);
        }
    }

    if idx == 0 || idx > matching_locals.len() {
        return Ok(vec![Value::Nil]);
    }

    let local_info = matching_locals[idx - 1];
    let reg_idx = local_info.reg;
    let stack_slot = frame.base + reg_idx as usize;
    let val = vm.stack.get(stack_slot).cloned().unwrap_or(Value::Nil);

    Ok(vec![Value::String((local_info.name.clone()).into()), val])
}

fn debug_setlocal(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let level = match args.first() {
        Some(Value::Int(i)) => i.to_usize().unwrap_or(1),
        Some(Value::Float(f)) => *f as usize,
        _ => 1,
    };

    let idx = match args.get(1) {
        Some(Value::Int(i)) => i.to_usize().unwrap_or(1),
        Some(Value::Float(f)) => *f as usize,
        _ => 1,
    };

    let new_val = args.get(2).cloned().unwrap_or(Value::Nil);

    if level == 0 || level > vm.frames.len() {
        return Ok(vec![Value::Nil]);
    }

    let frame = &vm.frames[vm.frames.len() - level];
    let proto = &frame.closure.proto;

    let current_ip = frame.ip as u32;
    let mut matching_locals: Vec<&crate::bytecode::proto::LocalVarInfo> = Vec::new();

    for info in &proto.local_names {
        if current_ip >= info.from_pc && current_ip <= info.to_pc {
            matching_locals.push(info);
        }
    }

    if idx == 0 || idx > matching_locals.len() {
        return Ok(vec![Value::Nil]);
    }

    let local_info = matching_locals[idx - 1];
    let reg_idx = local_info.reg;
    let stack_slot = frame.base + reg_idx as usize;
    if stack_slot >= vm.stack.len() {
        vm.stack.resize(stack_slot + 1, Value::Nil);
    }
    vm.stack[stack_slot] = new_val;

    Ok(vec![Value::String((local_info.name.clone()).into())])
}

fn debug_getupvalue(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let Some(Value::Closure(c)) = args.first() else {
        return Err("bad argument #1 to 'debug.getupvalue' (function expected)".to_string());
    };
    let idx = match args.get(1) {
        Some(Value::Int(i)) => i.to_usize().unwrap_or(1),
        Some(Value::Float(f)) => *f as usize,
        _ => 1,
    };
    if idx == 0 || idx > c.upvalues.len() {
        return Ok(vec![Value::Nil]);
    }
    let val = _vm.upvalue_get(&c.upvalues[idx - 1]);
    Ok(vec![Value::String((format!("upval_{}", idx)).into()), val])
}

fn debug_setupvalue(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let Some(Value::Closure(c)) = args.first() else {
        return Err("bad argument #1 to 'debug.setupvalue' (function expected)".to_string());
    };
    let idx = match args.get(1) {
        Some(Value::Int(i)) => i.to_usize().unwrap_or(1),
        Some(Value::Float(f)) => *f as usize,
        _ => 1,
    };
    let new_val = args.get(2).cloned().unwrap_or(Value::Nil);
    if idx == 0 || idx > c.upvalues.len() {
        return Ok(vec![Value::Nil]);
    }
    let uv = c.upvalues[idx - 1].clone();
    _vm.upvalue_set(&uv, new_val);
    Ok(vec![Value::String((format!("upval_{}", idx)).into())])
}
