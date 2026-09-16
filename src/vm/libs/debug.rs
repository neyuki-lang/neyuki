// Debug standard library for Neyuki VM.

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::rc::Rc;

use crate::vm::machine::VM;
use crate::vm::value::{Value, VmTable};

pub fn create_debug_lib() -> Value {
    let t = Rc::new(RefCell::new(VmTable::new()));
    let mut b = t.borrow_mut();

    b.set_str("traceback", Value::Native("debug.traceback", debug_traceback));
    b.set_str("getinfo", Value::Native("debug.getinfo", debug_getinfo));
    b.set_str("getlocal", Value::Native("debug.getlocal", debug_getlocal));

    Value::Table(t.clone())
}

fn debug_traceback(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let mut out = String::new();
    if let Some(msg) = args.first()
        && !matches!(msg, Value::Nil) {
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

        out.push_str(&format!("  [frame {}] function '{}' at line {}\n", total_frames.saturating_sub(i), name, line));
    }

    Ok(vec![Value::String(out)])
}

fn debug_getinfo(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let level = match args.first() {
        Some(Value::Int(i)) => i.to_usize().unwrap_or(1),
        Some(Value::Float(f)) => *f as usize,
        _ => 1,
    };

    if level == 0 || level > vm.frames.len() {
        return Ok(vec![Value::Nil]);
    }

    let frame = &vm.frames[vm.frames.len() - level];
    let proto = &frame.closure.proto;

    let tbl = Rc::new(RefCell::new(VmTable::new()));
    let mut b = tbl.borrow_mut();

    let name = proto.name.clone().unwrap_or_else(|| "<anonymous>".to_string());
    b.set_str("name", Value::String(name));
    b.set_str("numparams", Value::Int(BigInt::from(proto.num_params)));
    b.set_str("isvararg", Value::Bool(proto.is_vararg));

    let line = if frame.ip < proto.lines.len() {
        proto.lines[frame.ip]
    } else {
        proto.lines.last().copied().unwrap_or(1)
    };
    b.set_str("currentline", Value::Int(BigInt::from(line)));
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

    // Search local names at current frame ip
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
    let reg_idx = (idx - 1) as u8;
    let stack_slot = frame.base + reg_idx as usize;
    let val = vm.stack.get(stack_slot).cloned().unwrap_or(Value::Nil);

    Ok(vec![Value::String(local_info.name.clone()), val])
}
